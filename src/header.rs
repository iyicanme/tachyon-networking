use crate::int_buffer::IntBuffer;

pub const MESSAGE_TYPE_UNRELIABLE: u8 = 0;
pub const MESSAGE_TYPE_RELIABLE: u8 = 1;
pub const MESSAGE_TYPE_FRAGMENT: u8 = 2;
pub const MESSAGE_TYPE_NONE: u8 = 3;
pub const MESSAGE_TYPE_NACK: u8 = 4;
pub const MESSAGE_TYPE_RELIABLE_WITH_NACK: u8 = 5;

pub const MESSAGE_TYPE_LINK_IDENTITY: u8 = 6;
pub const MESSAGE_TYPE_UNLINK_IDENTITY: u8 = 7;

pub const MESSAGE_TYPE_IDENTITY_LINKED: u8 = 8;
pub const MESSAGE_TYPE_IDENTITY_UNLINKED: u8 = 9;

pub const TACHYON_HEADER_SIZE: usize = 4;
pub const TACHYON_NACKED_HEADER_SIZE: usize = 10;
pub const TACHYON_FRAGMENTED_HEADER_SIZE: usize = 10;

#[derive(Clone, Copy)]
#[derive(Default)]
pub struct Header {
    pub message_type: u8,
    pub channel: u8,
    pub sequence: u16,

    // fragment - optional
    pub fragment_group: u16,
    pub fragment_start_sequence: u16,
    pub fragment_count: u16,

    // nacked - optional
    pub start_sequence: u16,
    pub flags: u32,
}

impl Header {
    pub fn write_unreliable(&self, buffer: &mut [u8]) {
        let mut writer = IntBuffer { index: 0 };

        writer.write_u8(self.message_type, buffer);
    }

    pub fn write_nacked(&self, buffer: &mut [u8]) {
        let mut writer = IntBuffer { index: 0 };

        writer.write_u8(self.message_type, buffer);
        writer.write_u8(self.channel, buffer);
        writer.write_u16(self.sequence, buffer);

        writer.write_u16(self.start_sequence, buffer);
        writer.write_u32(self.flags, buffer);
    }

    pub fn write(&self, buffer: &mut [u8]) {
        let mut writer = IntBuffer { index: 0 };

        writer.write_u8(self.message_type, buffer);
        writer.write_u8(self.channel, buffer);
        writer.write_u16(self.sequence, buffer);
    }

    #[must_use]
    pub fn read(buffer: &[u8]) -> Self {
        let mut reader = IntBuffer { index: 0 };

        let message_type = reader.read_u8(buffer);
        let channel = reader.read_u8(buffer);
        let sequence = reader.read_u16(buffer);

        Self {
            message_type,
            channel,
            sequence,
            ..Self::default()
        }
    }

    // fragmented
    pub fn write_fragmented(&self, buffer: &mut [u8]) {
        let mut writer = IntBuffer { index: 0 };

        writer.write_u8(self.message_type, buffer);
        writer.write_u8(self.channel, buffer);
        writer.write_u16(self.sequence, buffer);

        writer.write_u16(self.fragment_group, buffer);
        writer.write_u16(self.fragment_start_sequence, buffer);
        writer.write_u16(self.fragment_count, buffer);
    }

    #[must_use]
    pub fn read_fragmented(buffer: &[u8]) -> Self {
        let mut reader = IntBuffer { index: 0 };

        let message_type = reader.read_u8(buffer);
        let channel = reader.read_u8(buffer);
        let sequence = reader.read_u16(buffer);

        let fragment_group = reader.read_u16(buffer);
        let fragment_start_sequence = reader.read_u16(buffer);
        let fragment_count = reader.read_u16(buffer);

        Self {
            message_type,
            channel,
            sequence,
            fragment_group,
            fragment_start_sequence,
            fragment_count,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn create_fragmented(
        sequence: u16,
        channel: u8,
        group: u16,
        start: u16,
        count: u16,
    ) -> Self {
        Self {
            message_type: MESSAGE_TYPE_FRAGMENT,
            sequence,
            channel,
            fragment_group: group,
            fragment_start_sequence: start,
            fragment_count: count,
            ..Self::default()
        }
    }
}
