use core::num::NonZeroU16;
use embassy_futures::select::select;
use embassy_futures::select::Either::{First, Second};
use embassy_net::tcp::TcpSocket;
use embassy_net::Stack;
use embassy_net_driver::Driver;
use embassy_time::{with_timeout, Duration};
use embedded_io_async::{Read, Write};
use heapless::Vec;
use log::{info, warn};
use mqtt_format::v5::packets::connack::{ConnackProperties, ConnackReasonCode, MConnack};
use mqtt_format::v5::packets::disconnect::{DisconnectProperties, MDisconnect};
use mqtt_format::v5::packets::pingresp::MPingresp;
use mqtt_format::v5::packets::puback::{MPuback, PubackProperties, PubackReasonCode};
use mqtt_format::v5::packets::publish::{MPublish, PublishProperties};
use mqtt_format::v5::packets::suback::{MSuback, SubackProperties, SubackReasonCode};
use mqtt_format::v5::packets::unsuback::{MUnsuback, UnsubackProperties};
use mqtt_format::v5::packets::MqttPacket;
use mqtt_format::v5::qos::QualityOfService;
use mqtt_format::v5::variable_header::PacketIdentifier;

use crate::codec::{MqttCodecDecoder, MqttCodecEncoder};
use crate::config::InnerDistributorMutex;
use crate::distributor::Distributor;
use crate::errors::DistributorError;

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
    let mut distributor = Distributor::new(distributor, id);

    loop {
        // cleanup previous connection settings
        // unlocks distributor as well
        distributor.cleanup();
        distributor.fulfill_will().await;

        let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
        socket.set_timeout(Some(Duration::from_secs(60)));
        socket.set_keep_alive(Some(Duration::from_secs(10)));
        info!("SOCKET {}: Listening on TCP:{}...", id, port);
        if let Err(e) = socket.accept(port).await {
            warn!("accept error: {:?}", e);
            continue;
        }
        // sometimes this fails since connection is closed immediately after accept 🤷‍
        let addr = match socket.remote_endpoint() {
            Some(addr) => addr,
            None => {
                warn!("SOCKET {}: could not get remote endpoint", id);
                continue;
            }
        };
        info!("SOCKET {}: Received connection from {}", id, addr);
        let (reader, writer) = socket.split();
        // connection handler
        let mut parser = MqttCodecDecoder::<_, 1024>::new(reader);
        let mut encoder = MqttCodecEncoder::<_, 1024>::new(writer);

        info!("SOCKET {}: Handshaking...", id);
        let timout = Duration::from_secs(10);
        match with_timeout(timout, parser.next()).await {
            Ok(Ok(Some(MqttPacket::Connect(connect)))) => {
                if let Some(conn_will) = connect.will {
                    let will = MPublish {
                        duplicate: false,
                        topic_name: conn_will.topic,
                        payload: conn_will.payload,
                        retain: conn_will.will_retain,
                        properties: PublishProperties::new(),
                        packet_identifier: None,
                        quality_of_service: QualityOfService::AtMostOnce,
                    };
                    if let Err(e) = distributor.set_will(will) {
                        warn!("SOCKET {}: error setting will {:?}", id, e);
                        continue;
                    }
                    info!("SOCKET {}: will topic: {}", id, conn_will.topic);
                }
                let pkg = MqttPacket::Connack(MConnack {
                    session_present: false,
                    reason_code: ConnackReasonCode::Success,
                    properties: ConnackProperties::new(),
                });
                if let Err(e) = encoder.write(pkg).await {
                    warn!("SOCKET {}: {:?}", id, e);
                    continue;
                }
            }
            Err(_e) => {
                warn!("SOCKET {}: connection to first packet timeout...", id);
                continue;
            }
            Ok(e) => {
                warn!("SOCKET {}: error decoding packet {:?}", id, e);
                continue;
            }
        }

        if let Err(error) = handle_socket(&mut parser, &mut encoder, &distributor).await {
            warn!("SOCKET {}: {:?}", id, error);
            let error = MqttPacket::Disconnect(MDisconnect {
                reason_code: error.into(),
                properties: DisconnectProperties::new(),
            });
            if let Err(e) = encoder.write(error).await {
                warn!(
                    "SOCKET {}: could not close connection because of {:?}",
                    id, e
                );
            }
        }
    }
}

async fn handle_socket<
    T,
    U,
    const DECODER_SIZE: usize,
    const ENCODER_SIZE: usize,
    const CONNECTIONS: usize,
>(
    parser: &mut MqttCodecDecoder<T, DECODER_SIZE>,
    encoder: &mut MqttCodecEncoder<U, ENCODER_SIZE>,
    distributor: &Distributor<CONNECTIONS>,
) -> Result<(), DistributorError>
where
    T: Read,
    U: Write,
{
    loop {
        // unlock after processing packet
        distributor.unlock();
        let selected = select(distributor.next(), distributor.lock(parser.next())).await;
        let packet = match selected {
            First(msg) => {
                let mut packet = MqttPacket::parse_complete(msg.message()).unwrap();

                if let MqttPacket::Publish(ref mut publish) = packet {
                    publish.packet_identifier = None;
                }

                encoder
                    .write(packet)
                    .await
                    .map_err(|_| DistributorError::Unknown)?;
                continue;
            }
            Second(Ok(Some(packet))) => packet,
            Second(Ok(None)) => {
                // socket closed
                return Ok(());
            }
            Second(Err(e)) => {
                // socket error, like connection reset by peer
                warn!("SOCKET: {:?}", e);
                return Err(DistributorError::Unknown);
            }
        };

        match packet {
            MqttPacket::Publish(publish) => {
                distributor.publish(publish.topic_name, &publish)?;
                let packet_identifier = publish
                    .packet_identifier
                    .unwrap_or(PacketIdentifier(NonZeroU16::new(1).unwrap()));
                let pkg = MqttPacket::Puback(MPuback {
                    packet_identifier,
                    reason: PubackReasonCode::Success,
                    properties: PubackProperties::new(),
                });
                encoder
                    .write(pkg)
                    .await
                    .map_err(|_| DistributorError::Unknown)?;
            }
            MqttPacket::Subscribe(subscribe) => {
                let result = subscribe
                    .subscriptions
                    .iter()
                    .filter_map(|s| distributor.subscribe(s.topic_filter).err())
                    .map(SubackReasonCode::from)
                    .take(8)
                    .collect::<Vec<_, 8>>();

                let pkg = MqttPacket::Suback(MSuback {
                    packet_identifier: subscribe.packet_identifier,
                    properties: SubackProperties::new(),
                    reasons: &result,
                });
                encoder
                    .write(pkg)
                    .await
                    .map_err(|_| DistributorError::Unknown)?;
            }
            MqttPacket::Unsubscribe(unsubscribe) => {
                unsubscribe
                    .unsubscriptions
                    .iter()
                    .for_each(|s| distributor.unsubscribe(s.topic_filter));
                let pkg = MqttPacket::Unsuback(MUnsuback {
                    packet_identifier: unsubscribe.packet_identifier,
                    properties: UnsubackProperties::new(),
                    reasons: &[],
                });
                encoder
                    .write(pkg)
                    .await
                    .map_err(|_| DistributorError::Unknown)?;
            }
            MqttPacket::Disconnect(_disconnect) => {
                info!("SOCKET {}: disconnecting", distributor.get_id());
                return Ok(());
            }
            MqttPacket::Pingreq(_pingreq) => {
                let pkg = MqttPacket::Pingresp(MPingresp {});
                encoder
                    .write(pkg)
                    .await
                    .map_err(|_| DistributorError::Unknown)?;
            }
            MqttPacket::Puback(_puback) => {
                // nothing to do (yet)
            }
            pkg => {
                warn!(
                    "SOCKET {}: unexpected packet {:?}",
                    distributor.get_id(),
                    pkg
                );
                return Err(DistributorError::UnexpectedPacket);
            }
        }
    }
}
