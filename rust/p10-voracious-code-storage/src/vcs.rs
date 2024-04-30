use std::cmp;
use std::io;
use std::mem;
use std::pin::{pin, Pin};
use std::result;
use std::sync::Arc;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use tracing::{debug, instrument};

use parking_lot::RwLock;

use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
#[allow(clippy::module_name_repetitions)]
pub enum VcsError {
    #[error("file not found")]
    FileNotFound,

    #[error("dir not found")]
    DirNotFound,

    #[error("release not found")]
    ReleaseNotFound,

    #[error("invalid path")]
    InvalidPath,

    #[error("invalid data")]
    InvalidData,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ListEntry {
    Dir(String, usize),
    File(String, usize),
}

impl ListEntry {
    #[must_use]
    pub fn name(&self) -> &String {
        match self {
            ListEntry::Dir(name, ..) | ListEntry::File(name, ..) => name,
        }
    }
}

impl PartialOrd for ListEntry {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ListEntry {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        self.name().cmp(other.name())
    }
}

#[derive(Debug)]
struct Entry {
    name: String,
    subdirs: Arc<RwLock<Vec<Entry>>>,
    releases: Arc<RwLock<Vec<Arc<Vec<u8>>>>>,
}

impl Entry {
    fn file(name: &[u8], data: Vec<u8>) -> Self {
        Self {
            name: String::from_utf8_lossy(name).into_owned(),
            releases: Arc::new(RwLock::new(vec![data.into()])),
            subdirs: Arc::new(RwLock::new(vec![])),
        }
    }

    fn name(&self) -> &String {
        &self.name
    }
}

type Result<T> = std::result::Result<T, VcsError>;

#[repr(transparent)]
pub struct Path([u8]);

impl Path {
    fn components(&self) -> impl Iterator<Item = &[u8]> {
        self.0.split(|b| *b == b'/')
    }

    fn is_dir(&self) -> bool {
        self.0.ends_with(b"/")
    }

    /// # Errors
    pub fn new(path: &[u8]) -> Result<&Path> {
        if Self::is_valid(path) {
            Ok(unsafe { &*(path as *const [u8] as *const Path) })
        } else {
            Err(VcsError::InvalidPath)
        }
    }

    fn is_valid(path: &[u8]) -> bool {
        !path.is_empty()
            && path[0] == b'/'
            && path.windows(2).all(|w| w[0] != b'/' || w[1] != b'/')
            && path
                .iter()
                .all(|c| c.is_ascii_alphanumeric() || [b'.', b'_', b'-', b'/'].contains(c))
    }
}

impl<'a> TryFrom<&'a str> for &'a Path {
    type Error = VcsError;

    fn try_from(value: &'a str) -> result::Result<Self, Self::Error> {
        Path::new(value.as_bytes())
    }
}

pub struct Revision(usize);

impl From<usize> for Revision {
    fn from(value: usize) -> Self {
        Self(value)
    }
}

#[derive(Debug)]
pub struct ReadRevision {
    data: Arc<Vec<u8>>,
    index: usize,
}

impl ReadRevision {
    #[must_use]
    pub fn len(&self) -> usize {
        self.data.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

impl AsyncRead for ReadRevision {
    #[instrument(skip(self, src))]
    fn poll_read(
        self: Pin<&mut Self>,
        _: &mut Context,
        src: &mut ReadBuf,
    ) -> Poll<result::Result<(), io::Error>> {
        let this = self.get_mut();
        let len = src.capacity().min(this.data.len() - this.index);
        src.put_slice(&this.data[this.index..this.index + len]);
        this.index += len;
        Poll::Ready(Ok(()))
    }
}

#[derive(Debug)]
pub struct WriteRevision<P> {
    root: Arc<RwLock<Vec<Entry>>>,
    path: P,
    data: Vec<u8>,
}

impl<'a, P> WriteRevision<P>
where
    P: TryInto<&'a Path, Error = VcsError>,
{
    /// # Errors
    #[instrument(skip(self))]
    pub fn commit(mut self) -> Result<usize> {
        if !self
            .data
            .iter()
            .all(|b| b.is_ascii_graphic() || b.is_ascii_whitespace())
        {
            debug!("invalid data");
            return Err(VcsError::InvalidData);
        }

        let path = self.path.try_into()?;

        if path.is_dir() {
            debug!("invalid path, is dir");
            return Err(VcsError::InvalidPath);
        }

        let mut entry = self.root.write_arc();
        let mut components = path
            .components()
            .filter(|component| !component.is_empty())
            .peekable();
        loop {
            match (components.next(), components.peek()) {
                (Some(dir), Some(_)) => {
                    if let Some(Entry { subdirs, .. }) =
                        entry.iter().find(|entry| entry.name().as_bytes() == dir)
                    {
                        entry = subdirs.write_arc();
                    } else {
                        let es = Arc::new(RwLock::new(vec![]));
                        let dir = Entry {
                            name: String::from_utf8_lossy(dir).into_owned(),
                            subdirs: es.clone(),
                            releases: Arc::new(RwLock::new(vec![])),
                        };
                        entry.push(dir);
                        entry = es.write_arc();
                    }
                }

                (Some(file), None) => {
                    if let Some(Entry { releases, .. }) =
                        entry.iter().find(|entry| entry.name().as_bytes() == file)
                    {
                        let mut releases = releases.write();
                        let r = releases.len();
                        match releases.last() {
                            Some(last) if **last == self.data => {
                                debug!("same data: r{}", r - 1);
                                return Ok(r - 1);
                            }

                            _ => {
                                debug!("new data: r{}", r);
                                releases.push(mem::take(&mut self.data).into());
                                return Ok(r);
                            }
                        }
                    }

                    debug!("new data: r0");
                    entry.push(Entry::file(file, mem::take(&mut self.data)));
                    return Ok(0);
                }

                (None, _) => {
                    debug!("file not found <leaf>");
                    return Err(VcsError::FileNotFound);
                }
            }
        }
    }
}

impl<P: Unpin> AsyncWrite for WriteRevision<P> {
    fn poll_write(
        self: Pin<&mut Self>,
        ctx: &mut Context,
        data: &[u8],
    ) -> Poll<result::Result<usize, io::Error>> {
        pin!(&mut self.get_mut().data).poll_write(ctx, data)
    }

    fn poll_flush(self: Pin<&mut Self>, ctx: &mut Context) -> Poll<result::Result<(), io::Error>> {
        pin!(&mut self.get_mut().data).poll_flush(ctx)
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        ctx: &mut Context,
    ) -> Poll<result::Result<(), io::Error>> {
        pin!(&mut self.get_mut().data).poll_flush(ctx)
    }
}

pub struct List<I> {
    len: usize,
    iter: I,
}

impl<I> List<I> {
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl<I: Iterator<Item = ListEntry>> Iterator for List<I> {
    type Item = ListEntry;

    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next()
    }
}

pub struct Vcs(Arc<RwLock<Vec<Entry>>>);

impl Vcs {
    #[must_use]
    pub fn new() -> Self {
        Self(Arc::new(RwLock::new(vec![])))
    }

    /// # Errors
    #[instrument(skip(self, path))]
    pub fn list<'a, P>(&'a self, path: P) -> Result<List<impl Iterator<Item = ListEntry>>>
    where
        P: TryInto<&'a Path, Error = VcsError>,
    {
        let mut entry = self.0.read_arc();
        for component in path
            .try_into()?
            .components()
            .filter(|component| !component.is_empty())
        {
            if let Some(Entry { subdirs, .. }) = entry
                .iter()
                .find(|entry| entry.name().as_bytes() == component)
            {
                entry = subdirs.read_arc();
            } else {
                return Err(VcsError::DirNotFound);
            }
        }

        let mut v = entry
            .iter()
            .map(
                |Entry {
                     name,
                     subdirs,
                     releases,
                 }| {
                    let releases = releases.read();
                    if releases.is_empty() {
                        ListEntry::Dir(name.clone(), subdirs.read().len())
                    } else {
                        ListEntry::File(name.clone(), releases.len())
                    }
                },
            )
            .collect::<Vec<_>>();
        v.sort();

        Ok(List {
            len: v.len(),
            iter: v.into_iter(),
        })
    }

    fn get<F: FnOnce(&[Arc<Vec<u8>>]) -> Option<&Arc<Vec<u8>>>>(
        &self,
        path: &Path,
        f: F,
    ) -> Result<ReadRevision> {
        let mut entry = self.0.read_arc();
        let mut components = path
            .components()
            .filter(|component| !component.is_empty())
            .peekable();
        loop {
            match (components.next(), components.peek()) {
                (Some(dir), Some(_)) => {
                    if let Some(Entry { subdirs, .. }) =
                        entry.iter().find(|entry| entry.name().as_bytes() == dir)
                    {
                        entry = subdirs.read_arc();
                    } else {
                        return Err(VcsError::DirNotFound);
                    }
                }

                (Some(file), None) => {
                    if let Some(Entry { releases, .. }) =
                        entry.iter().find(|entry| entry.name().as_bytes() == file)
                    {
                        if let Some(release) = f(&releases.read_arc()) {
                            return Ok(ReadRevision {
                                data: release.clone(),
                                index: 0,
                            });
                        }
                        return Err(VcsError::ReleaseNotFound);
                    }
                }

                (None, _) => return Err(VcsError::FileNotFound),
            }
        }
    }

    /// # Errors
    #[instrument(skip(self, path))]
    pub fn get_current_revision<'a, P>(&'a self, path: P) -> Result<ReadRevision>
    where
        P: TryInto<&'a Path, Error = VcsError>,
    {
        self.get(path.try_into()?, <[_]>::last)
    }

    /// # Errors
    #[instrument(skip(self, path, revision))]
    pub fn get_revision<'a, P, R>(&'a self, path: P, revision: R) -> Result<ReadRevision>
    where
        P: TryInto<&'a Path, Error = VcsError>,
        R: Into<Revision>,
    {
        self.get(path.try_into()?, |vs| vs.get(revision.into().0))
    }

    /// # Errors
    pub fn put<'a, P>(&'a self, path: P) -> Result<WriteRevision<P>>
    where
        P: TryInto<&'a Path, Error = VcsError>,
    {
        Ok(WriteRevision {
            root: self.0.clone(),
            path,
            data: vec![],
        })
    }
}

impl Default for Vcs {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use parking_lot::Once;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    fn init_tracing_subscriber() {
        static INIT_TRACING_SUBSCRIBER: Once = Once::new();
        INIT_TRACING_SUBSCRIBER.call_once(tracing_subscriber::fmt::init);
    }

    #[test]
    fn test_simple_list() {
        init_tracing_subscriber();

        let vcs = Vcs::new();

        assert_eq!(vcs.list("/").unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_simple_put() {
        init_tracing_subscriber();

        let vcs = Vcs::new();

        let mut write = vcs.put("/test").unwrap();

        assert_eq!(vcs.list("/").unwrap().len(), 0);

        write.write_all(b"Hello World!").await.unwrap();

        assert_eq!(vcs.list("/").unwrap().len(), 0);

        write.commit().unwrap();

        assert_eq!(vcs.list("/").unwrap().len(), 1);
        assert_eq!(
            vcs.list("/").unwrap().next().unwrap(),
            ListEntry::File("test".to_string(), 1)
        );
    }

    #[tokio::test]
    async fn test_same_put() {
        init_tracing_subscriber();

        let vcs = Vcs::new();

        let mut write = vcs.put("/test").unwrap();
        write.write_all(b"Hello World!").await.unwrap();
        let r = write.commit().unwrap();

        let mut write = vcs.put("/test").unwrap();
        write.write_all(b"Hello World!").await.unwrap();
        assert_eq!(write.commit().unwrap(), r);

        assert_eq!(vcs.list("/").unwrap().len(), 1);
        assert_eq!(
            vcs.list("/").unwrap().next().unwrap(),
            ListEntry::File("test".to_string(), 1)
        );
    }

    #[tokio::test]
    async fn test_simple_get_current_revision() {
        init_tracing_subscriber();

        let vcs = Vcs::new();

        let mut write = vcs.put("/test").unwrap();

        write.write_all(b"Hello World!").await.unwrap();
        write.commit().unwrap();

        let mut read = vcs.get_current_revision("/test").unwrap();

        let mut buffer = vec![];
        read.read_to_end(&mut buffer).await.unwrap();

        assert_eq!(b"Hello World!".as_slice(), &buffer);
    }

    #[tokio::test]
    async fn test_simple_get_current_revision_2() {
        init_tracing_subscriber();

        let vcs = Vcs::new();

        let mut write = vcs.put("/test").unwrap();

        write.write_all(b"Hello World! 1").await.unwrap();
        write.commit().unwrap();

        let mut write = vcs.put("/test").unwrap();

        write.write_all(b"Hello World! 2").await.unwrap();
        write.commit().unwrap();

        let mut read = vcs.get_current_revision("/test").unwrap();

        let mut buffer = vec![];
        read.read_to_end(&mut buffer).await.unwrap();

        assert_eq!(b"Hello World! 2".as_slice(), &buffer);
    }

    #[tokio::test]
    async fn test_simple_get_current_revision_1() {
        init_tracing_subscriber();

        let vcs = Vcs::new();

        let mut write = vcs.put("/test").unwrap();

        write.write_all(b"Hello World! 1").await.unwrap();
        write.commit().unwrap();

        let mut write = vcs.put("/test").unwrap();

        write.write_all(b"Hello World! 2").await.unwrap();
        write.commit().unwrap();

        let mut read = vcs.get_revision("/test", 0).unwrap();

        let mut buffer = vec![];
        read.read_to_end(&mut buffer).await.unwrap();

        assert_eq!(b"Hello World! 1".as_slice(), &buffer);

        assert_eq!(
            vcs.list("/").unwrap().collect::<Vec<_>>(),
            vec![ListEntry::File("test".to_string(), 2)]
        );
    }

    #[tokio::test]
    async fn test_simple_get_current_revision_invalid() {
        init_tracing_subscriber();

        let vcs = Vcs::new();

        let mut write = vcs.put("/test").unwrap();
        write.write_all(b"Hello World! 1").await.unwrap();
        write.commit().unwrap();

        let mut write = vcs.put("/test").unwrap();
        write.write_all(b"Hello World! 2").await.unwrap();
        write.commit().unwrap();

        assert_eq!(
            VcsError::ReleaseNotFound,
            vcs.get_revision("/test", 3).unwrap_err()
        );
    }

    #[tokio::test]
    async fn test_nexted_put() {
        init_tracing_subscriber();

        let vcs = Vcs::new();

        let mut write = vcs.put("/dir/test").unwrap();

        assert_eq!(vcs.list("/").unwrap().len(), 0);

        write.write_all(b"Hello World!").await.unwrap();

        assert_eq!(vcs.list("/").unwrap().len(), 0);

        write.commit().unwrap();

        assert_eq!(vcs.list("/").unwrap().len(), 1);
        assert_eq!(
            vcs.list("/").unwrap().next().unwrap(),
            ListEntry::Dir("dir".to_string(), 1)
        );

        assert_eq!(vcs.list("/dir").unwrap().len(), 1);
        assert_eq!(
            vcs.list("/dir").unwrap().next().unwrap(),
            ListEntry::File("test".to_string(), 1)
        );
    }

    #[tokio::test]
    async fn test_nexted_put_valid() {
        init_tracing_subscriber();

        let vcs = Vcs::new();

        let mut write = vcs.put("/dir/test").unwrap();
        write.write_all(b"Hello World!").await.unwrap();
        write.commit().unwrap();

        let mut write = vcs.put("/dir").unwrap();
        write.write_all(b"Hello World!").await.unwrap();
        assert_eq!(Ok(0), write.commit());
    }

    #[test]
    fn test_is_valid() {
        Path::new("/e4oq1-HLbqZW77rFpqerxYCBm7oPI-mkWXXYLC5C6-KRPp7lwXb_f_YUyVH-t4w53oKLzUhKTmqcfdHV_zX8ESbJv9tghYEnoew_lXGVEirTq3t.m_NyUMbW-pq9zXYpRwuDoK7pTtP47mhD-vlbnVLkspE5/cDoC.yg/-wagwgLrifbiR9vkdcD5y5-XZ1e6T/eRXRuLhcRecptRO6sx.GCoBw9KgRXWEKfgvWKeEODullCe".as_bytes()).unwrap();
    }
}
