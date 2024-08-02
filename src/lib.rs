use std::time::{Duration, Instant};

use rustc_hash::FxHashMap;

use crate::channel::{Channel, ChannelConfig, ChannelStats};
use crate::connection::{Connection, Identity};
use crate::connection_header::ConnectionHeader;
use crate::connection_impl::{
    ConnectionEventCallback, IdentityEventCallback, IDENTITY_LINKED_EVENT, IDENTITY_UNLINKED_EVENT,
    LINK_IDENTITY_EVENT, UNLINK_IDENTITY_EVENT,
};
use crate::fragmentation::Fragmentation;
use crate::header::{
    Header, MESSAGE_TYPE_FRAGMENT, MESSAGE_TYPE_IDENTITY_LINKED, MESSAGE_TYPE_IDENTITY_UNLINKED,
    MESSAGE_TYPE_LINK_IDENTITY, MESSAGE_TYPE_NACK, MESSAGE_TYPE_NONE, MESSAGE_TYPE_RELIABLE,
    MESSAGE_TYPE_RELIABLE_WITH_NACK, MESSAGE_TYPE_UNLINK_IDENTITY, MESSAGE_TYPE_UNRELIABLE,
};
use crate::network_address::NetworkAddress;
use crate::pool::SendTarget;
use crate::receive_result::{ReceiveError, ReceiveSuccess, ReceiveType};
use crate::tachyon_receive_result::{
    TachyonReceiveError, TachyonReceiveSuccess, TachyonReceiveType,
};
use crate::tachyon_socket::{SocketReceiveError, SocketReceiveSuccess, TachyonSocket};
use crate::unreliable_sender::UnreliableSender;

pub mod byte_buffer_pool;
pub mod channel;
pub mod connection;
pub mod fragmentation;
pub mod header;
pub mod int_buffer;
pub mod nack;
pub mod network_address;
pub mod pool;
pub mod receive_result;
pub mod receiver;
pub mod send_buffer_manager;
pub mod sequence;
pub mod sequence_buffer;
pub mod tachyon_socket;
pub mod unreliable_sender;

mod connection_impl;

// additional stress/scale testing
mod connection_header;
mod tachyon_receive_result;
#[cfg(test)]
pub mod tachyon_test;

const SOCKET_RECEIVE_BUFFER_LEN: usize = 1024 * 1024;

#[derive(Clone, Copy, Default, Debug)]
pub struct TachyonStats {
    pub channel_stats: ChannelStats,
    pub packets_dropped: u64,
    pub unreliable_sent: u64,
    pub unreliable_received: u64,
}

impl std::fmt::Display for TachyonStats {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        writeln!(
            f,
            "channel_stats:{0} packets_dropped:{1} unreliable_sent:{2} unreliable_received:{3}",
            self.channel_stats,
            self.packets_dropped,
            self.unreliable_sent,
            self.unreliable_received
        )
    }
}

#[derive(Default, Clone, Copy)]
pub struct TachyonConfig {
    pub use_identity: u32,
    pub drop_packet_chance: u64,
    pub drop_reliable_only: u32,
}

#[derive(Clone, Copy, Default, Debug)]
pub struct TachyonSendSuccess {
    pub sent_len: u32,
    pub header: Header,
}

#[derive(Eq, PartialEq, Debug)]
pub enum TachyonSendError {
    Socket,
    Channel,
    Fragment,
    Unknown,
    Length,
    Identity,
}

pub struct Tachyon {
    pub id: u16,
    pub socket: TachyonSocket,
    pub socket_receive_buffer: Vec<u8>,
    pub unreliable_sender: Option<UnreliableSender>,
    pub identities: FxHashMap<u32, u32>,
    pub connections: FxHashMap<NetworkAddress, Connection>,
    pub identity_to_address_map: FxHashMap<u32, NetworkAddress>,
    pub channels: FxHashMap<(NetworkAddress, u8), Channel>,
    pub channel_config: FxHashMap<u8, ChannelConfig>,
    pub config: TachyonConfig,
    pub nack_send_data: Vec<u8>,
    pub stats: TachyonStats,
    pub start_time: Instant,
    pub last_identity_link_request: Instant,
    pub identity: Identity,
    pub identity_event_callback: Option<IdentityEventCallback>,
    pub connection_event_callback: Option<ConnectionEventCallback>,
}

impl Tachyon {
    #[must_use]
    pub fn create(config: TachyonConfig) -> Self {
        Self::create_with_id(config, 0)
    }

    #[must_use]
    pub fn create_with_id(config: TachyonConfig, id: u16) -> Self {
        let mut channel_config = FxHashMap::default();
        channel_config.insert(1, ChannelConfig::default_ordered());
        channel_config.insert(2, ChannelConfig::default_unordered());

        Self {
            id,
            identities: FxHashMap::default(),
            connections: FxHashMap::default(),
            identity_to_address_map: FxHashMap::default(),
            channels: FxHashMap::default(),
            channel_config,
            socket: TachyonSocket::create(),
            socket_receive_buffer: vec![0; SOCKET_RECEIVE_BUFFER_LEN],
            unreliable_sender: None,
            config,
            nack_send_data: vec![0; 4096],
            stats: TachyonStats::default(),
            start_time: Instant::now(),
            last_identity_link_request: Instant::now()
                .checked_sub(Duration::from_secs(100))
                .unwrap_or_else(Instant::now),
            identity: Identity::default(),
            identity_event_callback: None,
            connection_event_callback: None,
        }
    }

    #[must_use]
    pub fn time_since_start(&self) -> u64 {
        Instant::now().duration_since(self.start_time).as_millis() as u64
    }

    pub fn bind(&mut self, address: NetworkAddress) -> bool {
        let bound = self.socket.bind_socket(address).is_ok();
        if bound {
            self.unreliable_sender = self.create_unreliable_sender();
        }

        bound
    }

    pub fn connect(&mut self, address: NetworkAddress) -> bool {
        let connected = self.socket.connect_socket(address).is_ok();
        if connected {
            let local_address = NetworkAddress::default();
            self.create_connection(local_address, Identity::default());
            self.unreliable_sender = self.create_unreliable_sender();
        }

        connected
    }

    #[must_use]
    pub fn create_unreliable_sender(&self) -> Option<UnreliableSender> {
        let socket = self.socket.clone_socket();
        let sender = UnreliableSender::create(socket);
        Some(sender)
    }

    pub fn get_channel(&mut self, address: NetworkAddress, channel_id: u8) -> Option<&mut Channel> {
        self.channels.get_mut(&(address, channel_id))
    }

    fn create_configured_channels(&mut self, address: NetworkAddress) {
        for (channel_id, config) in &self.channel_config {
            if self.channels.get_mut(&(address, *channel_id)).is_none() {
                let channel = Channel::create(*channel_id, address, *config);
                self.channels.insert((address, *channel_id), channel);
            }
        }
    }

    #[must_use]
    pub fn get_channel_count(&self, address: NetworkAddress) -> u32 {
        self.channel_config
            .keys()
            .filter(|channel_id| self.channels.contains_key(&(address, **channel_id)))
            .count() as u32
    }

    fn remove_configured_channels(&mut self, address: NetworkAddress) {
        for config in &self.channel_config {
            let channel_id = *config.0;
            self.channels.remove(&(address, channel_id));
        }
    }

    pub fn configure_channel(&mut self, channel_id: u8, config: ChannelConfig) -> bool {
        if channel_id < 3 {
            return false;
        }

        self.channel_config.insert(channel_id, config);

        true
    }

    pub fn get_combined_stats(&mut self) -> TachyonStats {
        let mut channel_stats = ChannelStats::default();
        for channel in self.channels.values_mut() {
            channel.update_stats();
            channel_stats.add_from(&channel.stats);
        }

        let mut stats = self.stats;
        stats.channel_stats = channel_stats;

        stats
    }

    pub fn update(&mut self) {
        self.client_identity_update();

        for channel in self.channels.values_mut() {
            channel.update(&self.socket);
        }
    }

    fn receive_published_channel_id(
        &mut self,
        receive_buffer: &mut [u8],
        address: NetworkAddress,
        channel_id: u8,
    ) -> u32 {
        self.channels
            .get_mut(&(address, channel_id))
            .map(|c| c.receive_published(receive_buffer).0)
            .unwrap_or_default()
    }

    fn receive_published_all_channels(
        &mut self,
        receive_buffer: &mut [u8],
    ) -> Result<TachyonReceiveSuccess, TachyonReceiveError> {
        for channel in self.channels.values_mut() {
            let (length, address) = channel.receive_published(receive_buffer);
            if length > 0 {
                return if channel.id == 0 {
                    Ok(TachyonReceiveSuccess {
                        length,
                        address,
                        receive_type: TachyonReceiveType::Unreliable,
                    })
                } else {
                    Ok(TachyonReceiveSuccess {
                        length,
                        address,
                        receive_type: TachyonReceiveType::Reliable {
                            channel: channel.id as u16,
                        },
                    })
                };
            }
        }

        Ok(TachyonReceiveSuccess {
            length: 0,
            address: NetworkAddress::default(),
            receive_type: TachyonReceiveType::Unreliable,
        })
    }

    pub fn receive_loop(
        &mut self,
        receive_buffer: &mut [u8],
    ) -> Result<TachyonReceiveSuccess, TachyonReceiveError> {
        for _ in 0..100 {
            let receive_result = self.receive_from_socket();
            match receive_result {
                Ok(ReceiveSuccess {
                    receive_type: ReceiveType::Reliable { channel_id },
                    network_address,
                }) => {
                    let published = self.receive_published_channel_id(
                        receive_buffer,
                        network_address,
                        channel_id,
                    );
                    if published > 0 {
                        return Ok(TachyonReceiveSuccess {
                            receive_type: TachyonReceiveType::Reliable {
                                channel: channel_id as u16,
                            },
                            length: published,
                            address: network_address,
                        });
                    }
                }
                Ok(ReceiveSuccess {
                    receive_type: ReceiveType::Unreliable { received_len },
                    network_address,
                }) => {
                    receive_buffer[0..received_len - 1]
                        .copy_from_slice(&self.socket_receive_buffer[1..received_len]);
                    return Ok(TachyonReceiveSuccess {
                        receive_type: TachyonReceiveType::Unreliable,
                        length: (received_len - 1) as u32,
                        address: network_address,
                    });
                }
                Err(ReceiveError::Empty) => {
                    break;
                }
                Err(ReceiveError::Retry) => {}
                Err(ReceiveError::Error) => {
                    return Err(TachyonReceiveError::Unknown);
                }
                Err(ReceiveError::ChannelError) => {
                    return Err(TachyonReceiveError::Channel);
                }
            }
        }

        self.receive_published_all_channels(receive_buffer)
    }

    fn receive_from_socket(&mut self) -> Result<ReceiveSuccess, ReceiveError> {
        let address: NetworkAddress;
        let received_len: usize;
        let header: Header;

        let socket_result = self.socket.receive(
            &mut self.socket_receive_buffer,
            self.config.drop_packet_chance,
            self.config.drop_reliable_only == 1,
        );
        match socket_result {
            Ok(SocketReceiveSuccess {
                bytes_received,
                network_address,
            }) => {
                received_len = bytes_received;
                address = network_address;

                header = Header::read(&self.socket_receive_buffer);

                if self.socket.is_server {
                    if self.config.use_identity == 1 {
                        let connection_header: ConnectionHeader;

                        if header.message_type == MESSAGE_TYPE_LINK_IDENTITY {
                            connection_header = ConnectionHeader::read(&self.socket_receive_buffer);
                            if self.try_link_identity(
                                address,
                                connection_header.id,
                                connection_header.session_id,
                            ) {
                                self.fire_identity_event(
                                    LINK_IDENTITY_EVENT,
                                    address,
                                    connection_header.id,
                                    connection_header.session_id,
                                );
                            }
                            return Err(ReceiveError::Retry);
                        } else if header.message_type == MESSAGE_TYPE_UNLINK_IDENTITY {
                            connection_header = ConnectionHeader::read(&self.socket_receive_buffer);
                            if self.try_unlink_identity(
                                address,
                                connection_header.id,
                                connection_header.session_id,
                            ) {
                                self.fire_identity_event(
                                    UNLINK_IDENTITY_EVENT,
                                    address,
                                    connection_header.id,
                                    connection_header.session_id,
                                );
                            }
                            return Err(ReceiveError::Retry);
                        } else if !self.validate_and_update_linked_connection(address) {
                            return Err(ReceiveError::Retry);
                        }
                    } else {
                        self.on_receive_connection_update(address);
                    }
                } else if self.config.use_identity == 1 {
                    if header.message_type == MESSAGE_TYPE_IDENTITY_LINKED {
                        self.identity.set_linked(1);
                        self.fire_identity_event(IDENTITY_LINKED_EVENT, address, 0, 0);

                        return Err(ReceiveError::Retry);
                    } else if header.message_type == MESSAGE_TYPE_IDENTITY_UNLINKED {
                        self.identity.set_linked(0);
                        self.fire_identity_event(IDENTITY_UNLINKED_EVENT, address, 0, 0);
                        return Err(ReceiveError::Retry);
                    }

                    if !self.identity.is_linked() {
                        return Err(ReceiveError::Retry);
                    }
                }
            }
            Err(SocketReceiveError::Empty) => {
                return Err(ReceiveError::Empty);
            }
            Err(SocketReceiveError::Error) => {
                return Err(ReceiveError::Error);
            }
            Err(SocketReceiveError::Dropped) => {
                self.stats.packets_dropped += 1;
                return Err(ReceiveError::Retry);
            }
        }

        if header.message_type == MESSAGE_TYPE_UNRELIABLE {
            self.stats.unreliable_received += 1;
            return Ok(ReceiveSuccess {
                receive_type: ReceiveType::Unreliable { received_len },
                network_address: address,
            });
        }

        let Some(channel) = self.channels.get_mut(&(address, header.channel)) else {
            return Err(ReceiveError::ChannelError);
        };

        channel.stats.bytes_received += received_len as u64;

        if header.message_type == MESSAGE_TYPE_NONE {
            channel.process_none_message(
                header.sequence,
                &mut self.socket_receive_buffer,
                received_len,
            );
            return Err(ReceiveError::Retry);
        }

        if header.message_type == MESSAGE_TYPE_NACK {
            channel.process_nack_message(address, &mut self.socket_receive_buffer);
            return Err(ReceiveError::Retry);
        }

        if header.message_type == MESSAGE_TYPE_FRAGMENT {
            channel.process_fragment_message(
                header.sequence,
                &mut self.socket_receive_buffer,
                received_len,
            );
            return Err(ReceiveError::Retry);
        }

        if header.message_type == MESSAGE_TYPE_RELIABLE
            || header.message_type == MESSAGE_TYPE_RELIABLE_WITH_NACK
        {
            if header.message_type == MESSAGE_TYPE_RELIABLE_WITH_NACK {
                channel.process_single_nack(address, &mut self.socket_receive_buffer);
            }

            return if channel.receiver.receive_packet(
                header.sequence,
                &self.socket_receive_buffer,
                received_len,
            ) {
                channel.stats.received += 1;
                Ok(ReceiveSuccess {
                    network_address: address,
                    receive_type: ReceiveType::Reliable {
                        channel_id: channel.id,
                    },
                })
            } else {
                Err(ReceiveError::Retry)
            };
        }

        Err(ReceiveError::Error)
    }

    pub fn send_to_target(
        &mut self,
        channel: u8,
        target: SendTarget,
        data: &mut [u8],
        length: usize,
    ) -> Result<TachyonSendSuccess, TachyonSendError> {
        let mut address = target.address;

        if target.identity_id > 0 {
            if let Some(addr) = self.identity_to_address_map.get(&target.identity_id) {
                address = *addr;
            } else {
                return Err(TachyonSendError::Identity);
            }
        }

        if channel > 0 {
            self.send_reliable(channel, address, data, length)
        } else {
            self.send_unreliable(address, data, length)
        }
    }

    pub fn send_unreliable(
        &mut self,
        address: NetworkAddress,
        data: &[u8],
        body_len: usize,
    ) -> Result<TachyonSendSuccess, TachyonSendError> {
        if !self.can_send() {
            return Err(TachyonSendError::Identity);
        }

        let Some(sender) = &mut self.unreliable_sender else {
            return Err(TachyonSendError::Unknown);
        };

        let result = sender.send(address, data, body_len);
        if result.is_ok() {
            self.stats.unreliable_sent += 1;
        }

        result
    }

    pub fn send_reliable(
        &mut self,
        channel_id: u8,
        address: NetworkAddress,
        data: &mut [u8],
        body_len: usize,
    ) -> Result<TachyonSendSuccess, TachyonSendError> {
        if !self.can_send() {
            return Err(TachyonSendError::Identity);
        }

        if body_len == 0 {
            return Err(TachyonSendError::Length);
        }

        if channel_id == 0 {
            return Err(TachyonSendError::Channel);
        }

        if self.socket.socket.is_none() {
            return Err(TachyonSendError::Socket);
        }

        let Some(channel) = self.channels.get_mut(&(address, channel_id)) else {
            return Err(TachyonSendError::Channel);
        };

        if Fragmentation::should_fragment(body_len) {
            let mut fragment_bytes_sent = 0;
            let frag_sequences = channel.frag.create_fragments(
                &mut channel.send_buffers,
                channel.id,
                data,
                body_len,
            );

            if frag_sequences.is_empty() {
                return Err(TachyonSendError::Fragment);
            }

            for seq in frag_sequences {
                let Some(fragment) = channel.send_buffers.get_send_buffer(seq) else {
                    return Err(TachyonSendError::Fragment);
                };

                let sent = self
                    .socket
                    .send_to(
                        address,
                        fragment.byte_buffer.get(),
                        fragment.byte_buffer.length,
                    )
                    .unwrap_or_default();

                fragment_bytes_sent += sent;

                channel.stats.bytes_sent += sent as u64;
                channel.stats.fragments_sent += 1;
            }

            channel.stats.sent += 1;

            return Ok(TachyonSendSuccess {
                header: Header {
                    message_type: MESSAGE_TYPE_FRAGMENT,
                    ..Header::default()
                },
                sent_len: fragment_bytes_sent as u32,
            });
        }

        channel.send_reliable(address, data, body_len, &self.socket)
    }
}

#[cfg(test)]
mod tests {
    use claims::{assert_err, assert_ok};

    use crate::channel::ChannelConfig;
    use crate::header::TACHYON_HEADER_SIZE;
    use crate::network_address::NetworkAddress;
    use crate::pool::SendTarget;
    use crate::tachyon_test::TachyonTest;
    use crate::{Tachyon, TachyonConfig};

    #[test]
    #[serial_test::serial]
    fn test_connect_before_bind() {
        let address = NetworkAddress::test_address();
        let mut buffer: Vec<u8> = vec![0; 1024];
        let config = TachyonConfig::default();
        let mut server = Tachyon::create(config);
        let mut client = Tachyon::create(config);

        assert!(client.connect(address));
        server.bind(address);

        let target = SendTarget {
            address: NetworkAddress::default(),
            identity_id: 0,
        };
        let sent = client.send_to_target(1, target, &mut buffer, 32);
        assert_ok!(sent);

        let res = server.receive_loop(&mut buffer).unwrap();
        assert_eq!(32, res.length);
    }

    #[test]
    #[serial_test::serial]
    fn test_server_receive_invalid_without_bind() {
        let mut buffer: Vec<u8> = vec![0; 1024];
        let config = TachyonConfig::default();
        let mut server = Tachyon::create(config);
        let res = server.receive_loop(&mut buffer);
        assert_err!(res);
    }

    #[test]
    #[serial_test::serial]
    fn test_reliable() {
        // reliable messages just work with message bodies, headers are all internal

        let mut test = TachyonTest::default();
        test.connect();

        test.send_buffer[0] = 4;
        let sent = test.client_send_reliable(1, 2);
        // sent_len reports total including header.
        assert_eq!(2 + TACHYON_HEADER_SIZE, sent.unwrap().sent_len as usize);

        let res = test.server_receive().unwrap();
        assert_eq!(2, res.length);
        assert_eq!(4, test.receive_buffer[0]);

        let _ = test.client_send_reliable(2, 33).unwrap();
        let res = test.server_receive().unwrap();
        assert_eq!(33, res.length);

        // fragmented
        let _ = test.client_send_reliable(2, 3497).unwrap();
        let res = test.server_receive().unwrap();
        assert_eq!(3497, res.length);
    }

    #[test]
    #[serial_test::serial]
    fn test_unconfigured_channel_fails() {
        let mut test = TachyonTest::default();
        let channel_config = ChannelConfig::default_ordered();
        test.client.configure_channel(3, channel_config);
        test.connect();

        let sent = test.client_send_reliable(3, 2);
        assert_eq!(2 + TACHYON_HEADER_SIZE, sent.unwrap().sent_len as usize);

        let res = test.server_receive();
        assert_err!(res);
    }

    #[test]
    #[serial_test::serial]
    fn test_configured_channel() {
        let mut test = TachyonTest::default();
        let channel_config = ChannelConfig::default_ordered();
        test.client.configure_channel(3, channel_config);
        test.server.configure_channel(3, channel_config);
        test.connect();

        let sent = test.client_send_reliable(3, 2);
        assert_eq!(2 + TACHYON_HEADER_SIZE, sent.unwrap().sent_len as usize);

        let res = test.server_receive().unwrap();
        assert_eq!(2, res.length);
    }

    #[test]
    #[serial_test::serial]
    fn test_unreliable() {
        let mut test = TachyonTest::default();
        test.connect();

        // unreliable messages need to be body length + 1;
        // send length error
        let send = test.client_send_unreliable(0);
        assert_err!(send);

        let res = test.server_receive().unwrap();
        assert_eq!(0, res.length);

        test.receive_buffer[0] = 1;
        test.send_buffer[0] = 3;
        test.send_buffer[1] = 4;
        test.send_buffer[2] = 5;
        test.send_buffer[3] = 6;
        let sent = test.client_send_unreliable(4).unwrap();
        assert_eq!(5, sent.sent_len as usize);

        let res = test.server_receive().unwrap();
        assert_eq!(4, res.length);
        assert_eq!(3, test.receive_buffer[0]);
        assert_eq!(4, test.receive_buffer[1]);
        assert_eq!(5, test.receive_buffer[2]);
        assert_eq!(6, test.receive_buffer[3]);
    }
}
