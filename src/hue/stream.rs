use byteorder::{BigEndian, WriteBytesExt};

pub const HUE_STREAM_HEADER: &[u8; 9] = b"HueStream";

pub struct HueStreamPacketBuilder {
    sequence_number: u8,
}

impl HueStreamPacketBuilder {
    pub fn new() -> Self {
        Self { sequence_number: 0 }
    }

    /// Builds a binary HueStream packet for RGB channel updates
    pub fn build_packet(&mut self, channels: &[(u8, (u16, u16, u16))]) -> Vec<u8> {
        let mut buffer = Vec::with_capacity(16 + channels.len() * 7);

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

        // Color Mode: 0x00 = RGB (1 byte)
        buffer.write_u8(0x00).unwrap();

        // Reserved: 1 byte
        buffer.write_u8(0x00).unwrap();

        // Channel updates
        for (channel_id, (r, g, b)) in channels {
            buffer.write_u8(*channel_id).unwrap();
            buffer.write_u16::<BigEndian>(*r).unwrap();
            buffer.write_u16::<BigEndian>(*g).unwrap();
            buffer.write_u16::<BigEndian>(*b).unwrap();
        }

        buffer
    }
}
