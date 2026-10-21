//! The model the cluster runs (SPEC NO-41..NO-44, NO-51, NO-52). A computer of the cluster places it within what
//! every member offers and the computers agree on the plan through Raft; phones and browsers hear it over gossip.
//! Every member then carries out its own part: it fetches the bytes of its windows, loads them and takes its place in
//! the ring. The head answers prompts, and a prompt asked anywhere in the cluster is passed on to it.

#[cfg(feature = "coordinator")]
use std::collections::HashSet;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;
#[cfg(feature = "coordinator")]
use std::time::Duration;

use n0_future::boxed::BoxStream;
use n0_future::task::AbortOnDropHandle;
use omnarc_engine_host::{Backend, DType};
use omnarc_network::protocol::{AcceptError, ProtocolHandler};
use omnarc_network::{Connection, EndpointId, Peer};
use omnarc_protocol::v1;
use omnarc_store::{Hash, Model, Window};
use prost::Message;
use tokio::sync::mpsc;

use super::Shared;
#[cfg(feature = "coordinator")]
use super::{Command, node_id};
use crate::ring::{self, BoxError, Head, Net};

/// The protocol that carries a prompt to the head and the answer back.
pub const ASK_ALPN: &[u8] = b"omnarc/ask/1";

/// How often the computers' leader repeats the plan over gossip, for members that missed it.
#[cfg(feature = "coordinator")]
pub(super) const SPREAD_EVERY: Duration = Duration::from_secs(5);

/// What each stage keeps aside for the engine's compute buffers, besides its windows.
pub const ENGINE_RESERVE: u64 = 192 << 20;

/// The context the KV caches are sized for when the request names none.
pub const DEFAULT_CONTEXT: u32 = 4096;

/// A member gone quiet this long, in milliseconds, has left as far as the plan is concerned (NO-43).
#[cfg(feature = "coordinator")]
const GONE_AFTER_MS: i64 = 5_000;

/// What this device runs now.
#[derive(Default)]
pub(super) struct Running {
    version: u64,
    // the part this device plays and the model it plays it in: a new plan that leaves both as they are changes nothing
    role: Option<Role>,
    model: Vec<u8>,
    head: Option<Arc<Head>>,
    // a stage or a replica that serves the ring until the plan changes
    serving: Option<AbortOnDropHandle<()>>,
}

impl std::fmt::Debug for Running {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Running")
            .field("version", &self.version)
            .field("head", &self.head.is_some())
            .field("serving", &self.serving.is_some())
            .field("role", &self.role)
            .finish_non_exhaustive()
    }
}

/// This device's part in a plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Role {
    pub windows: Vec<Range<u32>>,
    /// It answers prompts: it holds layer 0 and the output.
    pub head: bool,
    /// It stands by for another device's windows and only keeps its cache in step (EN-17).
    pub mirror_only: bool,
    pub next: Option<Vec<u8>>,
    pub next_replica: Option<Vec<u8>>,
    pub prev: Vec<Vec<u8>>,
    pub replica: Option<Vec<u8>>,
}

fn key(node: Option<&v1::NodeId>) -> Vec<u8> {
    node.map(|n| n.ed25519_public_key.clone()).unwrap_or_default()
}

/// The part `me` plays in `plan`, if any.
#[must_use]
pub fn role(plan: &v1::PlacementPlan, me: &[u8]) -> Option<Role> {
    let stages = &plan.stages;
    let n = stages.len();
    if n == 0 {
        return None;
    }
    let layers = plan.model.as_ref().map_or(0, |m| m.layer_count);
    let replica_of = |i: usize| {
        let s = &stages[i];
        plan.replicas
            .iter()
            .find(|r| r.layer_begin == s.layer_begin && r.layer_end == s.layer_end)
            .map(|r| key(r.node.as_ref()))
    };
    let windows = |i: usize| {
        let s = &stages[i];
        if n == 1 {
            // one device holds it all: one window through the output
            std::iter::once(0..layers + 1).collect()
        } else if i == 0 {
            vec![s.layer_begin..s.layer_end, layers..layers + 1]
        } else {
            std::iter::once(s.layer_begin..s.layer_end).collect()
        }
    };
    let node = |i: usize| key(stages[i].node.as_ref());
    let before = |i: usize| (i + n - 1) % n;
    let after = |i: usize| (i + 1) % n;

    if let Some(i) = stages.iter().position(|s| key(s.node.as_ref()) == me) {
        if n == 1 {
            return Some(Role {
                windows: windows(0),
                head: true,
                mirror_only: false,
                next: None,
                next_replica: None,
                prev: Vec::new(),
                replica: None,
            });
        }
        let mut prev = vec![node(before(i))];
        prev.extend(replica_of(before(i)));
        return Some(Role {
            windows: windows(i),
            head: i == 0,
            mirror_only: false,
            next: Some(node(after(i))),
            next_replica: replica_of(after(i)),
            prev,
            replica: replica_of(i),
        });
    }
    let replica = plan.replicas.iter().find(|r| key(r.node.as_ref()) == me)?;
    let i = stages.iter().position(|s| s.layer_begin == replica.layer_begin && s.layer_end == replica.layer_end)?;
    if n == 1 {
        // a whole copy of a model that fits on one device answers prompts of its own (R6)
        return Some(Role {
            windows: windows(0),
            head: true,
            mirror_only: false,
            next: None,
            next_replica: None,
            prev: Vec::new(),
            replica: None,
        });
    }
    let mut prev = vec![node(i), node(before(i))];
    prev.extend(replica_of(before(i)));
    prev.retain(|p| p.as_slice() != me);
    prev.dedup();
    Some(Role {
        windows: windows(i),
        head: false,
        mirror_only: true,
        next: Some(node(after(i))),
        next_replica: replica_of(after(i)),
        prev,
        replica: None,
    })
}

fn endpoint_id(bytes: &[u8]) -> Option<EndpointId> {
    EndpointId::from_bytes(&<[u8; 32]>::try_from(bytes).ok()?).ok()
}

/// Publishes a plan this device heard of, if it is newer than the one it knows.
pub(super) fn adopt(shared: &Shared, plan: v1::PlacementPlan) {
    shared.plan.send_if_modified(|current| {
        let newer = current.as_ref().is_none_or(|c| plan.version > c.version);
        if newer {
            let _ = shared.events.send(super::Event::PlanChanged(plan.clone()));
            *current = Some(plan);
        }
        newer
    });
}

/// Carries out every plan the cluster agrees on, for as long as the service runs.
pub(super) async fn carry_out(shared: Arc<Shared>) {
    let mut plans = shared.plan.subscribe();
    loop {
        let plan = plans.borrow_and_update().clone();
        if let Some(plan) = plan {
            let version = plan.version;
            if let Err(e) = apply(&shared, &plan).await {
                eprintln!("omnarc: plan {version} cannot run here: {e}");
                // nothing half-started stays behind
                let mut running = shared.running.lock().await;
                *running = Running { version, ..Running::default() };
                shared.stages.lock().expect("stages").clear();
            }
        }
        if plans.changed().await.is_err() {
            return;
        }
    }
}

async fn apply(shared: &Arc<Shared>, plan: &v1::PlacementPlan) -> Result<(), BoxError> {
    let me = shared.key.public();
    let role = role(plan, me.as_bytes());
    let model_hash = plan.model.as_ref().map(|m| m.blake3.clone()).unwrap_or_default();
    let mut running = shared.running.lock().await;
    if role.is_some() && running.role == role && running.model == model_hash {
        // the same windows between the same neighbors: the ring goes on as it is
        running.version = plan.version;
        for load in shared.stages.lock().expect("stages").iter_mut() {
            load.plan_version = plan.version;
        }
        return Ok(());
    }
    // what ran under the plan before stops first: its windows may not be this device's any more
    *running = Running { version: plan.version, ..Running::default() };
    *shared.tensors.lock().expect("tensors") = None;
    shared.stages.lock().expect("stages").clear();
    let Some(role) = role else { return Ok(()) };

    let model_ref = plan.model.as_ref().ok_or("the plan names no model")?;
    let hash = Hash::from_bytes(<[u8; 32]>::try_from(model_ref.blake3.as_slice()).map_err(|_| "a model hash")?);
    let windows: Vec<Window> = role.windows.iter().map(|w| Window { begin: w.start, end: w.end }).collect();
    let model = fetch(shared, plan, hash, &windows).await?;
    let dir = shared.data.join("run");
    std::fs::create_dir_all(&dir)?;
    let file = dir.join(format!("{}.gguf", hash.to_hex()));
    shared.models.export_windows(&model, &windows, &file).await?;

    let peer = |bytes: &Vec<u8>| -> Result<Peer, BoxError> {
        let id = endpoint_id(bytes).ok_or("a node id in the plan")?;
        Ok(Peer { id, addr: direct_addr(shared, id) })
    };
    let next = match &role.next {
        Some(next) => peer(next)?,
        None => Peer { id: me, addr: ([0, 0, 0, 0], 0).into() },
    };
    let config = ring::Config {
        model: file,
        windows: role.windows.iter().cloned().map(ring::Window).collect(),
        listen: ([0, 0, 0, 0], 0).into(),
        next,
        next_replica: role.next_replica.as_ref().map(peer).transpose()?,
        prev: role.prev.iter().filter_map(|p| endpoint_id(p)).collect(),
        key: PathBuf::new(),
        engine: ring::default_engine_path(),
        backend: Backend::Unspecified,
        max_tokens: if plan.context == 0 { DEFAULT_CONTEXT } else { plan.context },
        chunk: 256,
        kv: omnarc_engine_host::KvType::from_name(&plan.kv_cache).unwrap_or_default(),
        wire: DType::F16,
        mirror_only: role.mirror_only,
        replica: role.replica.as_ref().map(peer).transpose()?,
        threads: crate::budget::offered_cores(
            shared.budget.lock().expect("budget").as_ref(),
            u32::try_from(std::thread::available_parallelism().map_or(1, std::num::NonZero::get)).unwrap_or(1),
        ),
        draft: 4,
        prompt: None,
        generate: 0,
    };
    let (tx, rx) = mpsc::channel(8);
    *shared.tensors.lock().expect("tensors") = Some(tx);
    let network = Net::shared(shared.endpoint.clone(), rx);
    if role.head {
        running.head = Some(Arc::new(Head::open(config, network).await?));
    } else {
        let version = plan.version;
        running.serving = Some(AbortOnDropHandle::new(tokio::spawn(async move {
            if let Err(e) = ring::serve_with(config, network).await {
                eprintln!("omnarc: the ring of plan {version} stopped here: {e}");
            }
        })));
    }
    let loads = role
        .windows
        .iter()
        .map(|w| v1::StageLoad {
            plan_version: plan.version,
            layer_begin: w.start,
            layer_end: w.end,
            ..Default::default()
        })
        .collect();
    *shared.stages.lock().expect("stages") = loads;
    running.role = Some(role.clone());
    running.model = model_hash;
    eprintln!(
        "omnarc: plan {}: {} {:?}",
        plan.version,
        if role.head {
            "head"
        } else if role.mirror_only {
            "replica"
        } else {
            "stage"
        },
        role.windows
    );
    Ok(())
}

// The windows' bytes on this device: what is missing comes from the plan's source, and only what is missing.
async fn fetch(shared: &Shared, plan: &v1::PlacementPlan, hash: Hash, windows: &[Window]) -> Result<Model, BoxError> {
    let mut model = shared.models.model(&hash.to_hex()).await.ok();
    for &window in windows {
        if let Some(have) = &model
            && shared.models.missing(have, window).await? == 0
        {
            continue;
        }
        let source =
            endpoint_id(&key(plan.source.as_ref())).ok_or("the plan names no member to fetch the model from")?;
        let connection = shared.endpoint.connect(source, omnarc_store::BLOBS_ALPN).await?;
        let name = plan.model.as_ref().map(|m| m.name.as_str()).filter(|n| !n.is_empty());
        eprintln!("omnarc: fetching layers {window} from {}", source.fmt_short());
        let total = match &model {
            Some(have) => shared.models.missing(have, window).await?,
            None => 0,
        };
        let shown = name.unwrap_or("the model").to_owned();
        let progress = |received: u64| {
            *shared.fetching.lock().expect("fetching") =
                Some(v1::ModelFetch { model: shown.clone(), received_bytes: received, total_bytes: total });
        };
        let fetched = shared.models.fetch(&connection, name, hash, window, progress).await;
        *shared.fetching.lock().expect("fetching") = None;
        model = Some(fetched?);
    }
    model.ok_or_else(|| "the model is not on this device".into())
}

fn direct_addr(shared: &Shared, id: EndpointId) -> std::net::SocketAddr {
    shared
        .devices
        .lock()
        .expect("devices")
        .get(&id)
        .and_then(|d| d.direct_addresses.iter().find_map(|a| a.parse().ok()))
        // no address known: the endpoint's lookup finds the member through its relay
        .unwrap_or_else(|| ([0, 0, 0, 0], 0).into())
}

/// Hands the ring the tensor connections members open to this device.
#[derive(Debug, Clone)]
pub(super) struct TensorHandler(pub(super) Arc<std::sync::OnceLock<Arc<Shared>>>);

impl ProtocolHandler for TensorHandler {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let Some(shared) = self.0.get() else { return Ok(()) };
        let tensors = shared.tensors.lock().expect("tensors").clone();
        match tensors {
            Some(tensors) => {
                let _ = tensors.send(connection).await;
            }
            None => connection.close(503u32.into(), b"no ring runs here"),
        }
        Ok(())
    }
}

// A prompt on the wire: the request, then frames of `kind` (0 an answer step, 1 an error) and length.
async fn write_frame(send: &mut omnarc_network::SendStream, kind: u8, bytes: &[u8]) -> Result<(), BoxError> {
    send.write_all(&[kind]).await?;
    send.write_all(&u32::try_from(bytes.len())?.to_le_bytes()).await?;
    send.write_all(bytes).await?;
    Ok(())
}

async fn read_frame(recv: &mut omnarc_network::RecvStream) -> Result<Option<(u8, Vec<u8>)>, BoxError> {
    let mut kind = [0u8; 1];
    if recv.read_exact(&mut kind).await.is_err() {
        return Ok(None);
    }
    let mut len = [0u8; 4];
    recv.read_exact(&mut len).await?;
    let len = u32::from_le_bytes(len) as usize;
    if len > 1 << 20 {
        return Err("a frame larger than a prompt can be".into());
    }
    let mut bytes = vec![0u8; len];
    recv.read_exact(&mut bytes).await?;
    Ok(Some((kind[0], bytes)))
}

/// What an answer streams: its steps, or why it stopped.
pub(super) type Answer = BoxStream<Result<v1::AskResponse, String>>;

/// Asks the model the cluster runs: on this device if it holds a head, or on the head the plan names.
pub(super) fn ask(shared: Arc<Shared>, request: v1::AskRequest) -> Answer {
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let failed = tx.clone();
        if let Err(e) = answer(&shared, request, tx).await {
            let _ = failed.send(Err(e.to_string()));
        }
    });
    Box::pin(n0_future::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|step| (step, rx)) }))
}

async fn answer(
    shared: &Arc<Shared>,
    request: v1::AskRequest,
    tx: mpsc::UnboundedSender<Result<v1::AskResponse, String>>,
) -> Result<(), BoxError> {
    if request.paths > 1 {
        return accuracy(shared, request, &tx).await;
    }
    let head = shared.running.lock().await.head.clone();
    if let Some(head) = head {
        return answer_here(&head, &request, &tx).await;
    }
    let plan = shared.plan.borrow().clone().filter(|p| !p.stages.is_empty()).ok_or("no model runs in the cluster")?;
    let id = route(shared, &plan).ok_or("the plan names no head")?;
    ask_remote(shared, id, request, &tx).await
}

// Passes a prompt to the head `id` and relays its answer.
async fn ask_remote(
    shared: &Arc<Shared>,
    id: EndpointId,
    request: v1::AskRequest,
    tx: &mpsc::UnboundedSender<Result<v1::AskResponse, String>>,
) -> Result<(), BoxError> {
    let connection = shared.endpoint.connect(id, ASK_ALPN).await?;
    let (mut send, mut recv) = connection.open_bi().await?;
    write_frame(&mut send, 0, &request.encode_to_vec()).await?;
    send.finish()?;
    while let Some((kind, bytes)) = read_frame(&mut recv).await? {
        if kind == 1 {
            return Err(String::from_utf8_lossy(&bytes).into_owned().into());
        }
        let step = v1::AskResponse::decode(bytes.as_slice())?;
        let done = matches!(step.step, Some(v1::ask_response::Step::Done(_)));
        if tx.send(Ok(step)).is_err() || done {
            break;
        }
    }
    connection.close(0u32.into(), b"answered");
    Ok(())
}

/// The devices that answer prompts in `plan`: the head, and every whole copy of a model that fits on one device.
#[must_use]
pub fn heads(plan: &v1::PlacementPlan) -> Vec<Vec<u8>> {
    let mut heads: Vec<_> = plan.stages.first().map(|s| key(s.node.as_ref())).into_iter().collect();
    if plan.stages.len() == 1 {
        heads.extend(plan.replicas.iter().map(|r| key(r.node.as_ref())));
    }
    heads
}

// Where a prompt asked on this device goes (NO-42): to the least busy device that answers, as its telemetry says.
fn route(shared: &Shared, plan: &v1::PlacementPlan) -> Option<EndpointId> {
    let telemetry = shared.telemetry.lock().expect("telemetry");
    heads(plan).iter().filter_map(|k| endpoint_id(k)).min_by(|a, b| {
        let load = |id: &EndpointId| telemetry.get(id).map_or(f32::MAX, |t| t.cpu_load);
        load(a).total_cmp(&load(b))
    })
}

async fn answer_here(
    head: &Head,
    request: &v1::AskRequest,
    tx: &mpsc::UnboundedSender<Result<v1::AskResponse, String>>,
) -> Result<(), BoxError> {
    use v1::ask_response::Step;
    let max = if request.max_tokens == 0 { 256 } else { request.max_tokens };
    let params = omnarc_engine_host::SamplerParams {
        temperature: request.temperature.max(0.0),
        seed: request.seed,
        ..omnarc_engine_host::SamplerParams::default()
    };
    let report = head
        .ask(&request.prompt, max, params, |piece| {
            let _ = tx.send(Ok(v1::AskResponse { step: Some(Step::Piece(piece.to_owned())) }));
        })
        .await?;
    #[allow(clippy::cast_possible_truncation)]
    let done = v1::AskDone {
        prompt_tokens: u32::try_from(report.prompt_tokens).unwrap_or(u32::MAX),
        generated_tokens: u32::try_from(report.generated.len()).unwrap_or(u32::MAX),
        ttft_ms: (report.ttft.as_secs_f64() * 1e3) as f32,
        tpot_ms: report.tpot.map_or(0.0, |t| (t.as_secs_f64() * 1e3) as f32),
        paths: 1,
        agreed: 1,
    };
    let _ = tx.send(Ok(v1::AskResponse { step: Some(Step::Done(done)) }));
    Ok(())
}

/// The most paths accuracy mode runs for one question.
const MAX_PATHS: u32 = 8;

// Accuracy mode (EN-23): the question goes several ways at once, each sampled with a seed of its own, on the
// model's whole copies in turn; the answer most paths agree on is given, with how many agreed.
async fn accuracy(
    shared: &Arc<Shared>,
    request: v1::AskRequest,
    tx: &mpsc::UnboundedSender<Result<v1::AskResponse, String>>,
) -> Result<(), BoxError> {
    use v1::ask_response::Step;
    let plan = shared.plan.borrow().clone().filter(|p| !p.stages.is_empty()).ok_or("no model runs in the cluster")?;
    let heads: Vec<EndpointId> = heads(&plan).iter().filter_map(|k| endpoint_id(k)).collect();
    if heads.is_empty() {
        return Err("the plan names no head".into());
    }
    let paths = request.paths.min(MAX_PATHS);
    let temperature = if request.temperature > 0.0 { request.temperature } else { 0.7 };
    let runs = (0..paths).map(|i| {
        let shared = Arc::clone(shared);
        let head = heads[i as usize % heads.len()];
        let path = v1::AskRequest {
            paths: 1,
            temperature,
            seed: request.seed.wrapping_add(u64::from(i) + 1),
            ..request.clone()
        };
        async move { collect(&shared, head, path).await }
    });
    let answers: Vec<(String, Option<v1::AskDone>)> =
        futures_util::future::join_all(runs).await.into_iter().filter_map(Result::ok).collect();
    let (chosen, agreed) = vote(answers.iter().map(|(text, _)| text.as_str())).ok_or("no path answered")?;
    let mut done = answers.iter().find_map(|(_, d)| *d).unwrap_or_default();
    done.paths = u32::try_from(answers.len()).unwrap_or(u32::MAX);
    done.agreed = agreed;
    let _ = tx.send(Ok(v1::AskResponse { step: Some(Step::Piece(chosen.to_owned())) }));
    let _ = tx.send(Ok(v1::AskResponse { step: Some(Step::Done(done)) }));
    Ok(())
}

// One whole answer from the head `id`, this device's own head included.
async fn collect(
    shared: &Arc<Shared>,
    id: EndpointId,
    request: v1::AskRequest,
) -> Result<(String, Option<v1::AskDone>), BoxError> {
    use v1::ask_response::Step;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let local = if id == shared.key.public() { shared.running.lock().await.head.clone() } else { None };
    match local {
        Some(head) => answer_here(&head, &request, &tx).await?,
        None => ask_remote(shared, id, request, &tx).await?,
    }
    drop(tx);
    let (mut text, mut done) = (String::new(), None);
    while let Some(step) = rx.recv().await {
        match step.map_err(BoxError::from)?.step {
            Some(Step::Piece(piece)) => text.push_str(&piece),
            Some(Step::Done(d)) => done = Some(d),
            None => {}
        }
    }
    Ok((text, done))
}

/// The answer most texts agree on, and how many do: texts agree when they end in the same number, or else in the
/// same last line, whatever the spacing and the case. On a tie, the earliest wins.
#[must_use]
pub fn vote<'a>(texts: impl Iterator<Item = &'a str>) -> Option<(&'a str, u32)> {
    let texts: Vec<&str> = texts.collect();
    let keys: Vec<String> = texts.iter().map(|t| answer_key(t)).collect();
    let mut best: Option<(usize, u32)> = None;
    for (i, key) in keys.iter().enumerate() {
        let votes = u32::try_from(keys.iter().filter(|k| *k == key).count()).unwrap_or(u32::MAX);
        if best.is_none_or(|(_, b)| votes > b) {
            best = Some((i, votes));
        }
    }
    best.map(|(i, votes)| (texts[i], votes))
}

fn answer_key(text: &str) -> String {
    // the last number, as a maths answer ends
    let mut number = String::new();
    let mut last = String::new();
    for c in text.chars() {
        if c.is_ascii_digit() || (c == '.' && !number.is_empty()) || (c == '-' && number.is_empty()) {
            number.push(c);
        } else if !number.is_empty() {
            if number.chars().any(|c| c.is_ascii_digit()) {
                number.trim_end_matches('.').clone_into(&mut last);
            }
            number.clear();
        }
    }
    if number.chars().any(|c| c.is_ascii_digit()) {
        number.trim_end_matches('.').clone_into(&mut last);
    }
    if !last.is_empty() {
        return last;
    }
    let line = text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("");
    line.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// Answers the prompts members pass to this device's head.
#[derive(Debug, Clone)]
pub(super) struct AskHandler(pub(super) Arc<std::sync::OnceLock<Arc<Shared>>>);

impl ProtocolHandler for AskHandler {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let Some(shared) = self.0.get().cloned() else { return Ok(()) };
        let Ok((mut send, mut recv)) = connection.accept_bi().await else { return Ok(()) };
        let request = match read_frame(&mut recv).await {
            Ok(Some((0, bytes))) => v1::AskRequest::decode(bytes.as_slice()).ok(),
            _ => None,
        };
        let Some(request) = request else { return Ok(()) };
        let head = shared.running.lock().await.head.clone();
        let Some(head) = head else {
            let _ = write_frame(&mut send, 1, b"this device holds no head").await;
            let _ = send.finish();
            return Ok(());
        };
        let (tx, mut rx) = mpsc::unbounded_channel();
        // the sender goes with the answer: when it is done, the relay below runs dry
        let asking = async move { answer_here(&head, &request, &tx).await.map_err(|e| e.to_string()) };
        let relaying = async {
            while let Some(Ok(step)) = rx.recv().await {
                if write_frame(&mut send, 0, &step.encode_to_vec()).await.is_err() {
                    return;
                }
                if matches!(step.step, Some(v1::ask_response::Step::Done(_))) {
                    return;
                }
            }
        };
        let (result, ()) = tokio::join!(asking, relaying);
        if let Err(e) = result {
            let _ = write_frame(&mut send, 1, e.as_bytes()).await;
        }
        let _ = send.finish();
        let _ = connection.closed().await;
        Ok(())
    }
}

/// What the placement needs to know of `model` with a KV cache of `context` tokens.
#[cfg(feature = "coordinator")]
#[must_use]
pub fn shape(model: &Model, context: u32, kv: omnarc_engine_host::KvType) -> omnarc_scheduler::Model {
    use omnarc_store::layers::Part;
    let layout = &model.layout;
    let mut per_layer = vec![0u64; layout.layers as usize];
    // the header, the embedding, the output and what every window loads all stay with the head
    let mut head = layout.header.end - layout.header.start;
    for (part, range) in &layout.tensors {
        let bytes = range.end - range.start;
        match *part {
            Part::Layer(n) => per_layer[n as usize] += bytes,
            _ => head += bytes,
        }
    }
    let header = &model.header;
    let arch = header.architecture().unwrap_or("llama");
    let get = |k: &str| header.get(&format!("{arch}.{k}")).and_then(omnarc_store::gguf::Value::as_u64);
    let heads = get("attention.head_count").unwrap_or(1).max(1);
    let kv_heads = get("attention.head_count_kv").unwrap_or(heads);
    let embedding = get("embedding_length").unwrap_or(0);
    let key_len = get("attention.key_length").unwrap_or(embedding / heads);
    let value_len = get("attention.value_length").unwrap_or(key_len);
    // keys and values for every position of the context, in the cache's element type
    let values = u64::from(context) * kv_heads * (key_len + value_len);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_precision_loss)]
    let kv_bytes = (values as f64 * kv.bytes_per_value()).ceil() as u64;
    omnarc_scheduler::Model {
        layers: layout.layers,
        // the largest layer: a window is planned for its worst case
        layer_bytes: per_layer.iter().copied().max().unwrap_or(0),
        kv_bytes_per_layer: kv_bytes,
        head_bytes: head,
        tail_bytes: 0,
    }
}

// What every member offers now, fastest first: the plan's ring order. A member whose telemetry stopped, or that the
// profiler has not measured, takes no part.
#[cfg(feature = "coordinator")]
fn offers(shared: &Shared, shape: &omnarc_scheduler::Model) -> Vec<omnarc_scheduler::Device<EndpointId>> {
    let devices = shared.devices.lock().expect("devices").clone();
    let telemetry = shared.telemetry.lock().expect("telemetry").clone();
    let links = shared.links.lock().expect("links").clone();
    let now = crate::pairing::now_unix_ms();
    let mut offers: Vec<_> = shared
        .members
        .list()
        .into_iter()
        .filter_map(|id| {
            let t = telemetry.get(&id).filter(|t| now - t.unix_ms <= GONE_AFTER_MS)?;
            let gbps = devices.get(&id)?.accelerators.iter().map(|a| a.measured_bandwidth_gbps).fold(0.0, f32::max);
            if gbps <= 0.0 {
                return None;
            }
            #[allow(clippy::cast_precision_loss)]
            let seconds_per_layer = shape.layer_bytes as f64 / (f64::from(gbps) * 1e9);
            let link_seconds = if id == shared.key.public() {
                0.0
            } else {
                links.get(&id).map_or(0.002, |(sample, _)| f64::from(sample.rtt_ms) / 2e3)
            };
            Some(omnarc_scheduler::Device {
                id,
                offered_bytes: t.memory_offered_bytes.saturating_sub(ENGINE_RESERVE),
                seconds_per_layer,
                link_seconds,
            })
        })
        .collect();
    offers.sort_by(|a, b| a.seconds_per_layer.total_cmp(&b.seconds_per_layer).then(a.id.cmp(&b.id)));
    offers
}

#[cfg(feature = "coordinator")]
fn to_plan(
    version: u64,
    model: &Model,
    context: u32,
    mode: i32,
    kv: omnarc_engine_host::KvType,
    source: EndpointId,
    placement: &omnarc_scheduler::Placement<EndpointId>,
) -> v1::PlacementPlan {
    let hidden = model
        .header
        .get(&format!("{}.embedding_length", model.header.architecture().unwrap_or("llama")))
        .and_then(omnarc_store::gguf::Value::as_u64)
        .unwrap_or(0);
    v1::PlacementPlan {
        version,
        model: Some(v1::ModelRef {
            name: model.name.clone(),
            quantization: String::new(),
            blake3: model.hash.as_bytes().to_vec(),
            layer_count: model.layout.layers,
            hidden_dim: u32::try_from(hidden).unwrap_or(0),
        }),
        mode,
        stages: placement
            .stages
            .iter()
            .map(|s| v1::Stage {
                node: Some(node_id(s.node)),
                layer_begin: s.layers.start,
                layer_end: s.layers.end,
                ..Default::default()
            })
            .collect(),
        replicas: placement
            .replicas
            .iter()
            .map(|r| v1::Replica { node: Some(node_id(r.node)), layer_begin: r.layers.start, layer_end: r.layers.end })
            .collect(),
        source: Some(node_id(source)),
        context,
        backups: Vec::new(),
        #[allow(clippy::cast_possible_truncation)]
        estimated_ms_per_token: (placement.seconds_per_token * 1e3) as f32,
        kv_cache: kv.name().to_owned(),
    }
}

/// Places a model on the cluster within what every member offers and has the cluster agree on it (NO-41).
#[cfg(feature = "coordinator")]
pub(super) async fn run_model(
    shared: &Arc<Shared>,
    request: v1::RunModelRequest,
) -> Result<v1::RunModelResponse, BoxError> {
    let consensus = shared
        .consensus
        .clone()
        .ok_or("a phone follows the cluster's plan: start a model from one of its computers")?;
    let model = pick(shared, &request.model).await?;
    if shared.models.missing(&model, model.layout.all()).await? > 0 {
        return Err(format!("this device holds only part of {}: start it where all of it is", model.name).into());
    }
    let context = if request.context == 0 { DEFAULT_CONTEXT } else { request.context };
    let kv = omnarc_engine_host::KvType::from_name(&request.kv_cache)
        .ok_or_else(|| format!("unknown KV cache type {}: f16, q8_0 or q4_0", request.kv_cache))?;
    let shape = shape(&model, context, kv);
    let devices = offers(shared, &shape);
    let objective = objective(request.mode)?;
    let placement = omnarc_scheduler::place_for(&shape, &devices, objective)?;
    let version = shared.plan.borrow().as_ref().map_or(0, |p| p.version) + 1;
    let me = shared.key.public();
    let mut plan = to_plan(version, &model, context, request.mode, kv, me, &placement);
    plan.backups = omnarc_scheduler::backups_for(&shape, &devices, &placement, objective)
        .into_iter()
        .map(|(gone, backup)| v1::BackupPlan {
            gone: Some(node_id(gone)),
            plan: Some(to_plan(0, &model, context, request.mode, kv, me, &backup)),
        })
        .collect();
    if !request.dry_run {
        consensus.propose(Command::SetPlan(plan.encode_to_vec())).await?;
    }
    let left_out = placement
        .left_out
        .iter()
        .map(|(id, why)| v1::LeftOutDevice {
            node: Some(node_id(*id)),
            reason: match why {
                omnarc_scheduler::LeftOut::TooSmall => "too_small",
                omnarc_scheduler::LeftOut::NotNeeded => "not_needed",
            }
            .into(),
        })
        .collect();
    #[allow(clippy::cast_possible_truncation)]
    let estimated_ms_per_token = (placement.seconds_per_token * 1e3) as f32;
    Ok(v1::RunModelResponse { plan: Some(plan), left_out, estimated_ms_per_token })
}

#[cfg(feature = "coordinator")]
async fn pick(shared: &Shared, name: &str) -> Result<Model, BoxError> {
    if !name.is_empty() {
        return Ok(shared.models.model(name).await?);
    }
    let models = shared.models.list().await?;
    match models.as_slice() {
        [(_, hash)] => Ok(shared.models.model(&hash.to_hex()).await?),
        [] => Err("this device holds no model: add one first (omnarc-node model add)".into()),
        _ => Err(format!(
            "this device holds {} models: name one ({})",
            models.len(),
            models.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(", ")
        )
        .into()),
    }
}

/// What a run mode asks the placement for (NO-45).
#[cfg(feature = "coordinator")]
fn objective(mode: i32) -> Result<omnarc_scheduler::Objective, BoxError> {
    match v1::RunMode::try_from(mode).unwrap_or(v1::RunMode::Unspecified) {
        v1::RunMode::Throughput => Ok(omnarc_scheduler::Objective::Throughput),
        v1::RunMode::Capacity => {
            Err("capacity mode -- weights read from disk while the ring runs -- is not available yet".into())
        }
        v1::RunMode::Latency | v1::RunMode::Unspecified => Ok(omnarc_scheduler::Objective::Latency),
    }
}

/// How often the device that started the model looks for a faster plan for it.
#[cfg(feature = "coordinator")]
const SUGGEST_EVERY: Duration = Duration::from_secs(15);

/// A plan is suggested only when it is clearly faster: by a tenth at least.
#[cfg(feature = "coordinator")]
const SUGGEST_GAIN: f32 = 0.9;

/// On the device that started the running model: solves its placement again now and then, and when the devices or
/// their offers changed enough that another plan is clearly faster, offers it to the owner (NO-48). Nothing moves
/// until the owner applies it.
#[cfg(feature = "coordinator")]
pub(super) async fn suggest(shared: Arc<Shared>) {
    let mut ticks = tokio::time::interval(SUGGEST_EVERY);
    loop {
        ticks.tick().await;
        let suggestion = better_plan(&shared).await;
        let changed = {
            let mut current = shared.suggestion.lock().expect("suggestion");
            let changed = *current != suggestion;
            *current = suggestion;
            changed
        };
        if changed {
            let _ = shared.events.send(super::Event::ClusterChanged);
        }
    }
}

#[cfg(feature = "coordinator")]
async fn better_plan(shared: &Shared) -> Option<v1::PlanSuggestion> {
    let plan = shared.plan.borrow().clone().filter(|p| !p.stages.is_empty())?;
    if endpoint_id(&key(plan.source.as_ref())) != Some(shared.key.public()) || plan.estimated_ms_per_token <= 0.0 {
        return None;
    }
    let hash = Hash::from_bytes(<[u8; 32]>::try_from(plan.model.as_ref()?.blake3.as_slice()).ok()?).to_hex();
    let model = shared.models.model(&hash).await.ok()?;
    let kv = omnarc_engine_host::KvType::from_name(&plan.kv_cache).unwrap_or_default();
    let shape = shape(&model, plan.context, kv);
    let placement = omnarc_scheduler::place_for(&shape, &offers(shared, &shape), objective(plan.mode).ok()?).ok()?;
    #[allow(clippy::cast_possible_truncation)]
    let ms = (placement.seconds_per_token * 1e3) as f32;
    let candidate = to_plan(0, &model, plan.context, plan.mode, kv, shared.key.public(), &placement);
    let same = |a: &[v1::Stage], b: &[v1::Stage]| {
        a.len() == b.len()
            && a.iter()
                .zip(b)
                .all(|(x, y)| x.node == y.node && x.layer_begin == y.layer_begin && x.layer_end == y.layer_end)
    };
    (ms < plan.estimated_ms_per_token * SUGGEST_GAIN && !same(&candidate.stages, &plan.stages)).then_some({
        v1::PlanSuggestion {
            plan: Some(candidate),
            ms_per_token: ms,
            current_ms_per_token: plan.estimated_ms_per_token,
        }
    })
}

/// Stops the model: a plan without stages.
#[cfg(feature = "coordinator")]
pub(super) async fn stop_model(shared: &Arc<Shared>) -> Result<(), BoxError> {
    let consensus =
        shared.consensus.clone().ok_or("a phone follows the cluster's plan: stop the model from a computer")?;
    let version = shared.plan.borrow().as_ref().map_or(0, |p| p.version) + 1;
    let plan = v1::PlacementPlan { version, ..Default::default() };
    consensus.propose(Command::SetPlan(plan.encode_to_vec())).await
}

/// On the computers' leader: tells the phones the plan now and then, and when a device of the plan goes quiet,
/// switches to the backup solved for its leaving (NO-43).
#[cfg(feature = "coordinator")]
pub(super) async fn keep_running(shared: Arc<Shared>) {
    let Some(consensus) = shared.consensus.clone() else { return };
    let mut ticks = tokio::time::interval(Duration::from_secs(1));
    let mut events = shared.events.subscribe();
    let mut since_spread = Duration::ZERO;
    // members that said they are leaving: they need not go quiet first, so their backup takes over at once
    let mut leaving: HashSet<EndpointId> = HashSet::new();
    // a device just started has heard nobody yet: nobody has gone quiet before they had time to speak
    let started = std::time::Instant::now();
    let grace = Duration::from_millis(2 * GONE_AFTER_MS.unsigned_abs());
    loop {
        tokio::select! {
            _ = ticks.tick() => since_spread += Duration::from_secs(1),
            event = events.recv() => match event {
                Ok(super::Event::MemberLeft(id)) => {
                    leaving.insert(id);
                }
                Ok(super::Event::Telemetry(id, _)) => {
                    leaving.remove(&id);
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                _ => continue,
            },
        }
        if !consensus.is_leader() {
            continue;
        }
        let Some(plan) = shared.plan.borrow().clone() else { continue };
        if since_spread >= SPREAD_EVERY {
            since_spread = Duration::ZERO;
            super::spread_plan(&shared, &plan).await;
        }
        let settled = started.elapsed() >= grace;
        let now = crate::pairing::now_unix_ms();
        let quiet = |id: EndpointId| {
            let telemetry = shared.telemetry.lock().expect("telemetry");
            leaving.contains(&id)
                || !shared.members.contains(&id)
                || (settled && telemetry.get(&id).is_none_or(|t| now - t.unix_ms > GONE_AFTER_MS))
        };
        let gone = plan.stages.iter().filter_map(|s| endpoint_id(&key(s.node.as_ref()))).find(|id| quiet(*id));
        let Some(gone) = gone else { continue };
        let backup = plan.backups.iter().find(|b| endpoint_id(&key(b.gone.as_ref())) == Some(gone));
        let Some(mut next) = backup.and_then(|b| b.plan.clone()) else {
            eprintln!("omnarc: {} left and no backup plan holds the model without it", gone.fmt_short());
            continue;
        };
        next.version = plan.version + 1;
        // the backup's own backups are solved again when the owner starts the model anew
        next.backups.clear();
        eprintln!("omnarc: {} left: plan {} takes over", gone.fmt_short(), next.version);
        let _ = consensus.propose(Command::SetPlan(next.encode_to_vec())).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u8) -> v1::NodeId {
        v1::NodeId { ed25519_public_key: vec![n; 32] }
    }

    fn plan(stages: &[(u8, u32, u32)], replicas: &[(u8, u32, u32)]) -> v1::PlacementPlan {
        v1::PlacementPlan {
            version: 1,
            model: Some(v1::ModelRef { layer_count: 12, ..Default::default() }),
            stages: stages
                .iter()
                .map(|&(n, b, e)| v1::Stage { node: Some(id(n)), layer_begin: b, layer_end: e, ..Default::default() })
                .collect(),
            replicas: replicas
                .iter()
                .map(|&(n, b, e)| v1::Replica { node: Some(id(n)), layer_begin: b, layer_end: e })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn every_device_knows_its_neighbors_and_its_standby() {
        let split = plan(&[(1, 0, 4), (2, 4, 8), (3, 8, 12)], &[(9, 4, 8)]);
        let head = role(&split, &[1; 32]).unwrap();
        assert!(head.head && !head.mirror_only);
        assert_eq!(head.windows, vec![0..4, 12..13], "the head holds the output too");
        assert_eq!((head.next, head.next_replica), (Some(vec![2; 32]), Some(vec![9; 32])));
        assert_eq!(head.prev, vec![vec![3; 32]]);

        let middle = role(&split, &[2; 32]).unwrap();
        assert_eq!(middle.windows, vec![4..8]);
        assert_eq!(middle.replica, Some(vec![9; 32]), "it mirrors to its standby");
        let last = role(&split, &[3; 32]).unwrap();
        assert_eq!(last.prev, vec![vec![2; 32], vec![9; 32]], "the standby may be the one that feeds it");
        assert_eq!(last.next, Some(vec![1; 32]), "the ring closes at the head");

        let standby = role(&split, &[9; 32]).unwrap();
        assert!(standby.mirror_only && !standby.head);
        assert_eq!(standby.windows, vec![4..8]);
        assert_eq!(standby.prev, vec![vec![2; 32], vec![1; 32]], "the device it mirrors, then the one before it");
        assert_eq!(role(&split, &[7; 32]), None, "a device the plan leaves out does nothing");
    }

    #[test]
    fn accuracy_mode_gives_the_answer_most_paths_agree_on() {
        let answers = ["So x = 12.", "I think it is 15", "x equals 12", "The answer: 12"];
        assert_eq!(vote(answers.iter().copied()), Some(("So x = 12.", 3)));
        let words = ["Paris", "  paris ", "Lyon"];
        assert_eq!(vote(words.iter().copied()), Some(("Paris", 2)));
        assert_eq!(vote(std::iter::empty()), None);
    }

    #[test]
    fn a_model_on_one_device_has_whole_copies_that_answer_too() {
        let whole = plan(&[(1, 0, 12)], &[(2, 0, 12)]);
        for n in [1, 2] {
            let part = role(&whole, &[n; 32]).unwrap();
            assert!(part.head && !part.mirror_only && part.next.is_none());
            assert_eq!(part.windows, vec![0..13]);
        }
        assert_eq!(heads(&whole), vec![vec![1; 32], vec![2; 32]]);
        assert_eq!(heads(&plan(&[(1, 0, 6), (2, 6, 12)], &[(3, 6, 12)])), vec![vec![1; 32]], "a standby never answers");
    }
}
