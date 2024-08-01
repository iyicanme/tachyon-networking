use crate::network_address::NetworkAddress;

pub const RECEIVE_ERROR_UNKNOWN: u32 = 1;
pub const RECEIVE_ERROR_CHANNEL: u32 = 2;

pub enum ReceiveResult {
    Reliable {
        network_address: NetworkAddress,
        channel_id: u8,
    },
    Error,
    Empty,
    Retry,
    ChannelError,
    UnReliable {
        received_len: usize,
        network_address: NetworkAddress,
    },
}
