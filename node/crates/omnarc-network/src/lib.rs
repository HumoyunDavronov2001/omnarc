//! Networking of Omnarc nodes on iroh: QUIC connections dialed by public key, and tensor streams.
//!
//! Stage B1 covers what a ring of devices given by hand needs: a node identity, an endpoint on a known address
//! without relays or address lookup, and links — unidirectional streams of tensor frames
//! (`protocol/FRAME.md`) that feed one window of layers on the next device. Stage B2 adds clusters
//! (`protocol/CLUSTER.md`): the cluster key and membership tokens ([`cluster`]), the ticket and four-emoji check of
//! joining ([`ticket`]), the join conversation itself ([`join`]), the signed gossip of a cluster ([`gossip`]) and the
//! measurement of the links between members ([`links`]); members on other networks meet through relays ([`relay`]).

pub mod cluster;
pub mod gossip;
pub mod join;
pub mod links;
pub mod relay;
pub mod ticket;

use std::fmt;
use std::net::SocketAddr;
use std::path::Path;
use std::str::FromStr;

pub use iroh::address_lookup::memory::MemoryLookup;
use iroh::endpoint::presets;
pub use iroh::endpoint::{Connection, RecvStream, SendStream};
pub use iroh::protocol::{self, Router};
pub use iroh::{Endpoint, EndpointAddr, EndpointId, SecretKey, TransportAddr};
pub use iroh_mdns_address_lookup::{DiscoveryEvent, MdnsAddressLookup};
use omnarc_protocol::frame::{FrameError, FrameHeader, HEADER_LEN};

/// ALPN of the tensor streams between neighbors on a ring.
pub const TENSOR_ALPN: &[u8] = b"omnarc/tensor/1";

/// Magic bytes at the start of a link: a stream of tensor frames for one window of layers.
pub const LINK_MAGIC: [u8; 4] = *b"OML1";

/// An error of the network layer.
#[derive(Debug)]
pub struct Error(String);

impl Error {
    fn new(context: &str, error: impl fmt::Display) -> Self {
        Self(format!("{context}: {error}"))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// Loads this node's secret key from `path`, or creates one there. The public half is the node's id.
///
/// # Errors
/// The file cannot be read or written, or it does not hold a key.
pub fn load_or_create_key(path: &Path) -> Result<SecretKey> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let bytes: [u8; 32] = bytes.try_into().map_err(|_| Error(format!("{} is not a key", path.display())))?;
            Ok(SecretKey::from_bytes(&bytes))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let key = SecretKey::generate();
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| Error::new("creating the key directory", e))?;
            }
            write_private(path, &key.to_bytes()).map_err(|e| Error::new("writing the key", e))?;
            Ok(key)
        }
        Err(e) => Err(Error::new("reading the key", e)),
    }
}

/// Writes a new file that only this user can read: a key or another secret. An existing file is never overwritten.
///
/// # Errors
/// The file exists or cannot be written.
#[cfg(unix)]
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?;
    file.write_all(bytes)
}

/// Writes a new file that only this user can read: a key or another secret. An existing file is never overwritten.
///
/// # Errors
/// The file exists or cannot be written.
#[cfg(not(unix))]
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)
}

/// A peer given by hand: `<node id>@<ip:port>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    pub id: EndpointId,
    pub addr: SocketAddr,
}

impl FromStr for Peer {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let (id, addr) = s.split_once('@').ok_or_else(|| Error(format!("expected <node id>@<ip:port>, got {s}")))?;
        Ok(Self {
            id: id.parse().map_err(|e| Error::new("node id", e))?,
            addr: addr.parse().map_err(|e| Error::new("address", e))?,
        })
    }
}

impl fmt::Display for Peer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.id, self.addr)
    }
}

/// Binds an endpoint on `listen` that speaks the tensor protocol, with no relays and no address lookup.
///
/// # Errors
/// The address cannot be bound.
pub async fn bind(key: SecretKey, listen: SocketAddr) -> Result<Endpoint> {
    bind_with(key, listen, vec![TENSOR_ALPN.to_vec()]).await
}

/// Binds an endpoint on `listen` that accepts the protocols `alpns`, with no relays and no address lookup.
///
/// # Errors
/// The address cannot be bound.
pub async fn bind_with(key: SecretKey, listen: SocketAddr, alpns: Vec<Vec<u8>>) -> Result<Endpoint> {
    bind_with_relays(key, listen, alpns, &[]).await
}

/// [`bind_with`] that can also be reached through `relays`, for devices on other networks: inviting and joining.
///
/// # Errors
/// The address cannot be bound.
pub async fn bind_with_relays(
    key: SecretKey,
    listen: SocketAddr,
    alpns: Vec<Vec<u8>>,
    relays: &[relay::Relay],
) -> Result<Endpoint> {
    Endpoint::builder(presets::Minimal)
        .secret_key(key)
        .alpns(alpns)
        .relay_mode(relay::relay_mode(relays))
        .clear_ip_transports()
        .bind_addr(listen)
        .map_err(|e| Error::new("listen address", e))?
        .bind()
        .await
        .map_err(|e| Error::new("binding the endpoint", e))
}

/// Binds the endpoint of a cluster node on `listen`: no public relays and no public address lookup; members are
/// found through `lookup`, which the node fills with the addresses it learns from tickets and gossip, on the local
/// network through `lan` ([`lan_lookup`]), and on other networks through the cluster's own `relays`.
///
/// # Errors
/// The address cannot be bound.
pub async fn bind_cluster(
    key: SecretKey,
    listen: SocketAddr,
    lookup: MemoryLookup,
    lan: Option<MdnsAddressLookup>,
    relays: &[relay::Relay],
) -> Result<Endpoint> {
    cluster_builder(key, lookup, lan, relays)
        .bind_addr(listen)
        .map_err(|e| Error::new("listen address", e))?
        .bind()
        .await
        .map_err(|e| Error::new("binding the endpoint", e))
}

fn cluster_builder(
    key: SecretKey,
    lookup: MemoryLookup,
    lan: Option<MdnsAddressLookup>,
    relays: &[relay::Relay],
) -> iroh::endpoint::Builder {
    let mut builder = Endpoint::builder(presets::Minimal).secret_key(key).address_lookup(lookup);
    if let Some(lan) = lan {
        builder = builder.address_lookup(lan);
    }
    builder.relay_mode(relay::relay_mode(relays)).clear_ip_transports()
}

/// The service name Omnarc nodes announce themselves under on the local network: `<id>._omnarc._udp.local`.
pub const MDNS_SERVICE: &str = "omnarc";

/// Announces the node `id` on the local network by mDNS and finds the other nodes there, so a member is found even
/// after its address changed (`protocol/CLUSTER.md` §3.4). Give it to [`bind_cluster`].
///
/// # Errors
/// The network allows neither IPv4 nor IPv6 multicast.
///
/// # Panics
/// Outside a tokio runtime.
pub fn lan_lookup(id: EndpointId) -> Result<MdnsAddressLookup> {
    MdnsAddressLookup::builder().service_name(MDNS_SERVICE).build(id).map_err(|e| Error::new("mDNS", e))
}

/// Dials a peer by its id at a known address.
///
/// # Errors
/// The peer cannot be reached, or it is not the node with this id.
pub async fn connect(endpoint: &Endpoint, peer: &Peer) -> Result<Connection> {
    // port 0: no address is known, and the endpoint's own lookup finds the peer (a cluster's members, its relays)
    let addr = if peer.addr.port() == 0 {
        EndpointAddr::from(peer.id)
    } else {
        EndpointAddr::from_parts(peer.id, [TransportAddr::Ip(peer.addr)])
    };
    endpoint.connect(addr, TENSOR_ALPN).await.map_err(|e| Error::new(&format!("connecting to {peer}"), e))
}

/// Opens a link to the window that starts at layer `window_begin` on the peer.
///
/// # Errors
/// The connection is gone.
pub async fn open_link(connection: &Connection, window_begin: u32) -> Result<SendStream> {
    let mut stream = connection.open_uni().await.map_err(|e| Error::new("opening a link", e))?;
    let mut header = [0u8; 8];
    header[..4].copy_from_slice(&LINK_MAGIC);
    header[4..].copy_from_slice(&window_begin.to_le_bytes());
    stream.write_all(&header).await.map_err(|e| Error::new("writing a link header", e))?;
    Ok(stream)
}

/// Accepts the next link on a connection: the stream and the first layer of the window it feeds. `None` when the
/// connection closes.
///
/// # Errors
/// The stream does not start with a link header.
pub async fn accept_link(connection: &Connection) -> Result<Option<(u32, RecvStream)>> {
    let Ok(mut stream) = connection.accept_uni().await else {
        return Ok(None);
    };
    let mut header = [0u8; 8];
    stream.read_exact(&mut header).await.map_err(|e| Error::new("reading a link header", e))?;
    if header[..4] != LINK_MAGIC {
        return Err(Error("a link starts with an unknown magic".into()));
    }
    Ok(Some((u32::from_le_bytes([header[4], header[5], header[6], header[7]]), stream)))
}

/// Writes one tensor frame.
///
/// # Errors
/// The payload does not match the header, or the stream is gone.
pub async fn write_frame(stream: &mut SendStream, header: &FrameHeader, payload: &[u8]) -> Result<()> {
    if payload.len() != header.payload_len as usize {
        return Err(Error("the payload does not match the frame header".into()));
    }
    stream.write_all(&header.encode()).await.map_err(|e| Error::new("writing a frame", e))?;
    stream.write_all(payload).await.map_err(|e| Error::new("writing a frame", e))
}

/// Reads one tensor frame into `payload`; `None` when the stream ends cleanly between frames.
///
/// # Errors
/// The stream breaks inside a frame, or the header is invalid.
pub async fn read_frame(stream: &mut RecvStream, payload: &mut Vec<u8>) -> Result<Option<FrameHeader>> {
    let mut head = [0u8; HEADER_LEN];
    let mut filled = 0;
    while filled < HEADER_LEN {
        match stream.read(&mut head[filled..]).await.map_err(|e| Error::new("reading a frame", e))? {
            Some(0) => {}
            Some(n) => filled += n,
            None if filled == 0 => return Ok(None),
            None => return Err(Error("the stream ended inside a frame header".into())),
        }
    }
    let header = FrameHeader::decode(&head).map_err(|e: FrameError| Error::new("frame header", e))?;
    payload.resize(header.payload_len as usize, 0);
    stream.read_exact(payload).await.map_err(|e| Error::new("reading a frame payload", e))?;
    Ok(Some(header))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn members_on_other_networks_meet_through_a_relay() {
        let (_map, url, _relay_server) = iroh::test_utils::run_relay_server().await.unwrap();
        let relays = [relay::Relay { url: url.clone(), token: None }];
        let any: SocketAddr = "127.0.0.1:0".parse().unwrap();
        // the relay of the tests has a certificate of its own
        let trust = || iroh::tls::CaTlsConfig::insecure_skip_verify();
        let server = cluster_builder(SecretKey::generate(), MemoryLookup::new(), None, &relays)
            .ca_tls_config(trust())
            .alpns(vec![links::PROBE_ALPN.to_vec()])
            .bind_addr(any)
            .unwrap()
            .bind()
            .await
            .unwrap();
        let router = Router::builder(server.clone()).accept(links::PROBE_ALPN, links::ProbeHandler).spawn();
        tokio::time::timeout(std::time::Duration::from_secs(10), server.online()).await.expect("homed on the relay");
        assert_eq!(server.addr().relay_urls().next(), Some(&url));

        let client = cluster_builder(SecretKey::generate(), MemoryLookup::new(), None, &relays)
            .ca_tls_config(trust())
            .bind_addr(any)
            .unwrap()
            .bind()
            .await
            .unwrap();
        // the client knows the server only by its id and its relay, as across two networks
        let addr = EndpointAddr::from_parts(server.id(), [relays[0].transport_addr()]);
        let connection = client.connect(addr, links::PROBE_ALPN).await.unwrap();
        links::ping(&connection).await.unwrap();
        client.close().await;
        router.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[cfg_attr(not(target_os = "linux"), ignore = "multicast on the CI runners of this system is untested")]
    async fn nodes_on_one_network_find_each_other_by_id_alone() {
        let (a, b) = (SecretKey::generate(), SecretKey::generate());
        let any: SocketAddr = "0.0.0.0:0".parse().unwrap();
        let lan_a = lan_lookup(a.public()).unwrap();
        let server = bind_cluster(a, any, MemoryLookup::new(), Some(lan_a), &[]).await.unwrap();
        let router = Router::builder(server.clone()).accept(links::PROBE_ALPN, links::ProbeHandler).spawn();
        let lan_b = lan_lookup(b.public()).unwrap();
        let client = bind_cluster(b, any, MemoryLookup::new(), Some(lan_b), &[]).await.unwrap();
        // no address is given: mDNS finds it
        let connection =
            tokio::time::timeout(std::time::Duration::from_secs(30), client.connect(server.id(), links::PROBE_ALPN))
                .await
                .expect("found within 30 s")
                .unwrap();
        links::ping(&connection).await.unwrap();
        client.close().await;
        router.shutdown().await.unwrap();
    }
    use omnarc_protocol::frame::{DType, Flags};

    #[test]
    fn peer_round_trips() {
        let id = SecretKey::generate().public();
        let peer: Peer = format!("{id}@127.0.0.1:7700").parse().unwrap();
        assert_eq!(peer.addr, "127.0.0.1:7700".parse().unwrap());
        assert_eq!(peer.to_string().parse::<Peer>().unwrap(), peer);
        assert!("no-at-sign".parse::<Peer>().is_err());
    }

    #[tokio::test]
    async fn frames_travel_over_a_link() {
        let a = bind(SecretKey::generate(), "127.0.0.1:0".parse().unwrap()).await.unwrap();
        let b = bind(SecretKey::generate(), "127.0.0.1:0".parse().unwrap()).await.unwrap();
        let peer = Peer { id: b.id(), addr: b.bound_sockets()[0] };

        let receiver = tokio::spawn(async move {
            let incoming = b.accept().await.unwrap().await.unwrap();
            let (window, mut stream) = accept_link(&incoming).await.unwrap().unwrap();
            let mut payload = Vec::new();
            let mut frames = Vec::new();
            while let Some(header) = read_frame(&mut stream, &mut payload).await.unwrap() {
                frames.push((header, payload.clone()));
            }
            (window, frames)
        });

        let connection = connect(&a, &peer).await.unwrap();
        let mut link = open_link(&connection, 20).await.unwrap();
        for step in 0..3u32 {
            let payload: Vec<u8> = (0..16u8).map(|i| i + u8::try_from(step).unwrap()).collect();
            let header = FrameHeader::new(DType::F16, Flags::empty(), 9, step, step, 2, 4).unwrap();
            write_frame(&mut link, &header, &payload).await.unwrap();
        }
        link.finish().unwrap();

        let (window, frames) = receiver.await.unwrap();
        assert_eq!(window, 20);
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[2].0.step, 2);
        assert_eq!(frames[2].1[0], 2);
        connection.close(0u32.into(), b"done");
    }
}
