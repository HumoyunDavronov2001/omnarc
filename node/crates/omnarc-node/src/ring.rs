//! A ring of devices that runs one model together — prima.cpp's piped ring on Omnarc's engine and network.
//!
//! Every node of the ring holds windows of layers. The node that serves the request, the head, holds the first
//! layers and the output head; the hidden state of every token travels around the ring as tensor frames, one window
//! after another, in as many rounds as the nodes have windows, and comes back to the head as logits. While the other
//! nodes compute, a node reads its next window's weights from disk (prefetching), so a model larger than a device's
//! memory still runs.
//!
//! Stage B1 wires the ring by hand: each node is told its windows, the next node (`<id>@<ip:port>`) and the previous
//! node's id. Discovery, pairing and planning come in stages B2 and B3. Every node of a ring must use the same wire
//! format for hidden states.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io::Write as _;
use std::net::SocketAddr;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use omnarc_engine_host::{Backend, DType, Engine, Input, KvType, SamplerParams, ShardInfo, Step};
use omnarc_network::{Connection, Endpoint, EndpointId, Peer, SendStream};
use omnarc_protocol::frame::{Flags, FrameHeader};
use tokio::sync::{mpsc, watch};

pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Tokens this process's windows computed, and the time it took: a device's contribution, for its telemetry.
pub static TOKENS_COMPUTED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static MICROS_COMPUTING: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn tally(tokens: u32, since: Instant) {
    use std::sync::atomic::Ordering::Relaxed;
    TOKENS_COMPUTED.fetch_add(u64::from(tokens), Relaxed);
    MICROS_COMPUTING.fetch_add(u64::try_from(since.elapsed().as_micros()).unwrap_or(u64::MAX), Relaxed);
}

/// Where a ring node's connections come from: an endpoint of its own, for a ring wired by hand, or the node
/// service's, whose router hands over the tensor connections it accepts from members.
pub struct Net {
    endpoint: Endpoint,
    incoming: tokio::sync::Mutex<mpsc::Receiver<Connection>>,
    // the accept loop of an endpoint of its own
    accepting: Option<tokio::task::JoinHandle<()>>,
}

impl fmt::Debug for Net {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Net").field("endpoint", &self.endpoint.id()).finish_non_exhaustive()
    }
}

impl Net {
    /// The service's endpoint, with the tensor connections its router accepted.
    #[must_use]
    pub fn shared(endpoint: Endpoint, incoming: mpsc::Receiver<Connection>) -> Self {
        Self { endpoint, incoming: tokio::sync::Mutex::new(incoming), accepting: None }
    }

    /// An endpoint of its own for a ring wired by hand, with `config`'s key and address.
    ///
    /// # Errors
    /// The key cannot be read or the address cannot be bound.
    pub async fn own(config: &Config) -> Result<Self, BoxError> {
        let endpoint = bind(config).await?;
        let (tx, incoming) = mpsc::channel(4);
        let accepting = endpoint.clone();
        let accepting = tokio::spawn(async move {
            while let Some(incoming) = accepting.accept().await {
                if let Ok(connection) = incoming.await
                    && tx.send(connection).await.is_err()
                {
                    return;
                }
            }
        });
        Ok(Self { endpoint, incoming: tokio::sync::Mutex::new(incoming), accepting: Some(accepting) })
    }

    #[must_use]
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }
}

impl Drop for Net {
    fn drop(&mut self) {
        if let Some(accepting) = self.accepting.take() {
            accepting.abort();
        }
    }
}

/// A window of layers written `begin:end`; layer N of an N-layer model is the output head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window(pub Range<u32>);

impl FromStr for Window {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        let (a, b) = s.split_once(':').ok_or_else(|| format!("expected begin:end, got {s}"))?;
        let begin = a.trim().parse().map_err(|_| format!("bad layer {a}"))?;
        let end = b.trim().parse().map_err(|_| format!("bad layer {b}"))?;
        if begin >= end {
            return Err(format!("the window {s} is empty"));
        }
        Ok(Self(begin..end))
    }
}

impl fmt::Display for Window {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.0.start, self.0.end)
    }
}

/// What a ring node needs to know.
#[derive(Debug)]
pub struct Config {
    pub model: PathBuf,
    pub windows: Vec<Window>,
    pub listen: SocketAddr,
    pub next: Peer,
    /// The device that takes the next node's place if it leaves: the replica of the window after this one.
    pub next_replica: Option<Peer>,
    /// Connections are accepted from these nodes. Normally one -- the previous node of the ring -- but a
    /// replica also accepts the node it mirrors, and the node that will become its previous one if the
    /// device it stands in for leaves (NO-43).
    pub prev: Vec<EndpointId>,
    pub key: PathBuf,
    pub engine: PathBuf,
    pub backend: Backend,
    pub max_tokens: u32,
    /// Tokens per prefill step.
    pub chunk: u32,
    /// The KV cache's element type (EN-16); the same on every node of a ring.
    pub kv: KvType,
    /// Element type of the hidden state between nodes: F16 (default), Bf16, F32 or F8E4M3 (EN-19).
    pub wire: DType,
    /// This node only mirrors: it runs the frames a live node sends it to keep its KV cache in step, and
    /// connects to no next node. Taking over as the live node is stage B3's next piece (NO-43).
    pub mirror_only: bool,
    /// The device that holds a replica of these windows, if there is one. It is sent a copy of every frame
    /// this node receives, runs it to keep its KV cache in step, and forwards nothing, so it can carry an
    /// answer on from the same token if this node leaves (EN-17).
    pub replica: Option<Peer>,
    /// Threads the engine may keep busy: the cores the owner offers (Q1); 0: all of them.
    pub threads: u32,
    /// Only on the head: tokens it guesses ahead from what was said before (n-gram drafts, EN-13) and has the ring
    /// check in one round; 0: one token per round. Only for greedy sampling, where a check changes no answer.
    pub draft: u32,
    /// Only on the head.
    pub prompt: Option<String>,
    pub generate: u32,
}

/// Timings of one generation, measured on the head.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub prompt_tokens: usize,
    pub generated: Vec<i32>,
    pub text: String,
    /// Time to first token.
    pub ttft: Duration,
    /// Mean time per output token after the first.
    pub tpot: Option<Duration>,
    /// The longest wait between two tokens: what a device leaving costs the answer (NO-43).
    pub longest_pause: Duration,
}

/// The engine is synchronous; calls run on blocking threads.
type SharedEngine = Arc<Mutex<Engine>>;

async fn with_engine<T: Send + 'static>(
    engine: &SharedEngine,
    f: impl FnOnce(&mut Engine) -> Result<T, omnarc_engine_host::Error> + Send + 'static,
) -> Result<T, BoxError> {
    let engine = Arc::clone(engine);
    let result = tokio::task::spawn_blocking(move || {
        let mut guard = engine.lock().map_err(|_| "the engine lock is poisoned".to_owned())?;
        f(&mut guard).map_err(|e| e.to_string())
    })
    .await?;
    result.map_err(Into::into)
}

/// The shards of this node, in ring order.
struct Node {
    engine: SharedEngine,
    shards: Vec<ShardInfo>,
    hidden_dim: u32,
    vocab_size: u32,
    wire: DType,
}

impl Node {
    /// Runs a frame through one of this node's shards; returns its output bytes.
    async fn run(&self, shard: usize, header: &FrameHeader, payload: Vec<u8>) -> Result<Vec<u8>, BoxError> {
        let info = self.shards[shard].clone();
        let (request, position, count) = (header.request_id, header.position_start, u32::from(header.token_count));
        let wire = self.engine_wire();
        let hidden = self.hidden_dim as usize;
        // 8-bit hidden states are opened to f32 for the engine, which speaks no f8 (EN-19)
        let (dtype, payload) = if header.dtype == DType::F8E4M3 {
            let values = omnarc_protocol::f8::unpack(&payload, hidden).ok_or("an f8 frame of the wrong length")?;
            (DType::F32, values.iter().flat_map(|v| v.to_le_bytes()).collect())
        } else {
            (header.dtype, payload)
        };
        let replay = header.flags.contains(Flags::REPLAY);
        let draft = header.flags.contains(Flags::DRAFT);
        let started = Instant::now();
        let output = with_engine(&self.engine, move |engine| {
            // the output window holds no layer and so no cache
            if (replay || draft) && info.layers.start < info.model.layer_count {
                // a resent step may already be in this cache -- a replica ran it as a mirror -- and a speculative
                // step follows drafts the head rejected: the cache is cut at the step's position first
                let _ = engine.truncate(info.id, request, position);
            }
            let input = Input::Hidden { dtype, count, data: &payload };
            // the drafts are checked against the logits of every position, in full precision
            let (all_logits, output) = if draft && info.gives_logits() { (true, DType::F32) } else { (false, wire) };
            let step = Step { request, position, input, all_logits, output };
            engine.run(&info, &step).map(|out| out.data.to_vec())
        })
        .await?;
        tally(count, started);
        let output = if self.shards[shard].gives_logits() { output } else { self.to_wire(output) };
        // the next round's window is read from disk while the rest of the ring computes
        if self.shards.len() > 1 {
            let next = self.shards[(shard + 1) % self.shards.len()].id;
            with_engine(&self.engine, move |engine| engine.prefetch(next)).await?;
        }
        Ok(output)
    }

    async fn run_tokens(
        &self,
        shard: usize,
        request: u64,
        position: u32,
        tokens: Vec<i32>,
    ) -> Result<Vec<u8>, BoxError> {
        let info = self.shards[shard].clone();
        let wire = self.engine_wire();
        let (started, count) = (Instant::now(), u32::try_from(tokens.len()).unwrap_or(u32::MAX));
        let output = with_engine(&self.engine, move |engine| {
            let step = Step { request, position, input: Input::Tokens(&tokens), all_logits: false, output: wire };
            engine.run(&info, &step).map(|out| out.data.to_vec())
        })
        .await?;
        tally(count, started);
        Ok(if self.shards[shard].gives_logits() { output } else { self.to_wire(output) })
    }

    /// What the engine hands out: the wire's own type, or f32 to be packed into 8 bits.
    fn engine_wire(&self) -> DType {
        if self.wire == DType::F8E4M3 { DType::F32 } else { self.wire }
    }

    /// The engine's hidden state as the wire carries it.
    fn to_wire(&self, output: Vec<u8>) -> Vec<u8> {
        if self.wire != DType::F8E4M3 {
            return output;
        }
        let values: Vec<f32> = output.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect();
        omnarc_protocol::f8::pack(&values, self.hidden_dim as usize)
    }

    async fn truncate(&self, shard: usize, request: u64, position: u32) {
        let id = self.shards[shard].id;
        // nothing cached past the position is nothing to cut
        let _ = with_engine(&self.engine, move |engine| engine.truncate(id, request, position)).await;
    }

    async fn drop_requests(&self, requests: &HashSet<u64>) {
        for &request in requests {
            for shard in &self.shards {
                let id = shard.id;
                // a failed drop only leaves KV cache cells behind; the next session reuses the shard anyway
                let _ = with_engine(&self.engine, move |engine| engine.drop_request(id, request)).await;
            }
        }
    }

    /// A frame carrying token ids for the replica of the window that holds layer 0: that window takes tokens,
    /// not a hidden state, so its replica cannot be kept in step with an ordinary frame.
    fn token_frame(&self, request: u64, step: u32, position: u32, count: u32) -> Result<FrameHeader, BoxError> {
        let flags = Flags::MIRROR.union(Flags::TOKENS);
        let hidden = u16::try_from(self.hidden_dim)?;
        Ok(FrameHeader::new(self.wire, flags, request, step, position, u16::try_from(count)?, hidden)?)
    }

    fn frame(
        &self,
        request: u64,
        step: u32,
        position: u32,
        count: u32,
        to_logits: bool,
    ) -> Result<FrameHeader, BoxError> {
        let flags = if to_logits { Flags::LAST_STAGE } else { Flags::empty() };
        let token_count = u16::try_from(count)?;
        let hidden = u16::try_from(self.hidden_dim)?;
        Ok(FrameHeader::new(self.wire, flags, request, step, position, token_count, hidden)?)
    }
}

/// Starts the engine and opens this node's windows.
fn open_node(config: &Config) -> Result<Node, BoxError> {
    // each half of the shared memory holds one prefill chunk of f32 hidden states or one row of logits
    let mut probe = Engine::spawn(&config.engine, 1 << 20)?;
    let info = probe.model_info(&config.model)?;
    probe.quit()?;
    let hidden = info.hidden_dim as usize;
    // a speculative step brings back the logits of every token it carries
    let rows = config.draft as usize + 1;
    let half = (config.chunk as usize * hidden * 4).max(rows * info.vocab_size as usize * 4) + (1 << 16);
    let mut engine = Engine::spawn(&config.engine, 2 * half)?;

    let mut shards = Vec::new();
    for window in &config.windows {
        if window.0.end > info.layer_count + 1 {
            return Err(format!("window {window} ends after the output head (layer {})", info.layer_count).into());
        }
        let shard =
            engine.open_with_kv(&config.model, window.0.clone(), config.backend, config.max_tokens, config.kv)?;
        if config.threads > 0 {
            engine.set_threads(shard.id, config.threads)?;
        }
        let backend = shard.backend.as_str_name().trim_start_matches("BACKEND_").to_lowercase();
        eprintln!("omnarc: layers {window} loaded on {backend}");
        shards.push(shard);
    }
    Ok(Node {
        engine: Arc::new(Mutex::new(engine)),
        shards,
        hidden_dim: info.hidden_dim,
        vocab_size: info.vocab_size,
        wire: config.wire,
    })
}

/// Connects to the next node, retrying while it starts, and opens one link per window boundary that leads to it.
/// Opens a link per window to the device holding the replica of this node's windows. A replica that cannot be
/// reached is reported and left out: it is redundancy, not something the ring depends on.
/// Resolves when the connection closes; never, when there is none.
async fn closed(connection: Option<&Connection>) {
    match connection {
        Some(c) => {
            let _ = c.closed().await;
        }
        None => std::future::pending().await,
    }
}

/// Sends the replica of a window a copy of a frame. A replica that cannot keep up is dropped, not waited for:
/// it is redundancy, and the answer must not slow down on its account.
async fn mirror(mirrors: &mut HashMap<u32, SendStream>, window: u32, header: &FrameHeader, payload: &[u8]) {
    let Some(link) = mirrors.get_mut(&window) else { return };
    if let Err(e) = omnarc_network::write_frame(link, header, payload).await {
        eprintln!("omnarc: the replica fell behind and was dropped ({e})");
        mirrors.clear();
    }
}

/// Sends a frame on to the next window, and if the device holding it has gone, to the replica standing in
/// for it. The resent frame carries `REPLAY`, so a receiver that already holds the step can tell. The ring
/// keeps generating: this is the one second of NO-43, not a restart.
#[allow(clippy::too_many_arguments)]
async fn forward(
    endpoint: &Endpoint,
    config: &Config,
    node: &Node,
    next: &mut Option<Connection>,
    links: &mut HashMap<u32, SendStream>,
    key: u32,
    header: &FrameHeader,
    payload: &[u8],
) -> Result<(), BoxError> {
    if let Some(link) = links.get_mut(&key) {
        match omnarc_network::write_frame(link, header, payload).await {
            Ok(()) => return Ok(()),
            Err(e) => eprintln!("omnarc: the next node stopped answering ({e})"),
        }
    }
    let peer = config.next_replica.clone().ok_or("the next node is gone and no replica stands in for it")?;
    let connection = omnarc_network::connect(endpoint, &peer).await?;
    let mut fresh = HashMap::new();
    for shard in &node.shards {
        if !shard.gives_logits() {
            fresh.insert(shard.layers.end, omnarc_network::open_link(&connection, shard.layers.end).await?);
        }
    }
    *links = fresh;
    if let Some(old) = next.replace(connection) {
        old.close(0u32.into(), b"replaced by the replica");
    }
    eprintln!("omnarc: the replica of the next window took over");
    let mut replay = *header;
    replay.flags = header.flags.union(Flags::REPLAY);
    let link = links.get_mut(&key).ok_or("the replica holds no link for this window")?;
    omnarc_network::write_frame(link, &replay, payload).await?;
    Ok(())
}

/// How long the head waits for the ring to bring a step back: a long prompt chunk on a slow device takes a while,
/// a ring that went silent takes forever.
const STEP_PATIENCE: Duration = Duration::from_secs(300);

/// The latest copy the replica confirmed holding: the request and the position after its last token.
type Acks = watch::Receiver<(u64, u32)>;

/// How long a node waits for its replica to confirm a copy before it goes on without one.
const ACK_PATIENCE: Duration = Duration::from_secs(2);

async fn connect_replica(
    endpoint: &Endpoint,
    config: &Config,
    node: &Node,
) -> (Option<Connection>, HashMap<u32, SendStream>, Option<Acks>) {
    let Some(peer) = config.replica.as_ref() else { return (None, HashMap::new(), None) };
    let connection = match omnarc_network::connect(endpoint, peer).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("omnarc: no replica for these windows ({e})");
            return (None, HashMap::new(), None);
        }
    };
    let mut links = HashMap::new();
    for shard in &node.shards {
        // the replica holds the same window, so it is addressed by where that window starts
        match omnarc_network::open_link(&connection, shard.layers.start).await {
            Ok(link) => {
                links.insert(shard.layers.start, link);
            }
            Err(e) => {
                eprintln!("omnarc: the replica refused the link for layer {} ({e})", shard.layers.start);
                return (None, HashMap::new(), None);
            }
        }
    }
    eprintln!("omnarc: mirroring {} window(s) to the replica", links.len());
    // the replica confirms every copy it took off the wire, on a stream of its own (protocol/tla/Takeover.tla)
    let (confirmed, acks) = watch::channel((0u64, 0u32));
    let reading = connection.clone();
    tokio::spawn(async move {
        let Ok(mut stream) = reading.accept_uni().await else { return };
        let mut ack = [0u8; 12];
        while stream.read_exact(&mut ack).await.is_ok() {
            let (request, end) = ack.split_at(8);
            let request = u64::from_le_bytes(request.try_into().unwrap_or_default());
            let end = u32::from_le_bytes(end.try_into().unwrap_or_default());
            if confirmed.send((request, end)).is_err() {
                return;
            }
        }
    });
    (Some(connection), links, Some(acks))
}

/// Waits until the replica holds the copy of `request` up to `end`; a replica that does not confirm in time is
/// dropped, and the ring goes on without a standby rather than without an answer.
async fn confirmed(acks: &mut Option<Acks>, mirrors: &mut HashMap<u32, SendStream>, request: u64, end: u32) {
    let Some(waiting) = acks.as_mut() else { return };
    if mirrors.is_empty() {
        return;
    }
    let held = tokio::time::timeout(ACK_PATIENCE, waiting.wait_for(|&(r, e)| r == request && e >= end))
        .await
        .is_ok_and(|seen| seen.is_ok());
    if !held {
        eprintln!("omnarc: the replica did not confirm a copy in time and was dropped");
        mirrors.clear();
        *acks = None;
    }
}

async fn connect_next(
    endpoint: &Endpoint,
    config: &Config,
    node: &Node,
) -> Result<(Connection, HashMap<u32, SendStream>), BoxError> {
    let connection = loop {
        match omnarc_network::connect(endpoint, &config.next).await {
            Ok(c) => break c,
            Err(e) => {
                eprintln!("omnarc: waiting for the next node ({e})");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    };
    let mut links = HashMap::new();
    for shard in &node.shards {
        if !shard.gives_logits() {
            // the window that starts where this one ends lives on the next node
            links.insert(shard.layers.end, omnarc_network::open_link(&connection, shard.layers.end).await?);
        }
    }
    Ok((connection, links))
}

/// What reaches a node's one queue: a frame of a session, or the end of a session once all its frames are in.
enum Inbound {
    Frame {
        session: u64,
        shard: usize,
        header: FrameHeader,
        payload: Vec<u8>,
    },
    /// The previous node closed the session's connection: in good order, or by going away.
    Closed {
        session: u64,
        graceful: bool,
    },
}

/// Accepts a previous node's connection as session `session` and the links it opens, one per window of this node
/// that takes hidden states, and forwards their frames to `inbox`; once the connection is gone and its last frame
/// is queued, the session's end follows in the same queue.
async fn accept_prev(
    net: &Net,
    config: &Config,
    node: &Node,
    session: u64,
    inbox: mpsc::Sender<Inbound>,
) -> Result<Connection, BoxError> {
    let connection = loop {
        let connection = net.incoming.lock().await.recv().await.ok_or("the endpoint closed")?;
        if config.prev.contains(&connection.remote_id()) {
            break connection;
        }
        eprintln!("omnarc: refused a connection from {}, not the previous node", connection.remote_id());
        connection.close(1u32.into(), b"not the previous node");
    };
    let expected = node.shards.iter().filter(|s| !s.takes_tokens()).count();
    // the confirmations of mirrored copies go back on one stream per connection
    let acks = Arc::new(tokio::sync::Mutex::new(None::<SendStream>));
    // the last link to run dry reports the session's end, so it comes after every frame of the session
    let open_links = Arc::new(std::sync::atomic::AtomicUsize::new(expected.max(1)));
    let end = move |connection: Connection,
                    inbox: mpsc::Sender<Inbound>,
                    open_links: Arc<std::sync::atomic::AtomicUsize>| async move {
        if open_links.fetch_sub(1, std::sync::atomic::Ordering::AcqRel) == 1 {
            let reason = connection.closed().await;
            let graceful = matches!(reason, omnarc_network::ConnectionError::ApplicationClosed(ref close) if close.error_code == 0u32.into());
            let _ = inbox.send(Inbound::Closed { session, graceful }).await;
        }
    };
    if expected == 0 {
        tokio::spawn(end(connection.clone(), inbox.clone(), Arc::clone(&open_links)));
    }
    for _ in 0..expected {
        let (begin, mut stream) = omnarc_network::accept_link(&connection).await?.ok_or("the previous node left")?;
        let shard = node
            .shards
            .iter()
            .position(|s| s.layers.start == begin)
            .ok_or_else(|| format!("the previous node feeds layer {begin}, which this node does not hold"))?;
        let inbox = inbox.clone();
        let (connection, acks, open_links) = (connection.clone(), Arc::clone(&acks), Arc::clone(&open_links));
        tokio::spawn(async move {
            let mut payload = Vec::new();
            while let Ok(Some(header)) = omnarc_network::read_frame(&mut stream, &mut payload).await {
                let frame = Inbound::Frame { session, shard, header, payload: std::mem::take(&mut payload) };
                if inbox.send(frame).await.is_err() {
                    break;
                }
                if header.flags.contains(Flags::MIRROR) {
                    // the copy is in this node's queue, ahead of anything that can make it live: the node that
                    // mirrored it may answer now
                    let mut stream = acks.lock().await;
                    if stream.is_none() {
                        *stream = connection.open_uni().await.ok();
                    }
                    if let Some(stream) = stream.as_mut() {
                        let mut ack = header.request_id.to_le_bytes().to_vec();
                        ack.extend_from_slice(&(header.position_start + u32::from(header.token_count)).to_le_bytes());
                        let _ = stream.write_all(&ack).await;
                    }
                }
            }
            end(connection, inbox, open_links).await;
        });
    }
    Ok(connection)
}

/// Runs a node that serves the ring until it is stopped: every session starts when the previous node connects and
/// ends when it leaves.
///
/// # Errors
/// The engine or the model cannot be opened, the address cannot be bound, or a link breaks inside a session.
pub async fn serve(config: Config) -> Result<(), BoxError> {
    let net = Net::own(&config).await?;
    serve_with(config, net).await
}

/// [`serve`] on connections that come through `net`.
///
/// # Errors
/// As [`serve`].
pub async fn serve_with(config: Config, network: Net) -> Result<(), BoxError> {
    let config = Arc::new(config);
    let node = {
        let config = Arc::clone(&config);
        Arc::new(tokio::task::spawn_blocking(move || open_node(&config)).await??)
    };
    let network = Arc::new(network);
    let endpoint = network.endpoint().clone();
    // every connection is a session, taken whenever it comes; all of them share one queue, so a standby runs the
    // copies it was sent and the step resent to it in the order they reached it (protocol/tla/Takeover.tla)
    let (tx, mut inbox) = mpsc::channel(256);
    let accepting = {
        let (network, config, node) = (Arc::clone(&network), Arc::clone(&config), Arc::clone(&node));
        n0_future::task::AbortOnDropHandle::new(tokio::spawn(async move {
            for session in 1u64.. {
                match accept_prev(&network, &config, &node, session, tx.clone()).await {
                    Ok(_) => eprintln!("omnarc: ring session started"),
                    Err(e) => {
                        eprintln!("omnarc: no more sessions ({e})");
                        return;
                    }
                }
            }
        }))
    };
    let mut sessions: HashMap<u64, Session> = HashMap::new();
    // requests this node runs live: a copy of one still on its way is in its cache already
    let mut live: HashSet<u64> = HashSet::new();
    while let Some(message) = inbox.recv().await {
        match message {
            Inbound::Closed { session, graceful } => {
                let Some(ended) = sessions.remove(&session) else { continue };
                // a session that ran its requests, or ended in good order, frees their caches; one whose source
                // went away keeps them for the node that carries the answer on
                if ended.live || graceful {
                    node.drop_requests(&ended.requests).await;
                    for request in &ended.requests {
                        live.remove(request);
                    }
                }
                ended.close();
                eprintln!("omnarc: ring session ended");
            }
            Inbound::Frame { session, shard, header, payload } => {
                let state = sessions.entry(session).or_default();
                let frame = Arrival { shard, header, payload };
                if let Err(e) = step(&endpoint, &config, &node, &mut live, state, frame).await {
                    eprintln!("omnarc: session {session} stopped: {e}");
                    if let Some(failed) = sessions.remove(&session) {
                        failed.close();
                    }
                }
            }
        }
    }
    drop(accepting);
    Ok(())
}

/// One frame for one of a node's windows.
struct Arrival {
    shard: usize,
    header: FrameHeader,
    payload: Vec<u8>,
}

/// What a node keeps for one session: where its frames go on to, the standby they are copied to, the requests seen.
#[derive(Default)]
struct Session {
    started: bool,
    next: Option<Connection>,
    links: HashMap<u32, SendStream>,
    replica: Option<Connection>,
    mirrors: HashMap<u32, SendStream>,
    acks: Option<Acks>,
    requests: HashSet<u64>,
    live: bool,
}

impl Session {
    fn close(self) {
        if let Some(replica) = self.replica {
            replica.close(0u32.into(), b"session over");
        }
        if let Some(next) = self.next {
            next.close(0u32.into(), b"session over");
        }
    }
}

// Runs one frame of a session on this node and sends its output on.
async fn step(
    endpoint: &Endpoint,
    config: &Config,
    node: &Node,
    live: &mut HashSet<u64>,
    s: &mut Session,
    frame: Arrival,
) -> Result<(), BoxError> {
    let Arrival { shard, header, payload } = frame;
    s.requests.insert(header.request_id);
    let mirrored = header.flags.contains(Flags::MIRROR);
    if mirrored && live.contains(&header.request_id) {
        // a copy still on its way when this replica became the live node: the resent step that made it live is
        // already in the cache, so running the copy would append that step twice
        return Ok(());
    }
    if !s.started {
        s.started = true;
        if !mirrored {
            (s.replica, s.mirrors, s.acks) = connect_replica(endpoint, config, node).await;
        }
    }
    if !mirrored {
        live.insert(header.request_id);
        s.live = true;
        // the replica gets this node's input, not its output: running the same input is what keeps its KV cache
        // identical (EN-17)
        let mut copy = header;
        copy.flags = header.flags.union(Flags::MIRROR);
        mirror(&mut s.mirrors, node.shards[shard].layers.start, &copy, &payload).await;
    }
    if header.flags.contains(Flags::TOKENS) {
        // the replica of the layer-0 window runs the tokens themselves
        if header.flags.contains(Flags::DRAFT) {
            node.truncate(shard, header.request_id, header.position_start).await;
        }
        let tokens = payload.as_chunks::<4>().0.iter().map(|b| i32::from_le_bytes(*b)).collect();
        node.run_tokens(shard, header.request_id, header.position_start, tokens).await?;
        return Ok(());
    }
    let output = node.run(shard, &header, payload).await?;
    if mirrored {
        // a replica keeps its cache in step and forwards nothing
        return Ok(());
    }
    let info = node.shards[shard].clone();
    let count = u32::from(header.token_count);
    let to_logits = info.layers.end == info.model.layer_count;
    let mut frame = node.frame(header.request_id, header.step, header.position_start, count, to_logits)?;
    // a speculative step stays one all the way around the ring
    if header.flags.contains(Flags::DRAFT) {
        frame.flags = frame.flags.union(Flags::DRAFT);
    }
    // the answer goes on only once the standby holds its copy: otherwise a crash now could leave the standby with a
    // hole in its cache (protocol/tla/Takeover.tla)
    confirmed(&mut s.acks, &mut s.mirrors, header.request_id, header.position_start + count).await;
    if s.links.is_empty() {
        let (connection, fresh) = connect_next(endpoint, config, node).await?;
        s.links = fresh;
        s.next = Some(connection);
        if config.mirror_only {
            eprintln!("omnarc: this replica is now the live node for its windows");
        }
    }
    forward(endpoint, config, node, &mut s.next, &mut s.links, info.layers.end, &frame, &output).await
}

/// The head of a ring: it holds layer 0 and the output head, and answers prompts by sending them around the ring.
/// It keeps its windows loaded between prompts; each prompt is one session of the ring.
pub struct Head {
    node: Arc<Node>,
    net: Arc<Net>,
    config: Arc<Config>,
    // where the frames coming back around the ring go: each prompt being answered, by its request
    routes: Routes,
    _accepting: n0_future::task::AbortOnDropHandle<()>,
    _routing: n0_future::task::AbortOnDropHandle<()>,
}

type Routes = Arc<Mutex<HashMap<u64, mpsc::Sender<Arrival>>>>;

/// Request ids of this process: prompts answered at once each get their own.
static NEXT_REQUEST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn new_request() -> u64 {
    u64::from(std::process::id()) << 32 | NEXT_REQUEST.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

// Takes a prompt's route off the table when its answer ends, however it ends.
struct Route<'a> {
    routes: &'a Routes,
    request: u64,
}

impl Drop for Route<'_> {
    fn drop(&mut self) {
        self.routes.lock().expect("routes").remove(&self.request);
    }
}

impl fmt::Debug for Head {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Head").field("windows", &self.config.windows).finish_non_exhaustive()
    }
}

impl Head {
    /// Loads the head's windows.
    ///
    /// # Errors
    /// The engine or the model cannot be opened, or the windows lack layer 0 or the output head.
    ///
    /// # Panics
    /// If a task panicked while it held the table of prompts being answered.
    pub async fn open(config: Config, net: Net) -> Result<Self, BoxError> {
        let config = Arc::new(config);
        let node = {
            let config = Arc::clone(&config);
            tokio::task::spawn_blocking(move || open_node(&config)).await??
        };
        if !node.shards.iter().any(ShardInfo::takes_tokens) {
            return Err("the head holds no window at layer 0".into());
        }
        if !node.shards.iter().any(ShardInfo::gives_logits) {
            return Err("the head must hold the output head too (layer N, e.g. N:N+1)".into());
        }
        let (node, net) = (Arc::new(node), Arc::new(net));
        // the ring brings every prompt's frames back on connections of their own; each frame goes to the prompt
        // its request belongs to, so several prompts go around the ring at once
        let routes: Routes = Arc::default();
        let (tx, mut queue) = mpsc::channel(256);
        let accepting = {
            let (net, config, node) = (Arc::clone(&net), Arc::clone(&config), Arc::clone(&node));
            n0_future::task::AbortOnDropHandle::new(tokio::spawn(async move {
                for session in 1u64.. {
                    if accept_prev(&net, &config, &node, session, tx.clone()).await.is_err() {
                        return;
                    }
                }
            }))
        };
        let routing = {
            let routes = Arc::clone(&routes);
            n0_future::task::AbortOnDropHandle::new(tokio::spawn(async move {
                while let Some(message) = queue.recv().await {
                    if let Inbound::Frame { shard, header, payload, .. } = message {
                        let route = routes.lock().expect("routes").get(&header.request_id).cloned();
                        if let Some(route) = route {
                            let _ = route.send(Arrival { shard, header, payload }).await;
                        }
                    }
                }
            }))
        };
        Ok(Self { node, net, config, routes, _accepting: accepting, _routing: routing })
    }

    /// Answers `prompt` with at most `max_tokens` tokens, handing every piece of text to `on_piece` as it comes.
    ///
    /// # Errors
    /// The engine fails or the ring breaks.
    ///
    /// # Panics
    /// If a task panicked while it held the table of prompts being answered.
    // one loop, because every step of an answer shares the ring's connections, mirrors and the last frame sent
    #[allow(clippy::too_many_lines)]
    pub async fn ask(
        &self,
        prompt: &str,
        max_tokens: u32,
        params: SamplerParams,
        mut on_piece: impl FnMut(&str),
    ) -> Result<Report, BoxError> {
        let (node, config, endpoint) = (&*self.node, &*self.config, self.net.endpoint());
        let prompt = prompt.to_owned();
        let first =
            node.shards.iter().position(ShardInfo::takes_tokens).ok_or("the head holds no window at layer 0")?;
        if node.shards[first].gives_logits() {
            return self.ask_alone(first, prompt, max_tokens, params, on_piece).await;
        }
        let (connection, mut links) = connect_next(endpoint, config, node).await?;
        let mut next = Some(connection);
        // the head's own standby is not waited for: nobody takes over the head's sampling
        let (replica, mut mirrors, _) = connect_replica(endpoint, config, node).await;
        let request = new_request();
        let (tx, mut inbox) = mpsc::channel(256);
        self.routes.lock().expect("routes").insert(request, tx);
        let _route = Route { routes: &self.routes, request };
        // every step sent whose logits have not come back: the prompt goes out in chunks without waiting, so a
        // device that leaves may take several with it, and each is resent to its standby in order
        let mut inflight: std::collections::VecDeque<(u32, FrameHeader, Vec<u8>)> = std::collections::VecDeque::new();

        let shard_id = node.shards[first].id;
        let tokens = with_engine(&node.engine, move |engine| engine.tokenize(shard_id, &prompt, true)).await?;
        let sampler = with_engine(&node.engine, move |engine| engine.sampler_open(params)).await?;
        let prompt_len = u32::try_from(tokens.len())?;
        let start = Instant::now();

        // prefill: the prompt goes around the ring in chunks, one after another
        let first_end = node.shards[first].layers.end;
        let first_start = node.shards[first].layers.start;
        for (i, chunk) in tokens.chunks(config.chunk as usize).enumerate() {
            let position = u32::try_from(i * config.chunk as usize)?;
            let count = u32::try_from(chunk.len())?;
            let bytes: Vec<u8> = chunk.iter().flat_map(|t| t.to_le_bytes()).collect();
            mirror(&mut mirrors, first_start, &node.token_frame(request, 0, position, count)?, &bytes).await;
            let output = node.run_tokens(first, request, position, chunk.to_vec()).await?;
            let frame = node.frame(request, 0, position, count, false)?;
            inflight.push_back((first_end, frame, output.clone()));
            forward(endpoint, config, node, &mut next, &mut links, first_end, &frame, &output).await?;
        }

        let mut generated = Vec::new();
        let mut text = String::new();
        let mut times = Vec::new();
        // what was said so far, where the drafts come from; and the drafts the step on its way carries
        let mut context = tokens.clone();
        let mut drafts: Vec<i32> = Vec::new();
        loop {
            let message = tokio::select! {
                message = tokio::time::timeout(STEP_PATIENCE, inbox.recv()) => {
                    // a ring that stopped answering without closing is an error, not a wait without end
                    message.map_err(|_| format!("no answer from the ring for {} s", STEP_PATIENCE.as_secs()))?
                }
                () = closed(next.as_ref()) => {
                    // the device after this one has gone: its window goes to the replica, the steps it was working on
                    // are resent there, and the frames that come back now come from the replica (NO-43)
                    if inflight.is_empty() {
                        return Err("the next node left before anything was sent".into());
                    }
                    for (key, frame, payload) in &inflight {
                        let mut replay = *frame;
                        replay.flags = frame.flags.union(Flags::REPLAY);
                        forward(endpoint, config, node, &mut next, &mut links, *key, &replay, payload).await?;
                    }
                    continue;
                }
            };
            let Some(Arrival { shard, header, payload }) = message else { break };
            let info = node.shards[shard].clone();
            let count = u32::from(header.token_count);
            let mut copy = header;
            copy.flags = header.flags.union(Flags::MIRROR);
            mirror(&mut mirrors, info.layers.start, &copy, &payload).await;
            let output = node.run(shard, &header, payload).await?;
            if !info.gives_logits() {
                let to_logits = info.layers.end == info.model.layer_count;
                let mut frame = node.frame(request, header.step, header.position_start, count, to_logits)?;
                if header.flags.contains(Flags::DRAFT) {
                    frame.flags = frame.flags.union(Flags::DRAFT);
                }
                forward(endpoint, config, node, &mut next, &mut links, info.layers.end, &frame, &output).await?;
                continue;
            }
            // these logits answer the step that started at this position, and every step before it
            while inflight.front().is_some_and(|(_, f, _)| f.position_start <= header.position_start) {
                inflight.pop_front();
            }
            // logits: only the end of the prompt and every decoded token pick a next token
            let position = header.position_start + count;
            if position < prompt_len {
                continue;
            }
            let vocab = node.vocab_size;
            let picked = if header.flags.contains(Flags::DRAFT) {
                // the logits after the token sent and after each draft: the drafts hold while they are what the
                // model picks itself, and the first it would not pick is replaced by its own choice
                accepted(&argmax_rows(&output, vocab), &drafts)
            } else {
                vec![with_engine(&node.engine, move |engine| engine.sample(sampler, vocab)).await?]
            };
            let mut finished = false;
            for &token in &picked {
                times.push(start.elapsed());
                let (piece, is_end) =
                    with_engine(&node.engine, move |engine| engine.token_text(shard_id, token)).await?;
                generated.push(token);
                context.push(token);
                text.push_str(&piece);
                on_piece(&piece);
                if is_end || generated.len() >= max_tokens as usize {
                    finished = true;
                    break;
                }
            }
            if finished {
                break;
            }
            // the position of the last token picked: drafts the ring computed past it are cut from every cache
            let advanced = if header.flags.contains(Flags::DRAFT) { u32::try_from(picked.len())? } else { count };
            let position = header.position_start + advanced;
            let token = *picked.last().ok_or("the ring returned no logits")?;
            // a check against greedy picks changes no answer; a sampled one would, so drafts are for greedy only
            let speculative = config.draft > 0 && params.temperature <= 0.0;
            drafts = if speculative { draft(&context, config.draft as usize) } else { Vec::new() };
            let step_tokens: Vec<i32> = std::iter::once(token).chain(drafts.iter().copied()).collect();
            let n = u32::try_from(step_tokens.len())?;
            let step = u32::try_from(generated.len())?;
            let mut tokens_frame = node.token_frame(request, step, position, n)?;
            if speculative {
                tokens_frame.flags = tokens_frame.flags.union(Flags::DRAFT);
                node.truncate(first, request, position).await;
            }
            let bytes: Vec<u8> = step_tokens.iter().flat_map(|t| t.to_le_bytes()).collect();
            mirror(&mut mirrors, first_start, &tokens_frame, &bytes).await;
            let output = node.run_tokens(first, request, position, step_tokens).await?;
            let mut frame = node.frame(request, step, position, n, false)?;
            if speculative {
                frame.flags = frame.flags.union(Flags::DRAFT);
            }
            inflight.push_back((first_end, frame, output.clone()));
            forward(endpoint, config, node, &mut next, &mut links, first_end, &frame, &output).await?;
        }

        with_engine(&node.engine, move |engine| engine.sampler_close(sampler)).await?;
        node.drop_requests(&HashSet::from([request])).await;
        if let Some(replica) = replica {
            replica.close(0u32.into(), b"done");
        }
        if let Some(next) = next {
            next.close(0u32.into(), b"done");
        }

        let ttft = times.first().copied().unwrap_or_default();
        let tpot = (times.len() > 1).then(|| {
            let decode = times[times.len() - 1].saturating_sub(times[0]);
            decode / u32::try_from(times.len() - 1).unwrap_or(u32::MAX)
        });
        let longest_pause = times.windows(2).map(|w| w[1].saturating_sub(w[0])).max().unwrap_or_default();
        Ok(Report { prompt_tokens: tokens.len(), generated, text, ttft, tpot, longest_pause })
    }
}

impl Head {
    // A model that fits on one device (R6): no ring, the head runs every layer itself.
    async fn ask_alone(
        &self,
        shard: usize,
        prompt: String,
        max_tokens: u32,
        params: SamplerParams,
        mut on_piece: impl FnMut(&str),
    ) -> Result<Report, BoxError> {
        let node = &self.node;
        let info = node.shards[shard].clone();
        let shard_id = info.id;
        let tokens = with_engine(&node.engine, move |engine| engine.tokenize(shard_id, &prompt, true)).await?;
        let sampler = with_engine(&node.engine, move |engine| engine.sampler_open(params)).await?;
        let request = new_request();
        let start = Instant::now();
        let (mut generated, mut text, mut times) = (Vec::new(), String::new(), Vec::new());
        let mut position = 0u32;
        let mut input = tokens.clone();
        let chunk = self.config.chunk as usize;
        loop {
            // the prompt goes in chunks; every token after it, one at a time
            for part in input.chunks(chunk.max(1)) {
                let count = u32::try_from(part.len())?;
                let started = Instant::now();
                let (info, part) = (info.clone(), part.to_vec());
                with_engine(&node.engine, move |engine| {
                    let input = Input::Tokens(&part);
                    let step = Step { request, position, input, all_logits: false, output: DType::F32 };
                    engine.run(&info, &step).map(|_| ())
                })
                .await?;
                tally(count, started);
                position += count;
            }
            let vocab = node.vocab_size;
            let token = with_engine(&node.engine, move |engine| engine.sample(sampler, vocab)).await?;
            times.push(start.elapsed());
            let (piece, is_end) = with_engine(&node.engine, move |engine| engine.token_text(shard_id, token)).await?;
            generated.push(token);
            text.push_str(&piece);
            on_piece(&piece);
            if is_end || generated.len() >= max_tokens as usize {
                break;
            }
            input = vec![token];
        }
        with_engine(&node.engine, move |engine| engine.sampler_close(sampler)).await?;
        node.drop_requests(&HashSet::from([request])).await;
        let ttft = times.first().copied().unwrap_or_default();
        let tpot = (times.len() > 1).then(|| {
            let decode = times[times.len() - 1].saturating_sub(times[0]);
            decode / u32::try_from(times.len() - 1).unwrap_or(u32::MAX)
        });
        let longest_pause = times.windows(2).map(|w| w[1].saturating_sub(w[0])).max().unwrap_or_default();
        Ok(Report { prompt_tokens: tokens.len(), generated, text, ttft, tpot, longest_pause })
    }
}

/// Runs the head: sends the prompt around the ring, samples every next token from the logits that come back, and
/// reports the timings.
///
/// # Errors
/// There is no prompt, the head lacks layer 0 or the output head, the engine fails, or the ring breaks.
pub async fn generate(config: Config) -> Result<Report, BoxError> {
    let prompt = config.prompt.clone().ok_or("the head needs a prompt")?;
    let generate = config.generate;
    let net = Net::own(&config).await?;
    let head = Head::open(config, net).await?;
    let mut stdout = std::io::stdout();
    let report = head
        .ask(&prompt, generate, SamplerParams::default(), |piece| {
            print!("{piece}");
            let _ = stdout.flush();
        })
        .await?;
    println!();
    head.net.endpoint().close().await;
    Ok(report)
}

async fn bind(config: &Config) -> Result<Endpoint, BoxError> {
    let key = omnarc_network::load_or_create_key(&config.key)?;
    let endpoint = omnarc_network::bind(key, config.listen).await?;
    eprintln!("omnarc: node {} listening on {}", endpoint.id(), config.listen);
    Ok(endpoint)
}

/// The token each row of f32 logits picks greedily: the first of the highest, as the engine's greedy sampler does.
fn argmax_rows(logits: &[u8], vocab: u32) -> Vec<i32> {
    let vocab = vocab as usize;
    let values: Vec<f32> = logits.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect();
    values
        .chunks_exact(vocab.max(1))
        .map(|row| {
            let mut best = 0;
            for (i, &v) in row.iter().enumerate() {
                if v > row[best] {
                    best = i;
                }
            }
            i32::try_from(best).unwrap_or(0)
        })
        .collect()
}

/// The tokens a speculative step settles: each draft holds while it is what the model picks itself (`rows`, its
/// greedy pick after the token sent and after each draft), and the first it would not pick is replaced by the model's
/// own choice -- so a step always gains at least one token.
#[must_use]
pub fn accepted(rows: &[i32], drafts: &[i32]) -> Vec<i32> {
    let mut picked = Vec::new();
    for (i, &best) in rows.iter().enumerate() {
        picked.push(best);
        if drafts.get(i) != Some(&best) {
            break;
        }
    }
    picked
}

/// Up to `k` tokens guessed from what was said before (prompt lookup): the tokens that followed the last place the
/// latest few tokens appeared, longest match first.
#[must_use]
pub fn draft(context: &[i32], k: usize) -> Vec<i32> {
    for n in (1..=3).rev() {
        if context.len() <= n {
            continue;
        }
        let tail = &context[context.len() - n..];
        for start in (0..context.len() - n).rev() {
            if &context[start..start + n] == tail {
                let from = start + n;
                return context[from..(from + k).min(context.len())].to_vec();
            }
        }
    }
    Vec::new()
}

/// Where this device keeps its node key.
#[must_use]
pub fn default_key_path() -> PathBuf {
    data_dir().join("omnarc").join("node.key")
}

fn data_dir() -> PathBuf {
    let env = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    if cfg!(windows) {
        env("APPDATA").unwrap_or_else(|| PathBuf::from("."))
    } else if cfg!(target_os = "macos") {
        env("HOME").map_or_else(|| PathBuf::from("."), |h| h.join("Library/Application Support"))
    } else {
        env("XDG_DATA_HOME")
            .or_else(|| env("HOME").map(|h| h.join(".local/share")))
            .unwrap_or_else(|| PathBuf::from("."))
    }
}

/// The engine executable next to this program, unless given.
#[must_use]
pub fn default_engine_path() -> PathBuf {
    // the Android app ships the engine as a library of its own and says where
    if let Some(path) = std::env::var_os("OMNARC_ENGINE") {
        return PathBuf::from(path);
    }
    let name = if cfg!(windows) { "omnarc-engine.exe" } else { "omnarc-engine" };
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(name)))
        .unwrap_or_else(|| PathBuf::from(name))
}

/// Prints this node's id, creating its key if needed.
///
/// # Errors
/// The key file cannot be read or written.
pub fn node_id(key: &Path) -> Result<EndpointId, BoxError> {
    Ok(omnarc_network::load_or_create_key(key)?.public())
}

#[cfg(test)]
mod tests {
    use super::{Window, accepted, draft};

    #[test]
    fn drafts_come_from_what_was_said_and_hold_while_the_model_agrees() {
        // "a b c a b" -> the last "a b" was followed by "c a b" before
        assert_eq!(draft(&[1, 2, 3, 1, 2], 3), vec![3, 1, 2]);
        assert_eq!(draft(&[1, 2, 3], 4), Vec::<i32>::new(), "nothing repeats yet");
        // the model picks 5 after the token sent, agreeing with the first draft, then 9 where the draft said 6
        assert_eq!(accepted(&[5, 9, 7], &[5, 6]), vec![5, 9]);
        assert_eq!(accepted(&[5, 6, 7], &[5, 6]), vec![5, 6, 7], "all drafts hold, and the step gains one more");
        assert_eq!(accepted(&[4], &[]), vec![4]);
    }

    #[test]
    fn windows_parse() {
        assert_eq!("0:20".parse::<Window>().unwrap().0, 0..20);
        assert_eq!(" 80 : 81 ".parse::<Window>().unwrap().0, 80..81);
        assert!("20:20".parse::<Window>().is_err());
        assert!("20".parse::<Window>().is_err());
        assert_eq!(Window(3..9).to_string(), "3:9");
    }
}
