use crate::network_address::NetworkAddress;

pub struct ReceiveSuccess {
    pub network_address: NetworkAddress,
    pub receive_type: ReceiveType,
}

pub enum ReceiveType {
    Reliable {
        channel_id: u8,
    },
    Unreliable {
        received_len: usize,
    },
}

pub enum ReceiveError {
    Error,
    Empty,
    Retry,
    ChannelError,
}
