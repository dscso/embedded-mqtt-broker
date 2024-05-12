use core::ops::Sub;
use crate::MAX_CONNECTIONS;
use embassy_net::tcp::TcpSocket;
use embassy_net::Stack;
use embassy_time::{Duration, Instant};
use embedded_io_async::{Write, WriteReady};
use esp_wifi::wifi::{WifiDevice, WifiStaDevice};
use log::{error, info, warn};
use minimq::de::deserializer;
use minimq::de::packet_reader::PacketReader;
use minimq::de::received_packet::ReceivedPacket;
use minimq::ProtocolError::Deserialization;

#[embassy_executor::task(pool_size = MAX_CONNECTIONS)]
pub async fn listen_task(
    stack: &'static Stack<WifiDevice<'static, WifiStaDevice>>,
    id: usize,
    port: u16
) {
    let mut rx_buffer = [0; 1600];
    let mut tx_buffer = [0; 1600];
    let mut buf = [0; 1024*2];
    //let mut reader = PacketReader::new(&mut buf);

    loop {
        let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
        socket.set_timeout(Some(Duration::from_secs(10)));
        socket.set_keep_alive(Some(Duration::from_secs(1)));

        info!("SOCKET {}: Listening on TCP:{}...", id, port);
        if let Err(e) = socket.accept(port).await {
            warn!("accept error: {:?}", e);
            continue;
        }
        info!(
            "SOCKET {}: Received connection from {:?}",
            id,
            socket.remote_endpoint()
        );

        loop {
            let n = match socket.read(&mut buf).await {
                Ok(0) => {
                    warn!("read EOF");
                    break;
                },
                Ok(n) => n,
                Err(e) => {
                    warn!("SOCKET {}: {:?}", id, e);
                    break;
                }
            };
            /*let packet = ReceivedPacket::from_buffer(&buf[..n]);
            info!("rcv {n}bytes: {:?} {:?}", packet, &buf[..n]);*/
            let ack = [
                0x20u8, 0x03, // Remaining length = 3 bytes
                0x00, // Connect acknowledge flags - bit 0 clear.
                0x00, // Connect reason code - 0 (Success)
                0x00, // Property length = 0
                // No payload.
            ];
            socket.write(&ack).await.unwrap();
            /*let time = Instant::now();
            socket
                .write_all(&buf[..n])
                .await
                .unwrap();
            let end = Instant::now();
            info!("send took: {}us", end.sub(time).as_micros());

            info!(
                "SOCKET {}: rxd {}",
                id,
                core::str::from_utf8(&buf[..n]).unwrap_or_else(|_| "<invalid utf8>")
            );*/
        }
        socket.close();
    }
}
