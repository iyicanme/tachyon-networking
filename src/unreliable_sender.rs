use std::net::UdpSocket;

use crate::{TachyonSendError, TachyonSendSuccess};
use crate::header::{Header, MESSAGE_TYPE_UNRELIABLE};
use crate::network_address::NetworkAddress;

const UNRELIABLE_BUFFER_LEN: usize = 1024 * 16;

// this is created with a cloned UdpSocket which can then be used from another thread.
pub struct UnreliableSender {
    pub socket: Option<UdpSocket>,
    pub send_buffer: Vec<u8>,
}

impl UnreliableSender {
    #[must_use]
    pub fn create(socket: Option<UdpSocket>) -> Self {
        Self {
            socket,
            send_buffer: vec![0; UNRELIABLE_BUFFER_LEN],
        }
    }

    pub fn send(&mut self, address: NetworkAddress, data: &[u8], body_len: usize) -> Result<TachyonSendSuccess, TachyonSendError> {
        if body_len < 1 {
            return Err(TachyonSendError::Length);
        }

        if self.socket.is_none() {
            return Err(TachyonSendError::Channel);
        }

        // copy to send buffer at +1 offset for message_type
        self.send_buffer[1..=body_len].copy_from_slice(&data[0..body_len]);
        let length = body_len + 1;

        let header = Header {
            message_type: MESSAGE_TYPE_UNRELIABLE,
            ..Header::default()
        };

        header.write_unreliable(&mut self.send_buffer);
        let sent_len = self.send_to(address, length);

        Ok(TachyonSendSuccess { sent_len: sent_len as u32, header })
    }

    fn send_to(&self, address: NetworkAddress, length: usize) -> usize {
        let Some(socket) = &self.socket else {
            return 0;
        };

        let slice = &self.send_buffer[0..length];
        let socket_result = if address.port == 0 {
            socket.send(slice)
        } else {
            socket.send_to(slice, address.to_socket_addr())
        };

        socket_result.unwrap_or_default()
    }
}
