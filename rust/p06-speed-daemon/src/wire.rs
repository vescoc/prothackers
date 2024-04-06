use tokio::io::{self, AsyncReadExt, AsyncWriteExt};

#[derive(thiserror::Error, Debug)]
pub enum ReadError {
    #[error("internal error")]
    InternalError(#[from] io::Error),

    #[error("invalid message: 0x{0:2x}")]
    InvalidMessage(u8),
}

pub trait TaggedMessage {
    const TAG: u8;
}

pub trait WriteTo: TaggedMessage {
    #[allow(async_fn_in_trait)]
    async fn write_to<W: AsyncWriteExt + Unpin>(&self, write: &mut W) -> Result<(), io::Error> {
        write.write_u8(Self::TAG).await?;
        self.write_payload_to(write).await
    }

    #[allow(async_fn_in_trait)]
    async fn write_payload_to<W: AsyncWriteExt + Unpin>(
        &self,
        write: &mut W,
    ) -> Result<(), io::Error>;
}

pub trait ReadFrom: Sized + TaggedMessage {
    #[allow(async_fn_in_trait)]
    async fn read_from<R: AsyncReadExt + Unpin>(read: &mut R) -> Result<Self, ReadError> {
        let tag = read.read_u8().await?;
        if tag == Self::TAG {
            Self::read_payload_from(read).await
        } else {
            Err(ReadError::InvalidMessage(tag))
        }
    }

    #[allow(async_fn_in_trait)]
    async fn read_payload_from<R: AsyncReadExt + Unpin>(read: &mut R) -> Result<Self, ReadError>;
}

#[derive(Debug, PartialEq)]
pub struct Error {
    pub msg: String,
}

impl TaggedMessage for Error {
    const TAG: u8 = 0x10;
}

impl WriteTo for Error {
    #[allow(clippy::cast_possible_truncation)]
    async fn write_payload_to<W: AsyncWriteExt + Unpin>(
        &self,
        write: &mut W,
    ) -> Result<(), io::Error> {
        write.write_u8(self.msg.len() as u8).await?;
        write.write_all(self.msg.as_bytes()).await
    }
}

impl ReadFrom for Error {
    async fn read_payload_from<R: AsyncReadExt + Unpin>(read: &mut R) -> Result<Self, ReadError> {
        let mut buffer = [0; 256];

        let len = read.read_u8().await.map_err(ReadError::InternalError)? as usize;
        read.read_exact(&mut buffer[..len])
            .await
            .map_err(ReadError::InternalError)?;
        Ok(Self {
            msg: String::from_utf8_lossy(&buffer[..len]).into_owned(),
        })
    }
}

#[derive(Debug, PartialEq)]
pub struct Plate {
    pub plate: String,
    pub timestamp: u32,
}

impl TaggedMessage for Plate {
    const TAG: u8 = 0x20;
}

impl WriteTo for Plate {
    #[allow(clippy::cast_possible_truncation)]
    async fn write_payload_to<W: AsyncWriteExt + Unpin>(
        &self,
        write: &mut W,
    ) -> Result<(), io::Error> {
        write.write_u8(self.plate.len() as u8).await?;
        write.write_all(self.plate.as_bytes()).await?;
        write.write_u32(self.timestamp).await
    }
}

impl ReadFrom for Plate {
    async fn read_payload_from<R: AsyncReadExt + Unpin>(read: &mut R) -> Result<Self, ReadError> {
        let mut buffer = [0; 256];

        let len = read.read_u8().await? as usize;
        read.read_exact(&mut buffer[..len]).await?;
        Ok(Self {
            plate: String::from_utf8_lossy(&buffer[..len]).into_owned(),
            timestamp: read.read_u32().await?,
        })
    }
}

#[derive(Debug, PartialEq)]
pub struct Ticket {
    pub plate: String,
    pub road: u16,
    pub mile1: u16,
    pub timestamp1: u32,
    pub mile2: u16,
    pub timestamp2: u32,
    pub speed: u16,
}

impl TaggedMessage for Ticket {
    const TAG: u8 = 0x21;
}

impl WriteTo for Ticket {
    #[allow(clippy::cast_possible_truncation)]
    async fn write_payload_to<W: AsyncWriteExt + Unpin>(
        &self,
        write: &mut W,
    ) -> Result<(), io::Error> {
        write.write_u8(self.plate.len() as u8).await?;
        write.write_all(self.plate.as_bytes()).await?;
        write.write_u16(self.road).await?;
        write.write_u16(self.mile1).await?;
        write.write_u32(self.timestamp1).await?;
        write.write_u16(self.mile2).await?;
        write.write_u32(self.timestamp2).await?;
        write.write_u16(self.speed).await
    }
}

impl ReadFrom for Ticket {
    async fn read_payload_from<R: AsyncReadExt + Unpin>(read: &mut R) -> Result<Self, ReadError> {
        let mut buffer = [0; 256];

        let len = read.read_u8().await.map_err(ReadError::InternalError)? as usize;
        read.read_exact(&mut buffer[..len])
            .await
            .map_err(ReadError::InternalError)?;

        Ok(Self {
            plate: String::from_utf8_lossy(&buffer[..len]).into_owned(),
            road: read.read_u16().await.map_err(ReadError::InternalError)?,
            mile1: read.read_u16().await.map_err(ReadError::InternalError)?,
            timestamp1: read.read_u32().await.map_err(ReadError::InternalError)?,
            mile2: read.read_u16().await.map_err(ReadError::InternalError)?,
            timestamp2: read.read_u32().await.map_err(ReadError::InternalError)?,
            speed: read.read_u16().await.map_err(ReadError::InternalError)?,
        })
    }
}

#[derive(Debug, PartialEq)]
pub struct WantHeartbeat {
    pub interval: u32,
}

impl TaggedMessage for WantHeartbeat {
    const TAG: u8 = 0x40;
}

impl WriteTo for WantHeartbeat {
    async fn write_payload_to<W: AsyncWriteExt + Unpin>(
        &self,
        write: &mut W,
    ) -> Result<(), io::Error> {
        write.write_u32(self.interval).await
    }
}

impl ReadFrom for WantHeartbeat {
    async fn read_payload_from<R: AsyncReadExt + Unpin>(read: &mut R) -> Result<Self, ReadError> {
        Ok(Self {
            interval: read.read_u32().await.map_err(ReadError::InternalError)?,
        })
    }
}

#[derive(Debug, PartialEq)]
pub struct Heartbeat;

impl TaggedMessage for Heartbeat {
    const TAG: u8 = 0x41;
}

impl WriteTo for Heartbeat {
    async fn write_payload_to<W: AsyncWriteExt + Unpin>(&self, _: &mut W) -> Result<(), io::Error> {
        Ok(())
    }
}

impl ReadFrom for Heartbeat {
    async fn read_payload_from<R: AsyncReadExt + Unpin>(_: &mut R) -> Result<Self, ReadError> {
        Ok(Self)
    }
}

#[derive(Debug, PartialEq)]
pub struct IAmCamera {
    pub road: u16,
    pub mile: u16,
    pub limit: u16,
}

impl TaggedMessage for IAmCamera {
    const TAG: u8 = 0x80;
}

impl WriteTo for IAmCamera {
    async fn write_payload_to<W: AsyncWriteExt + Unpin>(
        &self,
        write: &mut W,
    ) -> Result<(), io::Error> {
        write.write_u16(self.road).await?;
        write.write_u16(self.mile).await?;
        write.write_u16(self.limit).await
    }
}

impl ReadFrom for IAmCamera {
    async fn read_payload_from<R: AsyncReadExt + Unpin>(read: &mut R) -> Result<Self, ReadError> {
        Ok(Self {
            road: read.read_u16().await.map_err(ReadError::InternalError)?,
            mile: read.read_u16().await.map_err(ReadError::InternalError)?,
            limit: read.read_u16().await.map_err(ReadError::InternalError)?,
        })
    }
}

#[derive(Debug, PartialEq)]
pub struct IAmDispatcher {
    pub roads: Vec<u16>,
}

impl TaggedMessage for IAmDispatcher {
    const TAG: u8 = 0x81;
}

impl WriteTo for IAmDispatcher {
    #[allow(clippy::cast_possible_truncation)]
    async fn write_payload_to<W: AsyncWriteExt + Unpin>(
        &self,
        write: &mut W,
    ) -> Result<(), io::Error> {
        write.write_u8(self.roads.len() as u8).await?;
        for road in &self.roads {
            write.write_u16(*road).await?;
        }
        Ok(())
    }
}

impl ReadFrom for IAmDispatcher {
    async fn read_payload_from<R: AsyncReadExt + Unpin>(read: &mut R) -> Result<Self, ReadError> {
        let len = read.read_u8().await.map_err(ReadError::InternalError)?;
        let mut roads = Vec::with_capacity(len as usize);
        for _ in 0..len {
            roads.push(read.read_u16().await.map_err(ReadError::InternalError)?);
        }
        Ok(Self { roads })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[allow(non_snake_case)]
    async fn test_read_Plate() {
        let buffer = [0x20, 0x04, 0x55, 0x4e, 0x31, 0x58, 0x00, 0x00, 0x03, 0xe8];
        let mut stream = &buffer[..];

        assert_eq!(
            Plate {
                plate: "UN1X".to_string(),
                timestamp: 1000
            },
            Plate::read_from(&mut stream).await.unwrap()
        );
    }

    #[tokio::test]
    #[allow(non_snake_case)]
    async fn test_write_Plate() {
        let mut buffer = vec![];

        Plate {
            plate: "UN1X".to_string(),
            timestamp: 1000,
        }
        .write_to(&mut buffer)
        .await
        .unwrap();

        assert_eq!(
            &buffer,
            &[0x20, 0x04, 0x55, 0x4e, 0x31, 0x58, 0x00, 0x00, 0x03, 0xe8]
        );
    }

    #[tokio::test]
    #[allow(non_snake_case)]
    async fn test_read_IAmCamera() {
        let buffer = [0x80, 0x00, 0x42, 0x00, 0x64, 0x00, 0x3c];
        let mut stream = &buffer[..];

        assert_eq!(
            IAmCamera {
                road: 66,
                mile: 100,
                limit: 60
            },
            IAmCamera::read_from(&mut stream).await.unwrap()
        );
    }

    #[tokio::test]
    #[allow(non_snake_case)]
    async fn test_write_IAmCamera() {
        let mut buffer = vec![];

        IAmCamera {
            road: 66,
            mile: 100,
            limit: 60,
        }
        .write_to(&mut buffer)
        .await
        .unwrap();

        assert_eq!(&buffer, &[0x80, 0x00, 0x42, 0x00, 0x64, 0x00, 0x3c],);
    }

    #[tokio::test]
    #[allow(non_snake_case)]
    async fn test_read_IAmDispatcher() {
        let buffer = [0x81, 0x01, 0x00, 0x42];
        let mut stream = &buffer[..];

        assert_eq!(
            IAmDispatcher { roads: vec![66] },
            IAmDispatcher::read_from(&mut stream).await.unwrap()
        );
    }

    #[tokio::test]
    #[allow(non_snake_case)]
    async fn test_write_IAmDispatcher() {
        let mut buffer = vec![];

        IAmDispatcher { roads: vec![66] }
            .write_to(&mut buffer)
            .await
            .unwrap();

        assert_eq!(&buffer, &[0x81, 0x01, 0x00, 0x42],);
    }

    #[tokio::test]
    #[allow(non_snake_case)]
    async fn test_read_WantHeartbeat() {
        let buffer = [0x40, 0x00, 0x00, 0x00, 0x0a];
        let mut stream = &buffer[..];

        assert_eq!(
            WantHeartbeat { interval: 10 },
            WantHeartbeat::read_from(&mut stream).await.unwrap()
        );
    }

    #[tokio::test]
    #[allow(non_snake_case)]
    async fn test_write_WantHeartbeat() {
        let mut buffer = vec![];

        WantHeartbeat { interval: 10 }
            .write_to(&mut buffer)
            .await
            .unwrap();

        assert_eq!(&buffer, &[0x40, 0x00, 0x00, 0x00, 0x0a]);
    }

    #[tokio::test]
    #[allow(non_snake_case)]
    async fn test_write_Error() {
        let mut buffer = vec![];

        Error {
            msg: "bad".to_string(),
        }
        .write_to(&mut buffer)
        .await
        .unwrap();

        assert_eq!(buffer, vec![0x10, 0x03, 0x62, 0x61, 0x64]);
    }

    #[tokio::test]
    #[allow(non_snake_case)]
    async fn test_read_Error() {
        let buffer = [0x10, 0x03, 0x62, 0x61, 0x64];
        let mut stream = &buffer[..];

        assert_eq!(
            Error {
                msg: "bad".to_string(),
            },
            Error::read_from(&mut stream).await.unwrap()
        );
    }

    #[tokio::test]
    #[allow(non_snake_case)]
    async fn test_write_Ticket() {
        let mut buffer = vec![];

        Ticket {
            plate: "UN1X".to_string(),
            road: 66,
            mile1: 100,
            timestamp1: 123456,
            mile2: 110,
            timestamp2: 123816,
            speed: 10000,
        }
        .write_to(&mut buffer)
        .await
        .unwrap();

        assert_eq!(
            buffer,
            vec![
                0x21, 0x04, 0x55, 0x4e, 0x31, 0x58, 0x00, 0x42, 0x00, 0x64, 0x00, 0x01, 0xe2, 0x40,
                0x00, 0x6e, 0x00, 0x01, 0xe3, 0xa8, 0x27, 0x10
            ]
        );
    }

    #[tokio::test]
    #[allow(non_snake_case)]
    async fn test_read_Ticket() {
        let buffer = [
            0x21, 0x04, 0x55, 0x4e, 0x31, 0x58, 0x00, 0x42, 0x00, 0x64, 0x00, 0x01, 0xe2, 0x40,
            0x00, 0x6e, 0x00, 0x01, 0xe3, 0xa8, 0x27, 0x10,
        ];
        let mut stream = &buffer[..];

        assert_eq!(
            Ticket {
                plate: "UN1X".to_string(),
                road: 66,
                mile1: 100,
                timestamp1: 123456,
                mile2: 110,
                timestamp2: 123816,
                speed: 10000,
            },
            Ticket::read_from(&mut stream).await.unwrap()
        );
    }

    #[tokio::test]
    #[allow(non_snake_case)]
    async fn test_write_Heartbeat() {
        let mut buffer = vec![];

        Heartbeat.write_to(&mut buffer).await.unwrap();

        assert_eq!(buffer, vec![0x41]);
    }

    #[tokio::test]
    #[allow(non_snake_case)]
    async fn test_read_Heartbeat() {
        let buffer = [0x41];
        let mut stream = &buffer[..];

        assert_eq!(Heartbeat, Heartbeat::read_from(&mut stream).await.unwrap());
    }
}
