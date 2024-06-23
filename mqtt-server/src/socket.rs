use crate::codec::MqttCodec;
use crate::distributor::{Distributor, InnerDistributorMutex};
use core::num::NonZeroU16;
use embassy_futures::select::select;
use embassy_futures::select::Either::{First, Second};
use embassy_net::tcp::TcpSocket;
use embassy_net::Stack;
use embassy_net_driver::Driver;
use embassy_time::Duration;
use log::{debug, info, warn};
use mqtt_format::v5::packets::connack::{ConnackProperties, ConnackReasonCode, MConnack};
use mqtt_format::v5::packets::pingresp::MPingresp;
use mqtt_format::v5::packets::puback::{MPuback, PubackProperties, PubackReasonCode};
use mqtt_format::v5::packets::publish::{MPublish, PublishProperties};
use mqtt_format::v5::packets::suback::{MSuback, SubackProperties};
use mqtt_format::v5::packets::MqttPacket;
use mqtt_format::v5::qos::QualityOfService;
use mqtt_format::v5::variable_header::PacketIdentifier;

pub async fn listen<T, const N: usize>(
    stack: &'static Stack<T>,
    id: usize,
    port: u16,
    distributor: &'static InnerDistributorMutex<N>,
) where
    T: Driver,
{
    let mut rx_buffer = [0; 1600];
    let mut tx_buffer = [0; 1600];
    let distributor = Distributor::new(distributor, id).await;

    loop {
        // after previous connection is closed, unsubscribe from all topics
        distributor.unsubscibe_all_topics().await;

        let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
        socket.set_timeout(Some(Duration::from_secs(10)));
        socket.set_keep_alive(Some(Duration::from_secs(1)));
        info!("SOCKET {}: Listening on TCP:{}...", id, port);
        if let Err(e) = socket.accept(port).await {
            warn!("accept error: {:?}", e);
            continue;
        }
        info!(
            "SOCKET {}: Received connection from {}",
            id,
            socket.remote_endpoint().unwrap()
        );
        // connection handler
        let mut parser = MqttCodec::<TcpSocket, 1024>::new(socket);
        match parser.next().await {
            Ok(Some(MqttPacket::Connect(_connect))) => {
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

        loop {
            let either = select(parser.next(), distributor.next()).await;
            let packet = match either {
                First(Ok(Some(packet))) => packet,
                First(Ok(None)) => {
                    // socket closed
                    break;
                }
                First(Err(e)) => {
                    // socket error, like connection reset by peer
                    warn!("SOCKET {}: {:?}", id, e);
                    break;
                }
                Second(msg) => {
                    let packet = MqttPacket::Publish(MPublish {
                        duplicate: false,
                        quality_of_service: QualityOfService::AtMostOnce,
                        retain: false,
                        topic_name: msg.topic(),
                        packet_identifier: None,
                        properties: PublishProperties::new(),
                        payload: msg.message(),
                    });
                    parser.write(packet).await.unwrap();
                    continue;
                }
            };
            // react on actual packets received by the socket
            match packet {
                MqttPacket::Publish(publish) => {
                    distributor
                        .publish(publish.topic_name, publish.payload)
                        .await;
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
                    for s in subscribe.subscriptions.iter() {
                        debug!("subscribing to: {}", s.topic_filter);
                        distributor.subscribe(s.topic_filter).await;
                    }
                    let pkg = MqttPacket::Suback(MSuback {
                        packet_identifier: subscribe.packet_identifier,
                        properties: SubackProperties::new(),
                        reasons: &[],
                    });
                    parser.write(pkg).await.unwrap();
                }
                MqttPacket::Disconnect(_disconnect) => {
                    info!("SOCKET {}: disconnecting", id);
                    break;
                }
                MqttPacket::Pingreq(_pingreq) => {
                    let pkg = MqttPacket::Pingresp(MPingresp {});
                    parser.write(pkg).await.unwrap();
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
