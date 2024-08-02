use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use socket2::{Domain, Socket, Type};

use crate::header::MESSAGE_TYPE_RELIABLE;
use crate::int_buffer::IntBuffer;
use crate::network_address::NetworkAddress;

pub struct SocketReceiveSuccess {
    pub bytes_received: usize,
    pub network_address: NetworkAddress,
}

pub enum SocketReceiveError {
    Empty,
    Error,
    Dropped,
}

pub struct TachyonSocket {
    pub address: NetworkAddress,
    pub is_server: bool,
    pub socket: Option<UdpSocket>,
    pub rng: StdRng,
}

impl TachyonSocket {
    #[must_use]
    pub fn create() -> Self {
        Self {
            address: NetworkAddress::default(),
            is_server: false,
            socket: None,
            rng: SeedableRng::seed_from_u64(32634),
        }
    }

    #[must_use]
    pub fn clone_socket(&self) -> Option<UdpSocket> {
        self.socket.as_ref()?.try_clone().ok()
    }

    pub fn bind_socket(&mut self, naddress: NetworkAddress) -> Result<(), ()> {
        if self.socket.is_some() {
            return Err(());
        }

        let address = naddress.to_socket_addr();
        self.address = naddress;

        let socket = Socket::new(Domain::IPV4, Type::DGRAM, None).unwrap();

        let Ok(()) = socket.bind(&address.into()) else {
            return Err(());
        };

        socket.set_recv_buffer_size(8192 * 256).unwrap();
        socket.set_nonblocking(true).unwrap();
        self.socket = Some(socket.into());
        self.is_server = true;

        Ok(())
    }

    pub fn connect_socket(&mut self, naddress: NetworkAddress) -> Result<(), ()> {
        if self.socket.is_some() {
            return Err(());
        }

        self.address = NetworkAddress::default();
        let sock_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0);
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, None).unwrap();

        let Ok(()) = socket.bind(&sock_addr.into()) else {
            return Err(());
        };

        socket.set_recv_buffer_size(8192 * 256).unwrap();
        socket.set_nonblocking(true).unwrap();

        let address = naddress.to_socket_addr();
        let udp_socket: UdpSocket = socket.into();

        let Ok(()) = udp_socket.connect(address) else {
            return Err(());
        };

        self.socket = Some(udp_socket);

        Ok(())
    }

    fn should_drop(&mut self, data: &[u8], drop_chance: u64, drop_reliable_only: bool) -> bool {
        let r = self.rng.gen_range(1..100);
        if r > drop_chance {
            return false;
        }

        let mut reader = IntBuffer { index: 0 };
        let message_type = reader.read_u8(data);

        // Drop everything
        !drop_reliable_only
            // or if drop only reliable and message type is reliable
            || message_type == MESSAGE_TYPE_RELIABLE
    }

    pub fn receive(
        &mut self,
        data: &mut [u8],
        drop_chance: u64,
        drop_reliable_only: bool,
    ) -> Result<SocketReceiveSuccess, SocketReceiveError> {
        if self.should_drop(data, drop_chance, drop_reliable_only) {
            return Err(SocketReceiveError::Dropped);
        }

        let Some(socket) = &self.socket else {
            return Err(SocketReceiveError::Error);
        };

        if self.is_server {
            Self::recv_server(socket, data)
        } else {
            Self::recv_client(socket, data)
        }
    }

    fn recv_server(
        socket: &UdpSocket,
        data: &mut [u8],
    ) -> Result<SocketReceiveSuccess, SocketReceiveError> {
        match socket.recv_from(data) {
            Ok((bytes_received, src_addr)) => {
                let address = NetworkAddress::from_socket_addr(src_addr);
                Ok(SocketReceiveSuccess {
                    bytes_received,
                    network_address: address,
                })
            }
            Err(_) => Err(SocketReceiveError::Empty),
        }
    }

    fn recv_client(
        socket: &UdpSocket,
        data: &mut [u8],
    ) -> Result<SocketReceiveSuccess, SocketReceiveError> {
        socket
            .recv(data)
            .map(|size| SocketReceiveSuccess {
                bytes_received: size,
                network_address: NetworkAddress::default(),
            })
            .map_err(|_| SocketReceiveError::Empty)
    }

    pub fn send_to(
        &self,
        address: NetworkAddress,
        data: &[u8],
        length: usize,
    ) -> Result<usize, ()> {
        let socket = self.socket.as_ref().ok_or(())?;

        let slice = &data[0..length];
        let socket_result = if address.port == 0 {
            socket.send(slice)
        } else {
            socket.send_to(slice, address.to_socket_addr())
        };

        socket_result.map_err(|_| ())
    }
}
