use crate::errors::MqttCodecError;
use embedded_io_async::{Read, Write};
use log::{error, warn};
use mqtt_format::v5::packets::MqttPacket;
use mqtt_format::v5::write::{MqttWriteError, WResult, WriteMqttPacket};
use winnow::Partial;

pub(crate) struct MqttCodec<T, const N: usize>
where
    T: Read,
{
    stream: T,
    buf: [u8; N],
    read: usize,
    write: usize,
}

pub(crate) struct MqttCodecEncoder<T, const N: usize>
where
    T: Write,
{
    stream: T,
}

impl<T, const N: usize> MqttCodec<T, N>
where
    T: Read,
{
    pub fn new(stream: T) -> MqttCodec<T, N> {
        MqttCodec {
            stream,
            buf: [0u8; N],
            read: 0,
            write: 0,
        }
    }

    async fn read_stream(&mut self) -> Result<Option<usize>, MqttCodecError> {
        let n = match self.stream.read(&mut self.buf[self.write..]).await {
            Ok(0) => {
                return Ok(None);
            }
            Ok(n) => n,
            Err(e) => {
                warn!("CODEC: {:?}", e);
                return Err(MqttCodecError::ConnectionReset);
            }
        };
        self.write += n;
        assert!(self.write <= self.buf.len());
        Ok(Some(n))
    }

    pub async fn next(&mut self) -> Result<Option<MqttPacket>, MqttCodecError> {
        // if buffer empty, reset and read from stream
        if self.read == self.write {
            self.read = 0;
            self.write = 0;
            if self.read_stream().await?.is_none() {
                return Ok(None);
            }
        }

        loop {
            let packet_len = match get_pkg_len(&self.buf[self.read..self.write]) {
                Ok(Some(len)) => len, // enough in buffer to read next packet
                Ok(None) => {
                    // receive more from socket
                    if self.read_stream().await?.is_none() {
                        return Ok(None);
                    }
                    continue;
                }
                // error parsing packet length
                Err(_) => return Err(MqttCodecError::InvalidLength),
            };

            if packet_len >= self.buf.len() {
                // todo copy stuff to location 0 to increase buffer size
                error!(
                    "packet too long! {}bytes buffer size: {}",
                    packet_len,
                    self.buf.len()
                );
                return Err(MqttCodecError::InvalidLength);
            }
            // if not enough data has been received yet
            if self.write - self.read < packet_len {
                continue;
            }

            let start = self.read;
            self.read += packet_len;
            if self.read > self.buf.len() {
                error!("read index out of bounds");
                return Err(MqttCodecError::BufferTooSmall);
            }

            let packet = MqttPacket::parse_complete(&self.buf[start..self.read]);
            if let Ok(packet) = packet {
                return Ok(Some(packet));
            }
            error!("error parsing packet {:?}", packet);
            return Err(MqttCodecError::Invalid);
        }
    }
}
impl<T, const N: usize> MqttCodecEncoder<T, N>
where
    T: Write,
{
    pub fn new(stream: T) -> MqttCodecEncoder<T, N> {
        MqttCodecEncoder { stream }
    }
    pub async fn write<'a>(&mut self, packet: MqttPacket<'a>) -> Result<(), MqttCodecError> {
        if packet.binary_size() > N as u32 {
            error!(
                "packet too large to write ({}/{} Bytes)",
                packet.binary_size(),
                N
            );
            return Err(MqttCodecError::BufferTooSmall);
        }
        // create a packet writer with the same size as the parser
        let mut writer = PacketWriter::<N>::default();

        if let Err(e) = packet.write(&mut writer) {
            error!("error writing packet {:?}", e);
            return Err(MqttCodecError::BufferTooSmall);
        }
        if let Err(e) = self.stream.write(writer.get_written_data()).await {
            warn!("codec sending to socket {:?}", e);
            return Err(MqttCodecError::ConnectionReset);
        }
        Ok(())
    }
}

pub struct PacketWriter<const N: usize> {
    pub buffer: [u8; N],
    pub write_index: usize,
}

impl<const N: usize> Default for PacketWriter<N> {
    fn default() -> Self {
        PacketWriter {
            buffer: [0; N],
            write_index: 0,
        }
    }
}
impl<const N: usize> PacketWriter<N> {
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

/// Some(usize) if enough data to read a packet
/// None if not enough data
/// Err() if data is not in the correct format
fn get_pkg_len(buf: &[u8]) -> Result<Option<usize>, ()> {
    if buf.len() < 2 {
        return Ok(None);
    }
    let remaining_length =
        match mqtt_format::v5::integers::parse_variable_u32(&mut Partial::new(&buf[1..])) {
            Ok(size) => size as usize,
            Err(winnow::error::ErrMode::Incomplete(winnow::error::Needed::Size(_needed))) => {
                return Ok(None);
            }
            Err(_) => return Err(()),
        };

    let total_packet_length = 1
        + mqtt_format::v5::integers::variable_u32_binary_size(remaining_length as u32) as usize
        + remaining_length;
    Ok(Some(total_packet_length))
}
