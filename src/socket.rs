use crate::codec::MqttCodec;
use crate::MAX_CONNECTIONS;
use core::num::NonZeroU16;
use embassy_net::tcp::TcpSocket;
use embassy_net::Stack;
use embassy_time::Duration;
use esp_wifi::wifi::{WifiDevice, WifiStaDevice};
use log::{info, warn};
use mqtt_format::v5::packets::connack::{ConnackProperties, ConnackReasonCode, MConnack};
use mqtt_format::v5::packets::puback::{MPuback, PubackProperties, PubackReasonCode};
use mqtt_format::v5::packets::suback::{MSuback, SubackProperties};
use mqtt_format::v5::packets::MqttPacket;
use mqtt_format::v5::variable_header::PacketIdentifier;

#[embassy_executor::task(pool_size = MAX_CONNECTIONS)]
pub async fn listen_task(
    stack: &'static Stack<WifiDevice<'static, WifiStaDevice>>,
    id: usize,
    port: u16,
) {
    let mut rx_buffer = [0; 1600];
    let mut tx_buffer = [0; 1600];

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
        // connection handler
        let mut parser = MqttCodec::<TcpSocket, 1024>::new(socket);
        match parser.next().await {
            Ok(Some(MqttPacket::Connect(_connect))) => {
                info!("decoded packet connect packet, sending ack...");
                let pkg = MqttPacket::Connack(MConnack {
                    session_present: false,
                    reason_code: ConnackReasonCode::Success,
                    properties: ConnackProperties::new(),
                });
                parser.write(pkg).await.unwrap();
            }
            _ => {
                warn!("SOCKET {}: error decoding packet", id);
                parser.get_mut().close();
                continue;
            }
        }

        info!("connected");

        loop {
            let packet = parser.next().await;
            let packet = match packet {
                Ok(Some(packet)) => packet,
                Ok(None) => {
                    break;
                }
                Err(e) => {
                    warn!("SOCKET {}: {:?}", id, e);
                    break;
                }
            };

            match packet {
                MqttPacket::Publish(publish) => {
                    info!(
                        "publish: {} the following data: {}",
                        publish.topic_name,
                        core::str::from_utf8(publish.payload).unwrap_or("<invalid utf8>")
                    );
                    let pkg = MqttPacket::Puback(MPuback {
                        packet_identifier: publish
                            .packet_identifier
                            .unwrap_or(PacketIdentifier(NonZeroU16::new(1).unwrap())),
                        reason: PubackReasonCode::Success,
                        properties: PubackProperties::new(),
                    });
                    parser.write(pkg).await.unwrap();
                }
                MqttPacket::Subscribe(subscribe) => {
                    info!("subscribe: {:?}", subscribe);
                    let pkg = MqttPacket::Suback(MSuback {
                        packet_identifier: subscribe.packet_identifier,
                        properties: SubackProperties::new(),
                        reasons: &[],
                    });
                    parser.write(pkg).await.unwrap();
                }
                MqttPacket::Disconnect(disconnect) => {
                    info!("disconnect: {:?}", disconnect);
                    break;
                }
                pkg => {
                    warn!("SOCKET: unexpected packet {:?}", pkg);
                    break;
                }
            }
        }
        parser.get_mut().close();
    }
}
