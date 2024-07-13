use core::num::NonZeroU16;
use embassy_futures::select::select;
use embassy_futures::select::Either::{First, Second};
use embassy_net::tcp::TcpSocket;
use embassy_net::Stack;
use embassy_net_driver::Driver;
use embassy_time::Duration;
use embedded_io_async::{Read, Write};
use log::{debug, info, warn};
use mqtt_format::v5::packets::connack::{ConnackProperties, ConnackReasonCode, MConnack};
use mqtt_format::v5::packets::disconnect::{
    DisconnectProperties, DisconnectReasonCode, MDisconnect,
};
use mqtt_format::v5::packets::pingresp::MPingresp;
use mqtt_format::v5::packets::puback::{MPuback, PubackProperties, PubackReasonCode};
use mqtt_format::v5::packets::publish::{MPublish, PublishProperties};
use mqtt_format::v5::packets::suback::{MSuback, SubackProperties};
use mqtt_format::v5::packets::MqttPacket;
use mqtt_format::v5::qos::QualityOfService;
use mqtt_format::v5::variable_header::PacketIdentifier;

use crate::codec::MqttCodec;
use crate::distributor::{Distributor, InnerDistributorMutex};
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
            "SOCKET {}: Received connection from {:?}",
            id,
            socket.remote_endpoint()
        );
        // connection handler
        let mut parser = MqttCodec::<_, 1024>::new(socket);

        match parser.next().await {
            Ok(Some(MqttPacket::Connect(_connect))) => {
                let pkg = MqttPacket::Connack(MConnack {
                    session_present: false,
                    reason_code: ConnackReasonCode::Success,
                    properties: ConnackProperties::new(),
                });
                if let Err(e) = parser.write(pkg).await {
                    warn!("SOCKET {}: {:?}", id, e);
                    continue;
                }
            }
            e => {
                warn!("SOCKET {}: error decoding packet {:?}", id, e);
                continue;
            }
        }

        if let Err(error) = handle_socket(&mut parser, &distributor).await {
            warn!("SOCKET {}: {:?}", id, error);
            let error = MqttPacket::Disconnect(MDisconnect {
                reason_code: error.into(),
                properties: DisconnectProperties::new(),
            });
            if let Err(e) = parser.write(error).await {
                warn!(
                    "SOCKET {}: could not close connection because of {:?}",
                    id, e
                );
            }
            continue;
        }
    }
}

async fn handle_socket<'a, T, const CODEC_SIZE: usize, const CONNECTIONS: usize>(
    parser: &mut MqttCodec<T, CODEC_SIZE>,
    distributor: &Distributor<CONNECTIONS>,
) -> Result<(), DistributorError>
where
    T: Write + Read,
{
    loop {
        let packet = match select(distributor.next(), parser.next()).await {
            First(msg) => {
                let packet = MqttPacket::Publish(MPublish {
                    duplicate: false,
                    quality_of_service: QualityOfService::AtMostOnce,
                    retain: false,
                    topic_name: msg.topic(),
                    packet_identifier: None,
                    properties: PublishProperties::new(),
                    payload: msg.message(),
                });
                let start = embassy_time::Instant::now();
                parser
                    .write(packet)
                    .await
                    .map_err(|_| DistributorError::Unknown)?;
                info!("SOCKET: publish took {}", start.elapsed().as_micros());
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
                distributor
                    .publish(publish.topic_name, publish.payload)
                    .await?;
                let packet_identifier = publish
                    .packet_identifier
                    .unwrap_or(PacketIdentifier(NonZeroU16::new(1).unwrap()));
                let pkg = MqttPacket::Puback(MPuback {
                    packet_identifier,
                    reason: PubackReasonCode::Success,
                    properties: PubackProperties::new(),
                });
                parser
                    .write(pkg)
                    .await
                    .map_err(|_| DistributorError::Unknown)?;
            }
            MqttPacket::Subscribe(subscribe) => {
                let mut reason = Ok(());
                for s in subscribe.subscriptions.iter() {
                    debug!("subscribing to: {}", s.topic_filter);
                    if let Err(e) = distributor.subscribe(s.topic_filter).await {
                        reason = Err(e);
                    }
                }
                let pkg = MqttPacket::Suback(MSuback {
                    packet_identifier: subscribe.packet_identifier,
                    properties: SubackProperties::new(),
                    reasons: &[],
                });
                parser
                    .write(pkg)
                    .await
                    .map_err(|_| DistributorError::Unknown)?;
            }
            MqttPacket::Disconnect(_disconnect) => {
                info!("SOCKET: disconnecting");
                return Ok(());
            }
            MqttPacket::Pingreq(_pingreq) => {
                let pkg = MqttPacket::Pingresp(MPingresp {});
                parser
                    .write(pkg)
                    .await
                    .map_err(|_| DistributorError::Unknown)?;
            }
            pkg => {
                warn!("SOCKET: unexpected packet {:?}", pkg);
                return Err(DistributorError::UnexpectedPacket);
            }
        }
    }
}
