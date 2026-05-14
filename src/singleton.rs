use std::net::{Ipv4Addr, SocketAddrV4, TcpListener};

/// Singleton port. Bind succeeds only for the first instance; later instances exit silently.
const SINGLETON_PORT: u16 = 47999;

pub struct SingletonLock(TcpListener);

pub fn acquire() -> Option<SingletonLock> {
    let addr = SocketAddrV4::new(Ipv4Addr::LOCALHOST, SINGLETON_PORT);
    TcpListener::bind(addr).ok().map(SingletonLock)
}
