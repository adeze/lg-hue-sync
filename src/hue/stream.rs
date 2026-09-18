use byteorder::{BigEndian, WriteBytesExt};

pub const HUE_STREAM_HEADER: &[u8; 9] = b"HueStream";

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HueColorSpace {
    Rgb = 0x00,
    XyBrightness = 0x01,
}

pub struct HueStreamPacketBuilder {
    sequence_number: u8,
    configuration_id: Option<String>,
}

impl HueStreamPacketBuilder {
    pub fn new(configuration_id: Option<String>) -> Self {
        Self {
            sequence_number: 0,
            configuration_id: configuration_id.map(|s| s.to_ascii_lowercase()),
        }
    }

    /// Builds a binary HueStream packet for RGB channel updates
    pub fn build_rgb_packet(&mut self, channels: &[(u8, (u16, u16, u16))]) -> Vec<u8> {
        self.build_packet(HueColorSpace::Rgb, channels)
    }

    /// Builds a binary HueStream packet for CIE 1931 xy chromaticity + brightness channels
    pub fn build_xy_packet(&mut self, channels: &[(u8, (u16, u16, u16))]) -> Vec<u8> {
        self.build_packet(HueColorSpace::XyBrightness, channels)
    }

    fn build_packet(
        &mut self,
        color_space: HueColorSpace,
        channels: &[(u8, (u16, u16, u16))],
    ) -> Vec<u8> {
        let has_config_id = self.configuration_id.is_some();
        let header_len = if has_config_id { 16 + 36 } else { 16 };
        let mut buffer = Vec::with_capacity(header_len + channels.len() * 7);

        // Protocol Name: "HueStream" (9 bytes)
        buffer.extend_from_slice(HUE_STREAM_HEADER);

        // Version: 2.0 (2 bytes)
        buffer.write_u8(0x02).unwrap();
        buffer.write_u8(0x00).unwrap();

        // Sequence ID: 1 byte (increments per frame)
        buffer.write_u8(self.sequence_number).unwrap();
        self.sequence_number = self.sequence_number.wrapping_add(1);

        // Reserved: 2 bytes
        buffer.write_u8(0x00).unwrap();
        buffer.write_u8(0x00).unwrap();

        // Color Mode: 0x00 = RGB, 0x01 = XY Brightness
        buffer.write_u8(color_space as u8).unwrap();

        // Reserved: 1 byte
        buffer.write_u8(0x00).unwrap();

        // If CLIP v2 UUID is present, append 36-byte lowercase UUID string
        if let Some(ref config_id) = self.configuration_id {
            if config_id.len() == 36 {
                buffer.extend_from_slice(config_id.as_bytes());
            }
        }

        // Channel updates
        for (channel_id, (c1, c2, c3)) in channels {
            buffer.write_u8(*channel_id).unwrap();
            buffer.write_u16::<BigEndian>(*c1).unwrap();
            buffer.write_u16::<BigEndian>(*c2).unwrap();
            buffer.write_u16::<BigEndian>(*c3).unwrap();
        }

        buffer
    }
}
