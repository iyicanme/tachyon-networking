use crate::network_address::NetworkAddress;

#[derive(Clone, Copy)]
#[repr(C)]
pub struct Connection {
    pub address: NetworkAddress,
    pub identity: Identity,
    pub tachyon_id: u16,
    pub received_at: u64,
    pub since_last_received: u64,
}

impl Connection {
    #[must_use]
    pub fn create(address: NetworkAddress, tachyon_id: u16) -> Self {
        Self {
            identity: Identity::default(),
            address,
            tachyon_id,
            received_at: 0,
            since_last_received: 0,
        }
    }
}

#[derive(Clone, Copy)]
#[repr(C)]
#[derive(Default)]
pub struct Identity {
    pub id: u32,
    pub session_id: u32,
    pub linked: u32,
}

impl Identity {
    #[must_use]
    pub const fn is_valid(&self) -> bool {
        self.id > 0 && self.session_id > 0
    }

    #[must_use]
    pub const fn is_linked(&self) -> bool {
        self.is_valid() && self.linked == 1
    }

    pub fn set_linked(&mut self, linked: u32) {
        self.linked = linked;
    }
}
