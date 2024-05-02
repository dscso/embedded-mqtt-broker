use crate::MAX_CONNECTIONS;
use embassy_futures::select::select;
use embassy_futures::select::Either::{First, Second};
use embassy_net::tcp::TcpSocket;
use embassy_net::Stack;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::pubsub::{Publisher, Subscriber, WaitResult};
use embassy_time::Duration;
use embedded_io_async::Write;
use esp_wifi::wifi::{WifiDevice, WifiStaDevice};
use log::{info, warn};

#[embassy_executor::task(pool_size = MAX_CONNECTIONS)]
pub async fn listen_task(
    stack: &'static Stack<WifiDevice<'static, WifiStaDevice>>,
    id: usize,
    port: u16,
    sender: &'static Publisher<'static, NoopRawMutex, u8, 100, 2, 2>,
    receiver: &'static mut Subscriber<'static, NoopRawMutex, u8, 100, 2, 2>,
) {
    let mut rx_buffer = [0; 1024];
    let mut tx_buffer = [0; 1024];
    let mut buf = [0; 64];
    loop {
        let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
        socket.set_timeout(Some(Duration::from_secs(10)));
        socket.set_keep_alive(Some(Duration::from_secs(5)));

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

        'inner: loop {
            let n = match select(socket.read(&mut buf), receiver.next_message()).await {
                First(Ok(0)) => {
                    warn!("read EOF");
                    break;
                }
                First(Ok(n)) => n,
                First(Err(e)) => {
                    warn!("SOCKET {}: {:?}", id, e);
                    break;
                }
                Second(WaitResult::Message(e)) => {
                    socket.write_all(&[e]).await.expect("TODO: panic message");
                    continue 'inner;
                }
                Second(WaitResult::Lagged(e)) => {
                    warn!("lagged {e}");
                    continue 'inner;
                    //break;
                }
            };
            for i in 0..n {
                sender.publish_immediate(buf[i]);
            }
            info!(
                "SOCKET {}: rxd {}",
                id,
                core::str::from_utf8(&buf[..n]).unwrap_or_else(|_| "<invalid utf8>")
            );
        }
        if let Err(e) = socket.flush().await {
            warn!("flush error: {:?}", e);
        }
        socket.close();
    }
}
