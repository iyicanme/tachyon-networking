use crate::int_buffer::IntBuffer;

#[derive(Clone, Copy)]
#[derive(Default)]
pub struct ConnectionHeader {
    pub message_type: u8,
    pub id: u32,
    pub session_id: u32,
}

impl ConnectionHeader {
    #[must_use]
    pub fn read(buffer: &[u8]) -> Self {
        let mut reader = IntBuffer { index: 0 };

        let message_type = reader.read_u8(buffer);
        let id = reader.read_u32(buffer);
        let session_id = reader.read_u32(buffer);

        Self {
            message_type,
            id,
            session_id,
        }
    }

    pub fn write(&self, buffer: &mut [u8]) {
        let mut writer = IntBuffer { index: 0 };

        writer.write_u8(self.message_type, buffer);
        writer.write_u32(self.id, buffer);
        writer.write_u32(self.session_id, buffer);
    }
}
