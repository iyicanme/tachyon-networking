use crate::network_address::NetworkAddress;

#[derive(Default, Clone, Copy)]
pub struct TachyonReceiveResult {
    pub channel: u16,
    pub address: NetworkAddress,
    pub length: u32,
    pub error: u32,
}
