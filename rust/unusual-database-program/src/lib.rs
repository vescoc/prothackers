use std::collections::HashMap;

use tokio::net::UdpSocket;

use tracing::debug;

#[tracing::instrument(skip(socket))]
pub async fn run(socket: UdpSocket) -> Result<(), anyhow::Error> {
    debug!("socket addr: {:?} ttl: {:?}", socket.local_addr(), socket.ttl());
    
    let mut data = HashMap::new();

    let mut buffer = [0; 1024];
    loop {
        let (len, addr) = socket.recv_from(&mut buffer).await?;

        let buffer = &buffer[0..len];
        let mut iter = buffer.iter();
        if let Some(position) = iter.position(|c| *c == b'=') {
            let key = &buffer[0..position];
            if key != b"version" {
                let value = iter.as_slice();
                data.insert(key.to_vec(), value.to_vec());
                debug!("[{:?}] insert key: {:?} value: {:?}", addr, std::str::from_utf8(key), std::str::from_utf8(value));
            }
        } else {
            let key = buffer;
            debug!("[{:?}] retrieve key: {:?}", addr, std::str::from_utf8(key));
            if key == b"version" {
                socket
                    .send_to(b"unusual-database-program 1.0.0", addr)
                    .await?;
            } else if let Some(value) = data.get(key) {
                debug!("[{:?}] value: {:?}", addr, std::str::from_utf8(value));
                socket.send_to(value, addr).await?;
            } else {
                debug!("[{:?}] no value", addr);
            }
        }
    }
}
