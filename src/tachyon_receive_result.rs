use crate::network_address::NetworkAddress;

#[derive(Clone, Copy, Debug)]
pub struct TachyonReceiveSuccess {
    pub receive_type: TachyonReceiveType,
    pub address: NetworkAddress,
    pub length: u32,
}

#[derive(Clone, Copy, Debug)]
pub enum TachyonReceiveType {
    Reliable { channel: u16 },
    Unreliable,
}

#[derive(Clone, Copy, Debug)]
pub enum TachyonReceiveError {
    Unknown,
    Channel,
}