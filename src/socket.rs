use core::ops::Sub;
use crate::distributor::Distributor;
use crate::MAX_CONNECTIONS;
use embassy_futures::select::select;
use embassy_futures::select::Either::{First, Second};
use embassy_net::tcp::TcpSocket;
use embassy_net::Stack;
use embassy_time::{Duration, Instant};
use embedded_io_async::{Write, WriteReady};
use esp_wifi::wifi::{WifiDevice, WifiStaDevice};
use futures_util::StreamExt;
use log::{error, info, warn};

#[embassy_executor::task(pool_size = MAX_CONNECTIONS)]
pub async fn listen_task(
    stack: &'static Stack<WifiDevice<'static, WifiStaDevice>>,
    id: usize,
    port: u16,
    distributor: &'static Distributor,
) {
    let mut rx_buffer = [0; 1600];
    let mut tx_buffer = [0; 1600];
    let mut buf = [0; 64];

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

        let publisher = distributor.add_publisher("hi").await;
        let mut subscriber = distributor.add_subscriber("hi").await;
        let mut subscriber2 = distributor.add_subscriber("hi").await;
        'inner: loop {
            let _ = subscriber2.try_recv();
            let _ = subscriber2.try_recv();
            let _ = subscriber2.try_recv();
            let n = match select(socket.read(&mut buf), subscriber.next()).await {
                First(Ok(0)) => {
                    warn!("read EOF");
                    break;
                }
                First(Ok(n)) => n,
                First(Err(e)) => {
                    warn!("SOCKET {}: {:?}", id, e);
                    break;
                }
                Second(Some(e)) => {
                    info!(
                        "received something form channel {}",
                        core::str::from_utf8(&e[..]).unwrap_or_else(|_| "<invalid utf8>")
                    );
                    if socket.write_ready().is_err() {
                        error!("{:?}", socket.write_ready());
                    }
                    let time = Instant::now();
                    socket
                        .write_all(e.as_slice())
                        .await
                        .unwrap();
                    let end = Instant::now();
                    info!("send took: {}ms", end.sub(time).as_millis());
                    continue 'inner;
                }
                Second(None) => {
                    warn!("channel closed");
                    break;
                }
            };

            publisher
                .broadcast(buf[..n].to_vec())
                .await
                .unwrap();

            info!(
                "SOCKET {}: rxd {}",
                id,
                core::str::from_utf8(&buf[..n]).unwrap_or_else(|_| "<invalid utf8>")
            );
        }
        socket.close();
    }
}
