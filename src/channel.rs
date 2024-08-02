use rustc_hash::FxHashMap;

use crate::{TachyonSendError, TachyonSendSuccess};
use crate::fragmentation::Fragmentation;
use crate::header::{
    Header, MESSAGE_TYPE_FRAGMENT, MESSAGE_TYPE_NACK, MESSAGE_TYPE_NONE, MESSAGE_TYPE_RELIABLE,
    MESSAGE_TYPE_RELIABLE_WITH_NACK, TACHYON_FRAGMENTED_HEADER_SIZE, TACHYON_HEADER_SIZE,
    TACHYON_NACKED_HEADER_SIZE,
};
use crate::int_buffer::IntBuffer;
use crate::nack::Nack;
use crate::network_address::NetworkAddress;
use crate::receiver::Receiver;
use crate::send_buffer_manager::SendBufferManager;
use crate::tachyon_socket::TachyonSocket;

pub static mut NONE_SEND_DATA: [u8; TACHYON_HEADER_SIZE] = [0; TACHYON_HEADER_SIZE];
const NACK_REDUNDANCY_DEFAULT: u32 = 1;
pub const RECEIVE_WINDOW_SIZE_DEFAULT: u32 = 512;

#[derive(Clone, Copy, Default, Debug)]
pub struct ChannelStats {
    pub sent: u64,
    pub received: u64,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub fragments_sent: u64,
    pub fragments_received: u64,
    pub fragments_assembled: u64,
    pub published: u64,
    pub published_consumed: u64,
    pub nacks_sent: u64,
    pub nacks_received: u64,
    pub resent: u64,
    pub nones_sent: u64,
    pub nones_received: u64,
    pub nones_accepted: u64,
    pub skipped_sequences: u64,
}

impl ChannelStats {
    pub fn add_from(&mut self, other: &Self) {
        self.sent += other.sent;
        self.received += other.received;
        self.bytes_sent += other.bytes_sent;
        self.bytes_received += other.bytes_received;
        self.fragments_sent += other.fragments_sent;
        self.fragments_received += other.fragments_received;
        self.fragments_assembled += other.fragments_assembled;
        self.published += other.published;
        self.published_consumed += other.published_consumed;
        self.nacks_sent += other.nacks_sent;
        self.nacks_received += other.nacks_received;
        self.resent += other.resent;
        self.nones_sent += other.nones_sent;
        self.nones_received += other.nones_received;
        self.nones_accepted += other.nones_accepted;
        self.skipped_sequences += other.skipped_sequences;
    }
}

impl std::fmt::Display for ChannelStats {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "sent:{} received:{},kb_sent:{} kb_received:{}
fragments_sent:{} fragments_received:{} fragments_assembled:{},
published: {} published_consumed:{} nacks_sent:{} nacks_received:{} resent:{}
nones_sent:{} nones_received:{} nones_accepted:{} skipped_sequences:{}\n\n",
            self.sent,
            self.received,
            self.bytes_sent / 1024,
            self.bytes_received / 1024,
            self.fragments_sent,
            self.fragments_received,
            self.fragments_assembled,
            self.published,
            self.published_consumed,
            self.nacks_sent,
            self.nacks_received,
            self.resent,
            self.nones_sent,
            self.nones_received,
            self.nones_accepted,
            self.skipped_sequences
        )
    }
}

#[derive(Clone, Copy, Default)]
pub struct ChannelConfig {
    pub receive_window_size: u32,
    pub nack_redundancy: u32,
    pub ordered: u32,
}

impl ChannelConfig {
    #[must_use]
    pub const fn default_ordered() -> Self {
        Self {
            ordered: 1,
            receive_window_size: RECEIVE_WINDOW_SIZE_DEFAULT,
            nack_redundancy: NACK_REDUNDANCY_DEFAULT,
        }
    }

    #[must_use]
    pub const fn default_unordered() -> Self {
        Self {
            ordered: 0,
            receive_window_size: RECEIVE_WINDOW_SIZE_DEFAULT,
            nack_redundancy: NACK_REDUNDANCY_DEFAULT,
        }
    }

    #[must_use]
    pub const fn is_ordered(&self) -> bool {
        self.ordered == 1
    }
}

pub struct Channel {
    pub id: u8,
    pub address: NetworkAddress,
    pub frag: Fragmentation,
    pub send_buffers: SendBufferManager,
    pub receiver: Receiver,
    pub stats: ChannelStats,
    nack_send_data: Vec<u8>,
    nacked_sequences: Vec<u16>,
    nacked_sequence_map: FxHashMap<u16, NetworkAddress>,
    pub resend_rewrite_buffer: Vec<u8>,
    pub nack_redundancy: u32,
}

impl Channel {
    #[must_use]
    pub fn create(id: u8, address: NetworkAddress, config: ChannelConfig) -> Self {
        Self {
            id,
            address,
            frag: Fragmentation::default(),
            send_buffers: SendBufferManager::create(),
            receiver: Receiver::create(config.is_ordered(), config.receive_window_size),
            stats: ChannelStats::default(),
            nack_send_data: vec![0; 512],
            nacked_sequences: Vec::new(),
            nacked_sequence_map: FxHashMap::default(),
            resend_rewrite_buffer: vec![0; 2048],
            nack_redundancy: config.nack_redundancy,
        }
    }

    fn create_none(sequence: u16, channel_id: u8) {
        let header = Header {
            message_type: MESSAGE_TYPE_NONE,
            sequence,
            channel: channel_id,
            ..Header::default()
        };

        header.write(unsafe { &mut NONE_SEND_DATA });
    }

    #[must_use]
    pub const fn is_ordered(&self) -> bool {
        self.receiver.is_ordered
    }

    pub fn update_stats(&mut self) {
        self.stats.skipped_sequences = self.receiver.skipped_sequences;
    }

    pub fn receive_published(&mut self, receive_buffer: &mut [u8]) -> (u32, NetworkAddress) {
        for _ in 0..1000 {
            let (length, address, should_retry) = self.receive_published_internal(receive_buffer);
            
            if length > 0 {
                return (length, address);
            }
            
            if !should_retry {
                break;
            }
        }

        (0, self.address)
    }

    // returns message length, address, should retry (queue not empty)
    fn receive_published_internal(
        &mut self,
        receive_buffer: &mut [u8],
    ) -> (u32, NetworkAddress, bool) {
        let Some(byte_buffer) = self.receiver.take_published() else {
            return (0, self.address, false);
        };

        let buffer_len = byte_buffer.length;

        let mut reader = IntBuffer { index: 0 };
        let message_type = reader.read_u8(byte_buffer.get());

        if message_type == MESSAGE_TYPE_NONE {
            self.receiver.return_buffer(byte_buffer);
            return (0, self.address, true);
        }

        if message_type == MESSAGE_TYPE_FRAGMENT {
            let header = Header::read_fragmented(byte_buffer.get());
            let Ok(res) = self.frag.assemble(header) else {
                self.receiver.return_buffer(byte_buffer);
                return (0, self.address, true);
            };

            let assembled_len = res.len();
            receive_buffer[0..assembled_len].copy_from_slice(&res[..]);
            self.stats.received += 1;
            self.stats.fragments_assembled += header.fragment_count as u64;
            self.stats.published_consumed += 1;

            return (assembled_len as u32, self.address, true);
        }

        let header_size: usize;
        if message_type == MESSAGE_TYPE_RELIABLE_WITH_NACK {
            header_size = TACHYON_NACKED_HEADER_SIZE;
        } else if message_type == MESSAGE_TYPE_RELIABLE {
            header_size = TACHYON_HEADER_SIZE;
        } else {
            // should not be possible
            return (0, self.address, true);
        }

        receive_buffer[0..buffer_len - header_size].copy_from_slice(&byte_buffer.get()[header_size..buffer_len]);
        self.receiver.return_buffer(byte_buffer);

        self.stats.published_consumed += 1;
        ((buffer_len - header_size) as u32, self.address, true)
    }

    pub fn process_none_message(
        &mut self,
        sequence: u16,
        receive_buffer: &[u8],
        received_len: usize,
    ) {
        self.stats.nones_received += 1;
        if self
            .receiver
            .receive_packet(sequence, receive_buffer, received_len)
        {
            self.stats.nones_accepted += 1;
        }
    }

    pub fn process_fragment_message(
        &mut self,
        sequence: u16,
        receive_buffer: &[u8],
        received_len: usize,
    ) {
        let fragmented = self.frag.receive_fragment(receive_buffer, received_len).0;
        if fragmented && self.receiver.receive_packet(sequence, receive_buffer, TACHYON_FRAGMENTED_HEADER_SIZE) {
            self.stats.fragments_received += 1;
        }
    }

    // separate nack message, varint encoded
    pub fn process_nack_message(&mut self, address: NetworkAddress, receive_buffer: &[u8]) {
        self.nacked_sequences.clear();
        Nack::read_varint(
            &mut self.nacked_sequences,
            receive_buffer,
            TACHYON_HEADER_SIZE,
        );
        self.copy_nacked_to_map(address);
    }

    // nack that is in a reliable message
    pub fn process_single_nack(&mut self, address: NetworkAddress, receive_buffer: &[u8]) {
        self.nacked_sequences.clear();
        Nack::read_single(
            &mut self.nacked_sequences,
            receive_buffer,
            TACHYON_HEADER_SIZE,
        );
        self.copy_nacked_to_map(address);
    }

    pub fn send_reliable(
        &mut self,
        address: NetworkAddress,
        data: &[u8],
        body_len: usize,
        socket: &TachyonSocket,
    ) -> Result<TachyonSendSuccess, TachyonSendError> {
        // Optionally include nacks in outgoing messages, up to nack_redundancy times for each nack
        let mut nack_option: Option<Nack> = None;
        let mut header_len = TACHYON_HEADER_SIZE;

        if self.nack_redundancy > 0 {
            if let Some(mut nack) = self.receiver.nack_queue.pop_front() {
                if nack.sent_count < self.nack_redundancy {
                    nack.sent_count += 1;
                    nack_option = Some(nack);
                    header_len = TACHYON_NACKED_HEADER_SIZE;
                }
                self.receiver.nack_queue.push_back(nack);
            }
        }

        let send_buffer_len = body_len + header_len;

        let Some(send_buffer) = self.send_buffers.create_send_buffer(send_buffer_len) else {
            return Err(TachyonSendError::Unknown);
        };

        let sequence = send_buffer.sequence;
        send_buffer.byte_buffer.get_mut()[header_len..body_len + header_len].copy_from_slice(&data[0..body_len]);

        let mut header = Header {
            channel: self.id,
            sequence,
            ..Header::default()
        };

        if let Some(nack) = nack_option {
            header.message_type = MESSAGE_TYPE_RELIABLE_WITH_NACK;
            header.start_sequence = nack.start_sequence;
            header.flags = nack.flags;

            self.stats.nacks_sent += nack.nacked_count as u64;
        } else {
            header.message_type = MESSAGE_TYPE_RELIABLE;
        }

        header.write(send_buffer.byte_buffer.get_mut());

        let sent_len = socket.send_to(address, send_buffer.byte_buffer.get(), send_buffer_len).unwrap_or_default();
        self.stats.bytes_sent += sent_len as u64;
        self.stats.sent += 1;

        Ok(TachyonSendSuccess { sent_len: sent_len as u32, header })
    }

    pub fn update(&mut self, socket: &TachyonSocket) {
        self.send_nacks(socket);
        self.resend_nacked(socket);

        // this takes way too long if there are a lot of frag groups, disabling until I find a better solution
        //self.frag.expire_groups();

        self.receiver.publish();
    }

    fn copy_nacked_to_map(&mut self, address: NetworkAddress) {
        for sequence in &self.nacked_sequences {
            self.nacked_sequence_map.insert(*sequence, address);
        }
    }

    // Resend messages for nacks sent to us. We accumulate these into a hashmap of unique sequence/address pairs
    // and then do the resends all at once when update() is run.
    fn resend_nacked(&mut self, socket: &TachyonSocket) {
        if self.nacked_sequence_map.is_empty() {
            return;
        }

        for (sequence, address) in &self.nacked_sequence_map {
            self.stats.nacks_received += 1;
            if let Some(send_buffer) = self.send_buffers.get_send_buffer(*sequence) {
                let mut reader = IntBuffer { index: 0 };

                let message_type = reader.read_u8(send_buffer.byte_buffer.get());
                let _ = if message_type == MESSAGE_TYPE_RELIABLE_WITH_NACK {
                    let send_len = Self::rewrite_reliable_nack_to_reliable(&mut self.resend_rewrite_buffer, send_buffer.byte_buffer.get());
                    socket.send_to(*address, &self.resend_rewrite_buffer, send_len)
                } else {
                    socket.send_to(*address, send_buffer.byte_buffer.get(), send_buffer.byte_buffer.length)
                };

                self.stats.resent += 1;
            } else {
                Self::create_none(*sequence, self.id);
                let _sent_len = socket.send_to(*address, unsafe { &NONE_SEND_DATA }, TACHYON_HEADER_SIZE);
                self.stats.nones_sent += 1;
            }
        }

        self.nacked_sequence_map.clear();
    }

    // Send nacks for sequences we are missing
    fn send_nacks(&mut self, socket: &TachyonSocket) {
        let nack_count = self.receiver.create_nacks();
        if self.receiver.nack_list.is_empty() {
            return;
        }

        let header = Header {
            message_type: MESSAGE_TYPE_NACK,
            channel: self.id,
            ..Header::default()
        };

        header.write(&mut self.nack_send_data);

        let position = Nack::write_varint(
            &self.receiver.nack_list,
            &mut self.nack_send_data,
            TACHYON_HEADER_SIZE as u64,
        );

        let _ = socket.send_to(self.address, &self.nack_send_data, position as usize);

        self.stats.nacks_sent += nack_count as u64;
    }

    pub fn rewrite_reliable_nack_to_reliable(
        rewrite_buffer: &mut [u8],
        send_buffer: &[u8],
    ) -> usize {
        let mut header = Header::read(send_buffer);
        let src_body = TACHYON_NACKED_HEADER_SIZE..send_buffer.len();
        let src_body_len = src_body.len();
        let dest = TACHYON_HEADER_SIZE..(TACHYON_HEADER_SIZE + src_body_len);
        rewrite_buffer[dest].copy_from_slice(&send_buffer[src_body]);

        header.message_type = MESSAGE_TYPE_RELIABLE;
        header.write(rewrite_buffer);

        TACHYON_HEADER_SIZE + src_body_len
    }
}

#[cfg(test)]
mod tests {
    use crate::channel::{Channel, ChannelConfig};
    use crate::header::{Header, MESSAGE_TYPE_RELIABLE, MESSAGE_TYPE_RELIABLE_WITH_NACK};
    use crate::network_address::NetworkAddress;

    #[test]
    fn test_rewrite_nack_to_reliable() {
        let mut channel = Channel::create(
            1,
            NetworkAddress::default(),
            ChannelConfig::default_ordered(),
        );
        let mut send_buffer: Vec<u8> = vec![0; 1200];
        let header = Header {
            message_type: MESSAGE_TYPE_RELIABLE_WITH_NACK,
            channel: 13,
            sequence: 200,
            start_sequence: 12345,
            flags: 99,
            ..Header::default()
        };

        header.write(&mut send_buffer);

        send_buffer[10] = 3;
        send_buffer[1199] = 7;
        let send_len = Channel::rewrite_reliable_nack_to_reliable(
            &mut channel.resend_rewrite_buffer,
            &send_buffer,
        );

        assert_eq!(1200 - 6, send_len);
        assert_eq!(3, channel.resend_rewrite_buffer[4]);
        assert_eq!(7, channel.resend_rewrite_buffer[1199 - 6]);

        let header = Header::read(&channel.resend_rewrite_buffer);
        assert_eq!(MESSAGE_TYPE_RELIABLE, header.message_type);
        assert_eq!(200, header.sequence);
        assert_eq!(13, header.channel);

        assert_eq!(0, header.start_sequence);
        assert_eq!(0, header.flags);
    }
}
