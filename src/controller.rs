use crate::protocol;
use crate::protocol::response::Response;
use core::num;
use crossbeam::channel::{unbounded, Receiver, Sender};
use dashmap::DashMap;
use log::warn;
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::thread;
use thiserror::Error;

#[derive(Debug)]
pub struct Controller {
    socket: UdpSocket,
    connections: Arc<DashMap<IpAddr, Sender<Response>>>,
}
const MAX_DATA_LENGTH: usize = 480 * 3;

pub struct Connection {
    pub pixel_config: protocol::PixelConfig,
    pub id: protocol::ID,

    sequence_number: u8,
    socket: UdpSocket,
    addr: SocketAddr,
    recv: Receiver<Response>,

    buffer: [u8; 1500],
}

impl Connection {
    pub fn write(&mut self, data: &[u8], offset: u32) -> Result<usize, DDPError> {
        let mut h = protocol::Header::default();

        h.packet_type.push(false);
        h.sequence_number = self.sequence_number;
        h.pixel_config = self.pixel_config;
        h.id = self.id;
        h.offset = offset;
        h.length = data.len() as u16;

        let sent = self.slice_send(&mut h, data)?;
        //Send data
        // let p: Vec<u8> = protocol::Packet::from_data(h, data).into();

        Ok(sent)
    }

    fn slice_send(
        &mut self,
        header: &mut protocol::Header,
        data: &[u8],
    ) -> Result<usize, DDPError> {
        let mut offset = 0;
        let mut sent = 0;

        let num_iterations = (data.len() + MAX_DATA_LENGTH - 1) / MAX_DATA_LENGTH;
        let mut iter = 0;

        while offset < data.len() {
            iter += 1;

            if iter == num_iterations {
                header.packet_type.push(true);
            }

            let chunk_end = std::cmp::min(offset + MAX_DATA_LENGTH, data.len());
            let chunk = &data[offset..chunk_end];
            let len = self.assemble_packet(*header, chunk);
            sent += self.socket.send_to(&self.buffer[0..len], self.addr)?;

            // Increment sequence number
            if self.sequence_number > 15 {
                self.sequence_number = 1;
            } else {
                self.sequence_number += 1;
            }
            offset += MAX_DATA_LENGTH;
        }

        Ok(sent)
    }

    // doing this to avoid allocations per frame
    // micro optimization, but it's a hot path
    // esp running this embedded
    pub fn assemble_packet(&mut self, header: protocol::Header, data: &[u8]) -> usize {
        let header_bytes: [u8; 10] = header.into();
        self.buffer[0..10].copy_from_slice(&header_bytes);
        self.buffer[10..(10 + data.len())].copy_from_slice(data);

        return 10 + data.len();
    }
}

impl Controller {
    /// Create a new DDP controller instance
    ///
    /// Listens to the world on UDP port 4048.
    /// If that's not desired, use `Controller::new_with_socket` instead.
    pub fn new() -> Result<Controller, DDPError> {
        // Listen to world on 4048
        let socket = UdpSocket::bind("0.0.0.0:4048")?;

        let conn = Arc::new(DashMap::new());

        let socket_reciever = socket.try_clone()?;
        let conn_rec = conn.clone();

        thread::spawn(move || {
            // Define our receieve buffer, "1500 bytes should be enough for anyone".
            // Github copilot actually suggested that, so sassy LOL.
            let mut buffer: [u8; 1500] = [0; 1500];
            match Self::recieve_filter(&socket_reciever, &mut buffer, &conn_rec) {
                Ok(bytes_recieved) => {
                    let packet = protocol::Packet::from_bytes(&buffer[0..bytes_recieved]);
                }
                Err(err) => {
                    warn!("Error recieving packet: {:?}", err);
                }
            }
        });

        Ok(Controller {
            socket,
            connections: conn,
        })
    }

    fn recieve_filter(
        socket: &std::net::UdpSocket,
        mut buffer: &mut [u8],
        conn: &Arc<DashMap<IpAddr, Sender<Response>>>,
    ) -> Result<usize, DDPError> {
        let (number_of_bytes, src_addr) = socket.recv_from(&mut buffer)?;

        match conn.contains_key(&src_addr.ip()) {
            true => Ok(number_of_bytes),
            false => Err(DDPError::UnknownClient {
                from: src_addr,
                data: Vec::from(&buffer[0..number_of_bytes]),
            }),
        }
    }

    // Basically new but you get to define your own socket if you want to use another port.
    // This is useful for binding localhost or other port or tbh whatever tf you want.
    // pub fn new_with_socket(socket: UdpSocket) -> Controller {
    //     Controller {
    //         socket,
    //         connections: HashSet::new(),
    //     }
    // }

    pub fn connect<A>(
        &mut self,
        addr: A,
        pixel_config: protocol::PixelConfig,
        id: protocol::ID,
    ) -> Result<Connection, DDPError>
    where
        A: std::net::ToSocketAddrs,
    {
        let socket_addr: SocketAddr = addr
            .to_socket_addrs()?
            .next()
            .ok_or(DDPError::NoValidSocketAddr)?;
        let (s, recv) = unbounded();
        self.connections.insert(socket_addr.ip(), s);

        let socket = self.socket.try_clone()?;

        Ok(Connection {
            addr: socket_addr,
            pixel_config,
            id,
            socket,
            recv,
            sequence_number: 1,
            buffer: [0; 1500],
        })
    }

    pub fn discover<A>(&mut self, addr: A)
    where
        A: std::net::ToSocketAddrs,
    {
    }
}

#[derive(Serialize, Deserialize)]
struct Person {
    name: String,
    age: u8,
    phones: Vec<String>,
}

#[derive(Error, Debug)]
pub enum DDPError {
    #[error("socket error")]
    Disconnect(#[from] std::io::Error),
    #[error("No valid socket addr found")]
    NoValidSocketAddr,
    #[error("invalid sender, did you forget to connect() ( data from {from:?} - {data:?})")]
    UnknownClient { from: SocketAddr, data: Vec<u8> },
    #[error("unknown data store error")]
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_conn() {
        let conn = Controller::new();
        assert!(conn.is_ok());
    }
}
