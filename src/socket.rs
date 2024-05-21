use crate::MAX_CONNECTIONS;
use core::num::NonZeroU16;
use embassy_net::tcp::TcpSocket;
use embassy_net::Stack;
use embassy_time::Duration;
use embedded_io_async::{Read, Write};
use esp_println::println;
use esp_wifi::wifi::{WifiDevice, WifiStaDevice};
use log::{error, info, warn};
use mqtt_format::v5::packets::connack::{ConnackProperties, ConnackReasonCode, MConnack};
use mqtt_format::v5::packets::puback::{MPuback, PubackProperties, PubackReasonCode};
use mqtt_format::v5::packets::suback::{MSuback, SubackProperties};
use mqtt_format::v5::packets::MqttPacket;
use mqtt_format::v5::variable_header::PacketIdentifier;
use mqtt_format::v5::write::{MqttWriteError, WResult, WriteMqttPacket};
use winnow::Partial;

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
        let mut parser = Parser::<TcpSocket, 1024>::new(socket);
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
                parser.stream.close();
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
                        core::str::from_utf8(publish.payload).unwrap_or_else(|_| "<invalid utf8>")
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
        parser.stream.close();
    }
}

fn get_pkg_len(buf: &[u8]) -> Result<Option<usize>, ()> {
    if buf.len() < 2 {
        return Ok(None);
    }
    let remaining_length =
        match mqtt_format::v5::integers::parse_variable_u32(&mut Partial::new(&buf[1..])) {
            Ok(size) => size as usize,
            Err(winnow::error::ErrMode::Incomplete(winnow::error::Needed::Size(needed))) => {
                println!("incomplete need {}", needed);
                return Ok(None);
            }
            Err(_) => return Err(()),
        };

    let total_packet_length = 1
        + mqtt_format::v5::integers::variable_u32_binary_size(remaining_length as u32) as usize
        + remaining_length;
    Ok(Some(total_packet_length))
}

struct Parser<T, const N: usize>
where
    T: Read + Write,
{
    stream: T,
    buf: [u8; N],
    read: usize,
    write: usize,
}

impl<T, const N: usize> Parser<T, N>
where
    T: Read + Write,
{
    fn new(socket: T) -> Parser<T, N> {
        Parser {
            stream: socket,
            buf: [0u8; N],
            read: 0,
            write: 0,
        }
    }

    async fn read_stream(&mut self) -> Result<usize, ()> {
        let n = match self.stream.read(&mut self.buf[self.write..]).await {
            Ok(0) => {
                info!("read EOF");
                return Err(());
            }
            Ok(n) => n,
            Err(e) => {
                warn!("SOCKET: {:?}", e);
                return Err(());
            }
        };
        println!("rcv: {n}");
        self.write += n; // todo wrapping
        println!("read: {} write {}", self.read, self.write);
        Ok(n)
    }

    async fn next(&mut self) -> Result<Option<MqttPacket>, ()> {
        // if buffer empty, reset and read from stream
        if self.read == self.write {
            self.read = 0;
            self.write = 0;
            self.read_stream().await?;
        }
        loop {
            let packet_len = match get_pkg_len(&self.buf[self.read..self.write]) {
                Ok(Some(len)) => len, // enough in buffer to read next packet
                Ok(None) => {
                    println!(
                        "continuing due to incomplete package length {}, {}",
                        self.read, self.write
                    );
                    // receive more from socket
                    self.read_stream().await?;
                    continue;
                }
                Err(_) => return Err(()),
            };
            println!("necessary_len: {}", packet_len);
            if packet_len >= self.buf.len() {
                // todo copy stuff to location 0 to increase buffer size
                error!(
                    "packet too long! {}bytes buffer size: {}",
                    packet_len,
                    self.buf.len()
                );
                return Err(());
            }
            if self.write - self.read < packet_len {
                continue;
            }
            let start = self.read;
            self.read += packet_len; // todo add wrapping
            println!("length: {}", self.buf[start..self.read].len());
            println!("parsing: {:?}", &self.buf[start..self.read]);
            let packet = MqttPacket::parse_complete(&self.buf[start..self.read]);
            if let Ok(packet) = packet {
                return Ok(Some(packet));
            }
            error!("error parsing packet {:?}", packet);
            return Err(());
        }
    }

    async fn write<'a>(&mut self, packet: MqttPacket<'a>) -> Result<(), ()> {
        // create a packet writer with the same size as the parser
        let mut writer = PacketWriter::<N>::new();
        packet.write(&mut writer).map_err(|_| ())?;
        self.stream
            .write(&writer.get_written_data())
            .await
            .map_err(|_| ())?;
        Ok(())
    }
}

#[derive(Debug)]
pub struct PacketWriter<const N: usize> {
    pub buffer: [u8; N],
    pub write_index: usize,
}

impl<const N: usize> PacketWriter<N> {
    pub fn new() -> Self {
        PacketWriter {
            buffer: [0; N],
            write_index: 0,
        }
    }
    pub fn get_written_data(&self) -> &[u8] {
        &self.buffer[..self.write_index]
    }
}
impl<const N: usize> WriteMqttPacket for PacketWriter<N> {
    type Error = MqttWriteError;

    #[inline]
    fn write_byte(&mut self, u: u8) -> WResult<Self> {
        self.buffer[self.write_index] = u;
        self.write_index += 1;
        Ok(())
    }

    fn write_slice(&mut self, u: &[u8]) -> WResult<Self> {
        self.buffer[self.write_index..self.write_index + u.len()].copy_from_slice(u);
        self.write_index += u.len();
        Ok(())
    }
}
