//! The agent's long-running service (`omnarc-node serve`): one endpoint for the cluster's protocols
//! (`protocol/CLUSTER.md`).
//!
//! The service keeps the set of members a node lets in, admits devices through the invitations its owner opens, and
//! talks on the cluster's gossip topic: its `DeviceInfo` when it starts, telemetry once per interval, and every
//! member an admin admits. What it hears it keeps in memory and passes on as [`Event`]s, which the control API and
//! the panel stream. It also serves the models on this device to the other members (`protocol/CLUSTER.md` §4), and
//! measures its link to every member: a ping each telemetry interval, a bandwidth probe once a minute (§3.3).

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use n0_future::StreamExt as _;
use n0_future::task::AbortOnDropHandle;
use omnarc_network::cluster::{self, ClusterKey};
use omnarc_network::gossip::{
    self, GOSSIP_ALPN, Gossip, GossipEvent, GossipReceiver, GossipSender, Members, MembersOnly,
};
use omnarc_network::join::{self, ClusterContext, Invitation, JOIN_ALPN, JoinAttempt};
use omnarc_network::links::{self, PROBE_ALPN, ProbeHandler};
use omnarc_network::protocol::{AcceptError, ProtocolHandler};
use omnarc_network::relay::{self, Relay};
use omnarc_network::ticket::Ticket;
use omnarc_network::{Connection, Endpoint, EndpointAddr, EndpointId, MemoryLookup, Router, SecretKey, TransportAddr};
use omnarc_protocol::v1;
use omnarc_store::{BLOBS_ALPN, Hash, Model, ModelStore, Window};
use prost::Message as _;
use tokio::sync::{broadcast, oneshot};

use crate::consensus::network::RAFT_ALPN;
use crate::consensus::{Agreed, Command, Consensus};
use crate::pairing::now_unix_ms;
use crate::ring::BoxError;
use crate::state::{Cluster, Member, Store};

mod run;

#[cfg(feature = "coordinator")]
pub use run::shape;
pub use run::{ASK_ALPN, DEFAULT_CONTEXT, ENGINE_RESERVE, Role, role};

/// What the service reports as it happens.
#[derive(Clone, Debug)]
pub enum Event {
    /// A member's telemetry.
    Telemetry(EndpointId, v1::Telemetry),
    /// A member told what it is.
    Device(EndpointId, v1::DeviceInfo),
    /// A device was admitted, by this node or by another admin.
    MemberJoined(EndpointId),
    /// A member left on purpose.
    MemberLeft(EndpointId),
    /// A direct gossip neighbor came up or went down.
    Neighbor { id: EndpointId, up: bool },
    /// A member was removed from the cluster: it is let in no more.
    MemberRemoved(EndpointId),
    /// The cluster was renamed or its relays changed.
    ClusterChanged,
    /// The cluster runs another plan, or stopped its model (a plan without stages).
    PlanChanged(v1::PlacementPlan),
    /// A device asks to join and waits for the owner to compare the symbols: answer with [`Service::answer_join`].
    JoinAttempt(JoinAttempt),
}

/// How the service runs.
#[derive(Clone, Debug)]
pub struct Config {
    /// The node's data directory.
    pub data: PathBuf,
    /// Where the node listens.
    pub listen: SocketAddr,
    /// What this device is; its id and addresses are filled in.
    pub device: v1::DeviceInfo,
    /// How often the node sends its telemetry.
    pub telemetry_every: Duration,
    /// Announce the node on the local network by mDNS and find members there (`protocol/CLUSTER.md` §3.4).
    pub lan: bool,
}

/// A running service.
#[derive(Debug)]
pub struct Service {
    shared: Arc<Shared>,
    router: Router,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

#[derive(Debug)]
struct Shared {
    key: SecretKey,
    cluster: Cluster,
    cluster_key: Option<ClusterKey>,
    store: Store,
    models: ModelStore,
    // the cluster's relays, for members on other networks
    relays: Vec<Relay>,
    members: Members,
    lookup: MemoryLookup,
    endpoint: Endpoint,
    sender: GossipSender,
    events: broadcast::Sender<Event>,
    telemetry: Mutex<HashMap<EndpointId, v1::Telemetry>>,
    devices: Mutex<HashMap<EndpointId, v1::DeviceInfo>>,
    invitation: Mutex<Option<Arc<Invitation>>>,
    answers: Mutex<HashMap<EndpointId, oneshot::Sender<bool>>>,
    // wakes the speaker to announce this device again, e.g. when a neighbor comes up
    announce: tokio::sync::Notify,
    // the latest measurement of the link to each member, and when it was taken
    links: Mutex<HashMap<EndpointId, (v1::LinkSample, Instant)>>,
    // this computer's part in the cluster's Raft; phones and browsers have none
    consensus: Option<Consensus>,
    // members being removed: the steward does not bring them back into the log meanwhile
    removing: Mutex<HashSet<EndpointId>>,
    // the cluster's name as the cluster last agreed on it
    name: Mutex<String>,
    data: PathBuf,
    // what the owner offers (Q1); none until the owner names it
    budget: Mutex<Option<v1::Budget>>,
    // the plan the cluster runs, as this device last heard it
    plan: tokio::sync::watch::Sender<Option<v1::PlacementPlan>>,
    // what this device runs of it
    running: tokio::sync::Mutex<run::Running>,
    // where the tensor connections members open go: the ring this device serves, if any
    tensors: Mutex<Option<tokio::sync::mpsc::Sender<Connection>>>,
    // this device's windows in the plan, for its telemetry
    stages: Mutex<Vec<v1::StageLoad>>,
    // a faster plan for the running model, on the device that started it
    suggestion: Mutex<Option<v1::PlanSuggestion>>,
    // the layers being fetched for the plan, for the telemetry
    fetching: Mutex<Option<v1::ModelFetch>>,
}

// What the service keeps doing while it runs.
fn spawn_tasks(
    shared: &Arc<Shared>,
    receiver: GossipReceiver,
    device: v1::DeviceInfo,
    every: Duration,
    lan: Option<omnarc_network::MdnsAddressLookup>,
) -> Vec<tokio::task::JoinHandle<()>> {
    let mut tasks = vec![
        tokio::spawn(listen(Arc::clone(shared), receiver)),
        tokio::spawn(speak(Arc::clone(shared), device, every)),
        tokio::spawn(measure(Arc::clone(shared), every)),
        tokio::spawn(run::carry_out(Arc::clone(shared))),
    ];
    if let Some(lan) = lan {
        tasks.push(tokio::spawn(discover(Arc::clone(shared), lan)));
    }
    if shared.consensus.is_some() {
        tasks.push(tokio::spawn(follow(Arc::clone(shared))));
        tasks.push(tokio::spawn(steward(Arc::clone(shared))));
        tasks.push(tokio::spawn(run::keep_running(Arc::clone(shared))));
        #[cfg(feature = "coordinator")]
        tasks.push(tokio::spawn(run::suggest(Arc::clone(shared))));
    }
    tasks
}

// The device with what the profiler measured within what the owner offers, on as many cores as are offered (NO-40).
async fn profiled(mut device: v1::DeviceInfo, budget: Option<&v1::Budget>) -> v1::DeviceInfo {
    let cores = crate::budget::offered_cores(budget, device.cpu_cores);
    let profile = tokio::task::spawn_blocking(move || crate::profile::measure(cores, PROFILE_TIME))
        .await
        .unwrap_or(crate::profile::Profile { bandwidth_bytes_per_s: 0.0, flops: 0.0 });
    device.accelerators.retain(|a| a.backend != v1::Backend::Cpu as i32);
    #[allow(clippy::cast_possible_truncation)]
    device.accelerators.push(v1::Accelerator {
        backend: v1::Backend::Cpu as i32,
        name: device.cpu_model.clone(),
        memory_bytes: crate::budget::offered_memory_now(budget, device.memory_total_bytes),
        measured_fp16_tflops: (profile.flops / 1e12) as f32,
        measured_bandwidth_gbps: (profile.bandwidth_bytes_per_s / 1e9) as f32,
        accuracy_gate_passed: false,
    });
    device
}

/// How long a leaving device gives its leave notice to reach its neighbors before it closes its connections.
const LEAVE_FLUSH: Duration = Duration::from_millis(250);

/// How long the profiler measures this device when the service starts.
const PROFILE_TIME: Duration = Duration::from_secs(2);

/// Telemetry ticks between two announcements of the device even when nothing changed.
const ANNOUNCE_EVERY: u32 = 60;

/// How often the bandwidth of a link is probed, besides when the link changes.
const BANDWIDTH_EVERY: Duration = Duration::from_secs(60);

/// How often the leader of the cluster's Raft brings computers into it.
const STEWARD_EVERY: Duration = Duration::from_secs(2);

/// Whether a device of this platform keeps the cluster's Raft: computers do, phones and browsers do not.
fn keeps_raft(platform: i32) -> bool {
    matches!(v1::Platform::try_from(platform), Ok(v1::Platform::Linux | v1::Platform::Windows | v1::Platform::Macos))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

impl Service {
    /// Starts the service of the node in `config.data`, which must belong to a cluster.
    ///
    /// # Errors
    /// The node belongs to no cluster, its state cannot be read, or the endpoint cannot be bound.
    ///
    /// # Panics
    /// Never: the join handler gets the shared state once.
    pub async fn start(config: Config) -> Result<Self, BoxError> {
        let store = Store::open(&config.data)?;
        let cluster =
            store.cluster()?.ok_or("this device belongs to no cluster yet: run omnarc-node create or join")?;
        let cluster_key = store.cluster_key()?;
        let key = omnarc_network::load_or_create_key(&config.data.join("node.key"))?;
        let known = store.members()?;
        let members = Members::new(known.iter().map(|member| member.id));
        let relays = relay::parse_all(&store.relays()?)?;
        let lookup = MemoryLookup::new();
        for member in known.iter().filter(|member| member.id != key.public()) {
            lookup.add_endpoint_info(reach(member.id, &member.addrs, &member.device.relay_url, &relays));
        }

        let models = crate::models::open(&config.data).await?;
        let budget = crate::budget::load(&config.data);
        let mut device = profiled(config.device, budget.as_ref()).await;
        // a network without multicast only loses the local discovery
        let lan = if config.lan { omnarc_network::lan_lookup(key.public()).ok() } else { None };
        let endpoint =
            omnarc_network::bind_cluster(key.clone(), config.listen, lookup.clone(), lan.clone(), &relays).await?;
        let gossip = Gossip::builder().spawn(endpoint.clone());
        let joins = JoinHandler(Arc::new(OnceLock::new()));
        let tensors = run::TensorHandler(Arc::new(OnceLock::new()));
        let asks = run::AskHandler(Arc::new(OnceLock::new()));
        let raft = if keeps_raft(device.platform) {
            Some(Consensus::start(&config.data, endpoint.clone()).await?)
        } else {
            None
        };
        let mut router = Router::builder(endpoint.clone())
            .accept(GOSSIP_ALPN, MembersOnly::new(gossip.clone(), members.clone()))
            .accept(JOIN_ALPN, joins.clone())
            .accept(BLOBS_ALPN, MembersOnly::new(models.protocol(), members.clone()))
            .accept(PROBE_ALPN, MembersOnly::new(ProbeHandler, members.clone()))
            .accept(omnarc_network::TENSOR_ALPN, MembersOnly::new(tensors.clone(), members.clone()))
            .accept(ASK_ALPN, MembersOnly::new(asks.clone(), members.clone()));
        let consensus = match raft {
            Some((consensus, handler)) => {
                router = router.accept(RAFT_ALPN, MembersOnly::new(handler, members.clone()));
                Some(consensus)
            }
            None => None,
        };
        let router = router.spawn();

        let others: Vec<EndpointId> = members.list().into_iter().filter(|id| *id != key.public()).collect();
        let (sender, receiver) = gossip.subscribe(gossip::topic(cluster.id), others).await?.split();
        let (events, _) = broadcast::channel(1024);
        let name = Mutex::new(cluster.name.clone());
        let shared = Arc::new(Shared {
            key,
            cluster,
            cluster_key,
            store,
            models,
            relays,
            members,
            lookup,
            endpoint,
            sender,
            events,
            telemetry: Mutex::default(),
            // what each member said when it joined stands until it speaks for itself, also after a restart
            devices: Mutex::new(
                known
                    .iter()
                    .filter(|member| !member.device.name.is_empty())
                    .map(|member| (member.id, member.device.clone()))
                    .collect(),
            ),
            invitation: Mutex::default(),
            answers: Mutex::default(),
            announce: tokio::sync::Notify::new(),
            links: Mutex::default(),
            name,
            consensus,
            removing: Mutex::default(),
            data: config.data.clone(),
            budget: Mutex::new(budget),
            plan: tokio::sync::watch::Sender::new(None),
            running: tokio::sync::Mutex::default(),
            tensors: Mutex::default(),
            stages: Mutex::default(),
            suggestion: Mutex::default(),
            fetching: Mutex::default(),
        });
        joins.0.set(Arc::clone(&shared)).expect("set once");
        tensors.0.set(Arc::clone(&shared)).expect("set once");
        asks.0.set(Arc::clone(&shared)).expect("set once");

        // the name the owner gave this device when it joined or created the cluster stays its name
        if let Some(me) = known.iter().find(|member| member.id == shared.key.public())
            && !me.device.name.is_empty()
        {
            device.name.clone_from(&me.device.name);
        }
        device.id = Some(node_id(shared.key.public()));
        let tasks = spawn_tasks(&shared, receiver, device, config.telemetry_every, lan);
        Ok(Self { shared, router, tasks })
    }

    /// This node's id.
    #[must_use]
    pub fn id(&self) -> EndpointId {
        self.shared.key.public()
    }

    /// The cluster the node belongs to.
    #[must_use]
    pub fn cluster(&self) -> &Cluster {
        &self.shared.cluster
    }

    /// The addresses this node can be dialed at.
    #[must_use]
    pub fn addrs(&self) -> Vec<SocketAddr> {
        own_addrs(&self.shared.endpoint)
    }

    /// The members this node lets in.
    #[must_use]
    pub fn members(&self) -> Vec<EndpointId> {
        self.shared.members.list()
    }

    /// The latest telemetry heard from each member.
    ///
    /// # Panics
    /// When a task panicked while holding the telemetry.
    #[must_use]
    pub fn telemetry(&self) -> HashMap<EndpointId, v1::Telemetry> {
        self.shared.telemetry.lock().expect("telemetry").clone()
    }

    /// What each member said it is.
    ///
    /// # Panics
    /// When a task panicked while holding the devices.
    #[must_use]
    pub fn devices(&self) -> HashMap<EndpointId, v1::DeviceInfo> {
        self.shared.devices.lock().expect("devices").clone()
    }

    /// The models on this device, which the other members can fetch from it.
    #[must_use]
    pub fn models(&self) -> &ModelStore {
        &self.shared.models
    }

    /// Fetches `window` of the model `hash` from the member `from`: only the bytes still missing travel. The model
    /// is kept under `name`, or under the name its header gives.
    ///
    /// # Errors
    /// The member cannot be reached or does not have those bytes.
    pub async fn fetch_model(
        &self,
        from: EndpointId,
        name: Option<&str>,
        hash: Hash,
        window: Window,
        progress: impl FnMut(u64),
    ) -> Result<Model, BoxError> {
        let connection = self.shared.endpoint.connect(from, BLOBS_ALPN).await?;
        Ok(self.shared.models.fetch(&connection, name, hash, window, progress).await?)
    }

    /// Events from now on.
    #[must_use]
    pub fn events(&self) -> broadcast::Receiver<Event> {
        self.shared.events.subscribe()
    }

    /// Opens an invitation for one device, valid for `ttl` (at most 10 minutes); it replaces an earlier one.
    ///
    /// # Errors
    /// This node holds no cluster key.
    ///
    /// # Panics
    /// When a task panicked while holding the invitation.
    pub fn invite(&self, ttl: Duration, relays: Vec<String>) -> Result<Ticket, BoxError> {
        if self.shared.cluster_key.is_none() {
            return Err("this device holds no cluster key: invite from the device that created the cluster".into());
        }
        // the cluster's relays, with their tokens, and any others given
        let mut all = self.shared.store.relays()?;
        for extra in relays {
            if !all.contains(&extra) {
                all.push(extra);
            }
        }
        let ticket =
            Ticket::new(self.shared.cluster.id, self.id(), own_addrs(&self.shared.endpoint), all, ttl, now_unix_ms())?;
        *self.shared.invitation.lock().expect("invitation") = Some(Arc::new(Invitation::new(ticket.clone())));
        Ok(ticket)
    }

    /// The owner's answer for a device that asked to join: whether the symbols match. Whether such a device waited.
    ///
    /// # Panics
    /// When a task panicked while holding the answers.
    #[must_use = "whether a device was waiting for this answer"]
    pub fn answer_join(&self, joiner: EndpointId, matches: bool) -> bool {
        let waiting = self.shared.answers.lock().expect("answers").remove(&joiner);
        waiting.is_some_and(|answer| answer.send(matches).is_ok())
    }

    /// The control API's view of this service.
    #[must_use]
    pub fn api(&self) -> Arc<dyn omnarc_api::Cluster> {
        Arc::new(ApiView(Arc::clone(&self.shared)))
    }

    /// What the cluster agreed on, when this device keeps the cluster's Raft.
    #[must_use]
    pub fn agreed(&self) -> Option<Agreed> {
        self.shared.consensus.as_ref().map(|c| c.agreed().borrow().clone())
    }

    /// Removes `member` from the cluster: the computers of the cluster agree on it, let it in no more and refuse its
    /// token. Only an admin removes members.
    ///
    /// # Errors
    /// This device is no admin or keeps no Raft, the member is unknown, or the cluster cannot agree now.
    pub async fn remove_member(&self, member: EndpointId) -> Result<(), BoxError> {
        remove_member(&self.shared, member).await
    }

    /// Tells the cluster that this device leaves, then stops.
    pub async fn shutdown(self) {
        let leave = v1::GossipPayload { kind: Some(v1::gossip_payload::Kind::Leave(v1::LeaveNotice {})) };
        let _ = self.shared.sender.broadcast(gossip::seal(&self.shared.key, &leave, now_unix_ms()).into()).await;
        // the notice goes out before the connections close, or the cluster learns of the leaving only from silence
        tokio::time::sleep(LEAVE_FLUSH).await;
        for task in &self.tasks {
            task.abort();
        }
        // the tasks hold the shared state and with it the open state database: once they have ended, a service can
        // start again on the same data (Windows keeps the database locked until then)
        for task in self.tasks {
            let _ = task.await;
        }
        if let Some(consensus) = &self.shared.consensus {
            consensus.shutdown().await;
        }
        let _ = self.router.shutdown().await;
        // the model store closes its database now, not whenever its last handle happens to go
        self.shared.models.shutdown().await;
    }
}

// Removes `member` once the computers of the cluster agree; see [`Service::remove_member`].
async fn remove_member(shared: &Arc<Shared>, member: EndpointId) -> Result<(), BoxError> {
    let me = cluster::verify(shared.cluster.id, shared.key.public(), &shared.cluster.token, |_| false)?;
    if !me.has(cluster::Right::Admin) {
        return Err("only an admin removes members".into());
    }
    if member == shared.key.public() {
        return Err("a device cannot remove itself".into());
    }
    let consensus = shared.consensus.as_ref().ok_or("only a computer of the cluster removes members")?;
    let known = shared.store.member(member)?.ok_or("no such member")?;
    let revocation_ids = cluster::verify(shared.cluster.id, member, &known.token, |_| false)
        .map(|m| m.revocation_ids.iter().map(|id| hex(id)).collect())
        .unwrap_or_default();
    // out of the log's membership first, while it is still let in, so the log never waits for it
    shared.removing.lock().expect("removing").insert(member);
    let result = async {
        consensus.remove_node(member).await?;
        consensus.propose(Command::RemoveMember { id: member.to_string(), revocation_ids }).await
    }
    .await;
    shared.removing.lock().expect("removing").remove(&member);
    result
}

// Renames the cluster or changes its relays once the computers of the cluster agree.
async fn update_cluster(shared: &Arc<Shared>, request: v1::UpdateClusterRequest) -> Result<(), BoxError> {
    let me = cluster::verify(shared.cluster.id, shared.key.public(), &shared.cluster.token, |_| false)?;
    if !me.has(cluster::Right::Admin) {
        return Err("only an admin changes the cluster".into());
    }
    let consensus = shared.consensus.as_ref().ok_or("only a computer of the cluster changes it")?;
    let name = request.name.trim();
    if !name.is_empty() {
        consensus.propose(Command::SetName(name.to_owned())).await?;
    }
    if request.add_relay.trim().is_empty() && request.remove_relay.trim().is_empty() {
        return Ok(());
    }
    // the same relay, whatever token is written with it
    let same = |a: &str, b: &str| a.parse::<Relay>().ok().map(|r| r.url) == b.parse::<Relay>().ok().map(|r| r.url);
    let mut relays = shared.store.relays()?;
    let remove = request.remove_relay.trim();
    if !remove.is_empty() {
        relays.retain(|relay| !same(relay, remove));
    }
    let add = request.add_relay.trim();
    if !add.is_empty() {
        add.parse::<Relay>()?;
        relays.retain(|relay| !same(relay, add));
        relays.push(add.to_owned());
    }
    consensus.propose(Command::SetRelays(relays)).await
}

// What the control API sees of the service.
struct ApiView(Arc<Shared>);

impl ApiView {
    fn device(&self, id: EndpointId) -> v1::DeviceInfo {
        let known = self.0.devices.lock().expect("devices").get(&id).cloned();
        known.unwrap_or_else(|| v1::DeviceInfo { id: Some(node_id(id)), ..Default::default() })
    }
}

impl omnarc_api::Cluster for ApiView {
    fn snapshot(&self) -> v1::ClusterSnapshot {
        let shared = &self.0;
        v1::ClusterSnapshot {
            id: Some(v1::ClusterId { ed25519_public_key: shared.cluster.id.as_bytes().to_vec() }),
            name: shared.name.lock().expect("name").clone(),
            devices: shared.members.list().into_iter().map(|id| self.device(id)).collect(),
            telemetry: shared.telemetry.lock().expect("telemetry").values().cloned().collect(),
            plans: shared.plan.borrow().clone().filter(|p| !p.stages.is_empty()).into_iter().collect(),
            this_node: Some(node_id(shared.key.public())),
            can_invite: shared.cluster_key.is_some(),
            suggestion: shared.suggestion.lock().expect("suggestion").clone(),
            relays: relay::parse_all(&shared.store.relays().unwrap_or_default())
                .unwrap_or_default()
                .iter()
                .map(|relay| relay.url.to_string())
                .collect(),
        }
    }

    fn watch(&self) -> n0_future::boxed::BoxStream<v1::WatchClusterResponse> {
        use v1::watch_cluster_response::Event as Out;
        let view = ApiView(Arc::clone(&self.0));
        let events = self.0.events.subscribe();
        Box::pin(n0_future::stream::unfold((view, events), |(view, mut events)| async move {
            loop {
                let event = match events.recv().await {
                    Ok(Event::Telemetry(_, telemetry)) => Out::Telemetry(telemetry),
                    Ok(Event::Device(_, device)) => Out::DeviceJoined(device),
                    Ok(Event::MemberJoined(id)) => Out::DeviceJoined(view.device(id)),
                    Ok(Event::MemberLeft(id)) => Out::DeviceLeft(node_id(id)),
                    Ok(Event::MemberRemoved(id)) => Out::DeviceRemoved(node_id(id)),
                    Ok(Event::ClusterChanged) => Out::Snapshot(view.snapshot()),
                    Ok(Event::PlanChanged(plan)) => Out::PlanChanged(plan),
                    Ok(Event::JoinAttempt(attempt)) => Out::JoinAttempt(v1::JoinAttempt {
                        joiner: Some(node_id(attempt.joiner)),
                        device: Some(attempt.device),
                        symbols: attempt
                            .emoji
                            .iter()
                            .map(|e| v1::SasSymbol { emoji: e.symbol.into(), name: e.name.into() })
                            .collect(),
                    }),
                    Ok(Event::Neighbor { .. }) | Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return None,
                };
                return Some((v1::WatchClusterResponse { event: Some(event) }, (view, events)));
            }
        }))
    }

    fn create_ticket(&self, ttl_seconds: u32) -> Result<v1::CreateTicketResponse, omnarc_api::Error> {
        use omnarc_api::{Code, Error};
        let shared = &self.0;
        if shared.cluster_key.is_none() {
            return Err(Error::new(Code::FailedPrecondition, "this device holds no cluster key"));
        }
        let ttl = Duration::from_secs(u64::from(ttl_seconds.clamp(1, 600)));
        // the cluster's relays, so a device on another network can join from the panel's ticket too
        let relays = shared.store.relays().map_err(|e| Error::new(Code::Internal, e.to_string()))?;
        let ticket = Ticket::new(
            shared.cluster.id,
            shared.key.public(),
            own_addrs(&shared.endpoint),
            relays,
            ttl,
            now_unix_ms(),
        )
        .map_err(|e| Error::new(Code::Internal, e.to_string()))?;
        *shared.invitation.lock().expect("invitation") = Some(Arc::new(Invitation::new(ticket.clone())));
        let qr_payload = ticket.encode();
        let qr_svg = qrcode::QrCode::new(qr_payload.as_bytes())
            .map(|code| {
                code.render::<qrcode::render::svg::Color<'_>>().min_dimensions(240, 240).quiet_zone(true).build()
            })
            .unwrap_or_default();
        Ok(v1::CreateTicketResponse { ticket: Some(ticket.to_proto()), qr_payload, qr_svg })
    }

    fn answer_join(&self, request: &v1::AnswerJoinRequest) -> Result<bool, omnarc_api::Error> {
        let joiner = request
            .joiner
            .as_ref()
            .and_then(|id| <[u8; 32]>::try_from(id.ed25519_public_key.as_slice()).ok())
            .and_then(|bytes| EndpointId::from_bytes(&bytes).ok())
            .ok_or_else(|| omnarc_api::Error::new(omnarc_api::Code::InvalidArgument, "no device named"))?;
        let waiting = self.0.answers.lock().expect("answers").remove(&joiner);
        Ok(waiting.is_some_and(|answer| answer.send(request.matches).is_ok()))
    }

    fn update_cluster(
        &self,
        request: v1::UpdateClusterRequest,
    ) -> n0_future::boxed::BoxFuture<Result<(), omnarc_api::Error>> {
        use omnarc_api::{Code, Error};
        let shared = Arc::clone(&self.0);
        Box::pin(async move {
            update_cluster(&shared, request).await.map_err(|e| {
                let message = e.to_string();
                let code =
                    if message.contains("only an admin") { Code::PermissionDenied } else { Code::FailedPrecondition };
                Error::new(code, message)
            })
        })
    }

    fn remove_member(&self, member: v1::NodeId) -> n0_future::boxed::BoxFuture<Result<(), omnarc_api::Error>> {
        use omnarc_api::{Code, Error};
        let shared = Arc::clone(&self.0);
        Box::pin(async move {
            let id = <[u8; 32]>::try_from(member.ed25519_public_key.as_slice())
                .ok()
                .and_then(|bytes| EndpointId::from_bytes(&bytes).ok())
                .ok_or_else(|| Error::new(Code::InvalidArgument, "not a member id"))?;
            remove_member(&shared, id).await.map_err(|e| {
                let message = e.to_string();
                let code = if message.contains("only an admin") {
                    Code::PermissionDenied
                } else if message.contains("no such member") {
                    Code::NotFound
                } else {
                    Code::FailedPrecondition
                };
                Error::new(code, message)
            })
        })
    }
    fn set_budget(&self, budget: v1::Budget) -> Result<(), omnarc_api::Error> {
        use omnarc_api::{Code, Error};
        crate::budget::save(&self.0.data, &budget).map_err(|e| Error::new(Code::InvalidArgument, e.to_string()))?;
        *self.0.budget.lock().expect("budget") = Some(budget);
        // the cluster hears of it with the next announcement, which goes out now
        self.0.announce.notify_one();
        Ok(())
    }

    fn run_model(
        &self,
        request: v1::RunModelRequest,
    ) -> n0_future::boxed::BoxFuture<Result<v1::RunModelResponse, omnarc_api::Error>> {
        let shared = Arc::clone(&self.0);
        Box::pin(async move {
            #[cfg(feature = "coordinator")]
            {
                run::run_model(&shared, request)
                    .await
                    .map_err(|e| omnarc_api::Error::new(omnarc_api::Code::FailedPrecondition, e.to_string()))
            }
            #[cfg(not(feature = "coordinator"))]
            {
                let _ = (shared, request);
                Err(omnarc_api::Error::new(
                    omnarc_api::Code::FailedPrecondition,
                    "this device follows the cluster's plan: start a model from one of its computers",
                ))
            }
        })
    }

    fn stop_model(&self) -> n0_future::boxed::BoxFuture<Result<(), omnarc_api::Error>> {
        let shared = Arc::clone(&self.0);
        Box::pin(async move {
            #[cfg(feature = "coordinator")]
            {
                run::stop_model(&shared)
                    .await
                    .map_err(|e| omnarc_api::Error::new(omnarc_api::Code::FailedPrecondition, e.to_string()))
            }
            #[cfg(not(feature = "coordinator"))]
            {
                let _ = shared;
                Err(omnarc_api::Error::new(
                    omnarc_api::Code::FailedPrecondition,
                    "this device follows the cluster's plan: stop the model from one of its computers",
                ))
            }
        })
    }

    fn metrics(&self) -> String {
        use std::fmt::Write as _;
        let shared = &self.0;
        let me = shared.key.public();
        let telemetry = shared.telemetry.lock().expect("telemetry").get(&me).cloned().unwrap_or_default();
        let plan = shared.plan.borrow().clone().unwrap_or_default();
        let mut out = String::new();
        let mut gauge = |name: &str, help: &str, kind: &str, value: f64| {
            let _ = writeln!(out, "# HELP omnarc_{name} {help}\n# TYPE omnarc_{name} {kind}\nomnarc_{name} {value}");
        };
        #[allow(clippy::cast_precision_loss)]
        {
            gauge(
                "tokens_computed_total",
                "Tokens this device's windows computed since the service started.",
                "counter",
                crate::ring::TOKENS_COMPUTED.load(Ordering::Relaxed) as f64,
            );
            gauge(
                "compute_seconds_total",
                "Time this device's windows spent computing.",
                "counter",
                crate::ring::MICROS_COMPUTING.load(Ordering::Relaxed) as f64 / 1e6,
            );
            gauge("members", "Devices in the cluster.", "gauge", shared.members.list().len() as f64);
            gauge("plan_version", "The version of the plan this device runs.", "gauge", plan.version as f64);
            gauge(
                "plan_stages",
                "Devices computing the running model; 0 when none runs.",
                "gauge",
                plan.stages.len() as f64,
            );
            gauge(
                "memory_offered_bytes",
                "Memory this device offers the cluster now.",
                "gauge",
                telemetry.memory_offered_bytes as f64,
            );
            gauge("memory_used_bytes", "Memory in use on this device.", "gauge", telemetry.memory_used_bytes as f64);
            gauge("cpu_load", "This device's CPU load, 0 to 1.", "gauge", f64::from(telemetry.cpu_load));
        }
        out
    }

    fn list_models(&self) -> n0_future::boxed::BoxFuture<Result<v1::ListModelsResponse, omnarc_api::Error>> {
        let shared = Arc::clone(&self.0);
        Box::pin(async move {
            let internal = |e: omnarc_store::Error| omnarc_api::Error::new(omnarc_api::Code::Internal, e.to_string());
            let mut models = Vec::new();
            for (_, hash) in shared.models.list().await.map_err(internal)? {
                let model = shared.models.model(&hash.to_hex()).await.map_err(internal)?;
                let complete = shared.models.missing(&model, model.layout.all()).await.map_err(internal)? == 0;
                models.push(v1::ModelInfo {
                    name: model.name.clone(),
                    blake3: hash.to_hex(),
                    file_bytes: model.layout.file_len,
                    layer_count: model.layout.layers,
                    complete,
                });
            }
            Ok(v1::ListModelsResponse { models })
        })
    }

    fn ask(&self, request: v1::AskRequest) -> n0_future::boxed::BoxStream<Result<v1::AskResponse, omnarc_api::Error>> {
        Box::pin(
            run::ask(Arc::clone(&self.0), request)
                .map(|step| step.map_err(|e| omnarc_api::Error::new(omnarc_api::Code::Unavailable, e))),
        )
    }
}

fn node_id(id: EndpointId) -> v1::NodeId {
    v1::NodeId { ed25519_public_key: id.as_bytes().to_vec() }
}

/// Where a member is dialed: its direct addresses, and the relay it announced — or, before it announced one, every
/// relay of the cluster.
pub(crate) fn reach(id: EndpointId, addrs: &[SocketAddr], relay_url: &str, relays: &[Relay]) -> EndpointAddr {
    let mut transports: Vec<TransportAddr> = addrs.iter().copied().map(TransportAddr::Ip).collect();
    match relay_url.parse::<Relay>() {
        Ok(home) => transports.push(home.transport_addr()),
        Err(_) => transports.extend(relays.iter().map(Relay::transport_addr)),
    }
    EndpointAddr::from_parts(id, transports)
}

fn own_addrs(endpoint: &Endpoint) -> Vec<SocketAddr> {
    endpoint.addr().ip_addrs().copied().filter(|addr| !addr.ip().is_unspecified()).collect()
}

// Applies what the cluster agreed on: agreed members are let in, removed ones no more.
async fn follow(shared: Arc<Shared>) {
    let Some(consensus) = shared.consensus.clone() else { return };
    let mut agreed = consensus.agreed();
    loop {
        let state = agreed.borrow_and_update().clone();
        for (id, token) in &state.members {
            let Some(id) = parse_id(id) else { continue };
            let revoked = |rid: &[u8]| state.revoked.contains(&hex(rid));
            if !shared.members.contains(&id) && cluster::verify(shared.cluster.id, id, token, revoked).is_ok() {
                let _ = shared.members.insert(id);
                if shared.store.member(id).ok().flatten().is_none() {
                    let _ = shared.store.update_member(id, None, None);
                }
                let _ = shared.events.send(Event::MemberJoined(id));
            }
        }
        // the name and the relays, once the log has them
        if !state.name.is_empty() {
            let renamed = {
                let mut name = shared.name.lock().expect("name");
                let renamed = *name != state.name;
                name.clone_from(&state.name);
                renamed
            };
            let relays_changed = shared.store.relays().is_ok_and(|relays| relays != state.relays);
            if renamed {
                let _ = shared.store.rename_cluster(&state.name);
            }
            if relays_changed {
                // tickets name them at once; the endpoint uses them from its next start
                let _ = shared.store.set_relays(&state.relays);
            }
            if renamed || relays_changed {
                let _ = shared.events.send(Event::ClusterChanged);
            }
        }
        if let Some(plan) = state.plan.as_deref().and_then(|bytes| v1::PlacementPlan::decode(bytes).ok()) {
            run::adopt(&shared, plan.clone());
            if consensus.is_leader() {
                spread_plan(&shared, &plan).await;
            }
        }
        for id in state.removed.iter().filter_map(|id| parse_id(id)) {
            if id != shared.key.public() && shared.members.remove(&id) {
                let _ = shared.store.remove_member(id);
                shared.devices.lock().expect("devices").remove(&id);
                shared.telemetry.lock().expect("telemetry").remove(&id);
                let _ = shared.events.send(Event::MemberRemoved(id));
            }
        }
        if agreed.changed().await.is_err() {
            return;
        }
    }
}

fn parse_id(hex_id: &str) -> Option<EndpointId> {
    let bytes: Vec<u8> = (0..hex_id.len())
        .step_by(2)
        .map(|i| hex_id.get(i..i + 2).and_then(|b| u8::from_str_radix(b, 16).ok()))
        .collect::<Option<_>>()?;
    EndpointId::from_bytes(&<[u8; 32]>::try_from(bytes.as_slice()).ok()?).ok()
}

// On the creator: starts the cluster's log, writes what it knows into it, and keeps bringing the computers of the
// cluster in — as learners, then as voters once they keep up. On the others: nothing until they lead.
async fn steward(shared: Arc<Shared>) {
    let Some(consensus) = shared.consensus.clone() else { return };
    if shared.cluster_key.is_some() {
        if consensus.initialize_alone().await.is_err() {
            return;
        }
        // a cluster that existed before its log: what this device knows becomes the agreed state
        if consensus.agreed().borrow().name.is_empty() {
            let mut commands = vec![
                Command::SetName(shared.cluster.name.clone()),
                Command::SetRelays(shared.store.relays().unwrap_or_default()),
            ];
            for member in shared.store.members().unwrap_or_default() {
                if !member.token.is_empty() {
                    commands.push(Command::AddMember { id: hex(member.id.as_bytes()), token: member.token });
                }
            }
            for command in commands {
                let _ = consensus.propose(command).await;
            }
        }
    }
    let mut ticks = tokio::time::interval(STEWARD_EVERY);
    loop {
        ticks.tick().await;
        if !consensus.is_leader() {
            continue;
        }
        let in_raft: HashSet<u64> = {
            let metrics = consensus.raft().metrics().borrow().clone();
            metrics.membership_config.membership().nodes().map(|(id, _)| *id).collect()
        };
        let devices = shared.devices.lock().expect("devices").clone();
        let removing = shared.removing.lock().expect("removing").clone();
        for id in shared.members.list().into_iter().filter(|id| !removing.contains(id)) {
            let computer = devices.get(&id).is_some_and(|device| keeps_raft(device.platform));
            if computer && !in_raft.contains(&crate::consensus::raft_id(id)) {
                let _ = consensus.add_learner(id).await;
            }
        }
        let _ = consensus.promote().await;
    }
}

// Keeps the addresses at which mDNS finds members on the local network, so they are known after a restart too.
async fn discover(shared: Arc<Shared>, lan: omnarc_network::MdnsAddressLookup) {
    let mut events = lan.subscribe().await;
    while let Some(event) = events.next().await {
        let omnarc_network::DiscoveryEvent::Discovered { endpoint_info, .. } = event else {
            continue;
        };
        let id = endpoint_info.endpoint_id;
        let addrs: Vec<SocketAddr> = endpoint_info.data.ip_addrs().copied().collect();
        if id == shared.key.public() || !shared.members.contains(&id) || addrs.is_empty() {
            continue;
        }
        let _ = shared.store.update_member(id, None, Some(addrs));
    }
}

// Measures the link to every other member, each in a task of its own, as members come and go. The handles stop their
// tasks when they are dropped: with a member that leaves, and with this task when the service stops.
async fn measure(shared: Arc<Shared>, every: Duration) {
    let mut tasks: HashMap<EndpointId, AbortOnDropHandle<()>> = HashMap::new();
    let mut rejoined: HashMap<EndpointId, Instant> = HashMap::new();
    let mut ticks = tokio::time::interval(every);
    loop {
        ticks.tick().await;
        let me = shared.key.public();
        let members: HashSet<EndpointId> = shared.members.list().into_iter().filter(|id| *id != me).collect();
        tasks.retain(|id, _| {
            let keep = members.contains(id);
            if !keep {
                shared.links.lock().expect("links").remove(id);
            }
            keep
        });
        // a member this device reaches but no longer hears from has dropped out of its gossip neighbors -- as when
        // it restarted and its old connection lingers here -- so it is asked to join again
        let now = now_unix_ms();
        let deaf: Vec<EndpointId> = {
            let telemetry = shared.telemetry.lock().expect("telemetry");
            let links = shared.links.lock().expect("links");
            members
                .iter()
                .copied()
                .filter(|id| links.get(id).is_some_and(|(_, at)| at.elapsed() < every * 5 + Duration::from_secs(2)))
                .filter(|id| telemetry.get(id).is_none_or(|t| now - t.unix_ms > UNHEARD_MS))
                .filter(|id| rejoined.get(id).is_none_or(|at: &Instant| at.elapsed() > REJOIN_EVERY))
                .collect()
        };
        if !deaf.is_empty() {
            for id in &deaf {
                rejoined.insert(*id, Instant::now());
            }
            let _ = shared.sender.join_peers(deaf).await;
        }
        for id in members {
            tasks
                .entry(id)
                .or_insert_with(|| AbortOnDropHandle::new(tokio::spawn(measure_link(Arc::clone(&shared), id, every))));
        }
    }
}

/// A member reached over its link but unheard over gossip this long is asked to join again; at most this often.
const UNHEARD_MS: i64 = 10_000;
const REJOIN_EVERY: Duration = Duration::from_secs(15);

// Keeps a probe connection to `member` and measures it: a ping every `every`, the bandwidth once a minute and when
// the kind of link changes. A probe aims at about a quarter of a second of sending, so it grows on fast links.
async fn measure_link(shared: Arc<Shared>, member: EndpointId, every: Duration) {
    let patience = every.max(Duration::from_secs(2));
    let (mut probe_bytes, mut mbps) = (1u64 << 20, 0.0f32);
    let (mut probed, mut probed_kind) = (None::<Instant>, None);
    loop {
        let Ok(Ok(connection)) = tokio::time::timeout(patience * 2, shared.endpoint.connect(member, PROBE_ALPN)).await
        else {
            tokio::time::sleep(patience * 5).await;
            continue;
        };
        let mut ticks = tokio::time::interval(every);
        loop {
            ticks.tick().await;
            let Ok(Ok(rtt)) = tokio::time::timeout(patience, links::ping(&connection)).await else {
                break;
            };
            let kind = links::kind(&connection);
            if probed.is_none_or(|at| at.elapsed() >= BANDWIDTH_EVERY) || probed_kind != Some(kind) {
                if let Ok(Ok(measured)) =
                    tokio::time::timeout(Duration::from_secs(10), links::bandwidth(&connection, probe_bytes)).await
                {
                    #[allow(clippy::cast_possible_truncation)] // megabits per second fit an f32
                    let measured_mbps = measured.mbps as f32;
                    mbps = measured_mbps;
                    if measured.took < Duration::from_millis(200) {
                        probe_bytes = (probe_bytes * 4).min(links::MAX_PROBE_BYTES);
                    } else if measured.took > Duration::from_secs(1) {
                        probe_bytes = (probe_bytes / 2).max(256 << 10);
                    }
                }
                (probed, probed_kind) = (Some(Instant::now()), Some(kind));
            }
            let sample = v1::LinkSample {
                peer: Some(node_id(member)),
                kind: kind as i32,
                rtt_ms: rtt.as_secs_f32() * 1000.0,
                bandwidth_mbps: mbps,
            };
            shared.links.lock().expect("links").insert(member, (sample, Instant::now()));
        }
        connection.close(0u32.into(), b"probing again");
    }
}

// Sends this node's device once and its telemetry every `every`.
// The tokens this device computed since local midnight.
struct Today {
    day: chrono::NaiveDate,
    // the process's count when the day began
    base: u64,
}

impl Today {
    fn new() -> Self {
        Self { day: chrono::Local::now().date_naive(), base: crate::ring::TOKENS_COMPUTED.load(Ordering::Relaxed) }
    }

    fn tokens(&mut self) -> u64 {
        let total = crate::ring::TOKENS_COMPUTED.load(Ordering::Relaxed);
        let day = chrono::Local::now().date_naive();
        if day != self.day {
            *self = Self { day, base: total };
        }
        total - self.base
    }
}

// This device's windows in the plan, with the mean time a token takes in them so far.
fn stage_loads(shared: &Shared) -> Vec<v1::StageLoad> {
    let tokens = crate::ring::TOKENS_COMPUTED.load(Ordering::Relaxed);
    let micros = crate::ring::MICROS_COMPUTING.load(Ordering::Relaxed);
    #[allow(clippy::cast_precision_loss)]
    let ms_per_token = if tokens == 0 { 0.0 } else { micros as f32 / tokens as f32 / 1000.0 };
    let mut loads = shared.stages.lock().expect("stages").clone();
    for load in &mut loads {
        load.ms_per_token = ms_per_token;
    }
    loads
}

async fn speak(shared: Arc<Shared>, mut device: v1::DeviceInfo, every: Duration) {
    let mut today = Today::new();
    let mut system = sysinfo::System::new();
    let mut sensors = crate::sensors::Sensors::new();
    let mut ticks = tokio::time::interval(every);
    let mut since_announced = 0u32;
    loop {
        let woken = tokio::select! {
            _ = ticks.tick() => false,
            () = shared.announce.notified() => true,
        };
        // the device goes out at the start, when its addresses or its relay change, when a neighbor comes up, and now
        // and then
        let addrs: Vec<String> = own_addrs(&shared.endpoint).iter().map(ToString::to_string).collect();
        let relay_url = shared.endpoint.addr().relay_urls().next().map(ToString::to_string).unwrap_or_default();
        let budget = shared.budget.lock().expect("budget").clone();
        since_announced += 1;
        if woken
            || since_announced >= ANNOUNCE_EVERY
            || addrs != device.direct_addresses
            || relay_url != device.relay_url
            || budget != device.budget
            || !shared.devices.lock().expect("devices").contains_key(&shared.key.public())
        {
            since_announced = 0;
            device.direct_addresses = addrs;
            device.relay_url = relay_url;
            device.budget = budget;
            shared.devices.lock().expect("devices").insert(shared.key.public(), device.clone());
            let _ = shared.events.send(Event::Device(shared.key.public(), device.clone()));
            let payload = v1::GossipPayload { kind: Some(v1::gossip_payload::Kind::Device(device.clone())) };
            let _ = shared.sender.broadcast(gossip::seal(&shared.key, &payload, now_unix_ms()).into()).await;
        }
        if woken {
            continue;
        }
        system.refresh_cpu_usage();
        system.refresh_memory();
        // links measured within the last few intervals; a member that stopped answering drops out
        let fresh = every * 5 + Duration::from_secs(2);
        let links = shared
            .links
            .lock()
            .expect("links")
            .values()
            .filter(|(_, at)| at.elapsed() < fresh)
            .map(|(sample, _)| sample.clone())
            .collect();
        let telemetry = v1::Telemetry {
            node: Some(node_id(shared.key.public())),
            unix_ms: now_unix_ms(),
            cpu_load: system.global_cpu_usage() / 100.0,
            memory_used_bytes: system.used_memory(),
            thermal: sensors.thermal() as i32,
            power: crate::sensors::power(),
            links,
            memory_offered_bytes: crate::budget::offered_memory_now(
                shared.budget.lock().expect("budget").as_ref(),
                system.total_memory(),
            ),
            stages: stage_loads(&shared),
            tokens_today: today.tokens(),
            fetching: shared.fetching.lock().expect("fetching").clone(),
        };
        // this device's own telemetry reaches the panel the same way as the others'
        shared.telemetry.lock().expect("telemetry").insert(shared.key.public(), telemetry.clone());
        let _ = shared.events.send(Event::Telemetry(shared.key.public(), telemetry.clone()));
        let payload = v1::GossipPayload { kind: Some(v1::gossip_payload::Kind::Telemetry(telemetry)) };
        let _ = shared.sender.broadcast(gossip::seal(&shared.key, &payload, now_unix_ms()).into()).await;
    }
}

// Handles what the gossip topic brings.
async fn listen(shared: Arc<Shared>, mut receiver: GossipReceiver) {
    while let Some(event) = receiver.next().await {
        match event {
            Ok(GossipEvent::Received(message)) => {
                if let Ok(heard) = gossip::open(&message.content, &shared.members) {
                    hear(&shared, heard);
                }
            }
            Ok(GossipEvent::NeighborUp(id)) => {
                shared.announce.notify_one();
                let _ = shared.events.send(Event::Neighbor { id, up: true });
            }
            Ok(GossipEvent::NeighborDown(id)) => {
                let _ = shared.events.send(Event::Neighbor { id, up: false });
            }
            Ok(GossipEvent::Lagged) => {}
            Err(_) => break,
        }
    }
}

fn hear(shared: &Shared, heard: gossip::Heard) {
    use v1::gossip_payload::Kind;
    match heard.payload.kind {
        Some(Kind::Telemetry(telemetry)) => {
            shared.telemetry.lock().expect("telemetry").insert(heard.sender, telemetry.clone());
            let _ = shared.events.send(Event::Telemetry(heard.sender, telemetry));
        }
        Some(Kind::Device(device)) => {
            let addrs: Vec<SocketAddr> = device.direct_addresses.iter().filter_map(|addr| addr.parse().ok()).collect();
            shared.lookup.add_endpoint_info(reach(heard.sender, &addrs, &device.relay_url, &shared.relays));
            let _ = shared.store.update_member(heard.sender, Some(device.clone()), Some(addrs));
            shared.devices.lock().expect("devices").insert(heard.sender, device.clone());
            let _ = shared.events.send(Event::Device(heard.sender, device));
        }
        Some(Kind::Member(member)) => admit_announced(shared, &member),
        // the computers keep the plan in their log; the others take it from them
        Some(Kind::Plan(plan)) if shared.consensus.is_none() => run::adopt(shared, plan),
        Some(Kind::Leave(_)) => {
            let _ = shared.events.send(Event::MemberLeft(heard.sender));
        }
        Some(Kind::Plan(_)) | None => {}
    }
}

// Tells the members that keep no log the plan the computers agreed on.
async fn spread_plan(shared: &Shared, plan: &v1::PlacementPlan) {
    let payload = v1::GossipPayload { kind: Some(v1::gossip_payload::Kind::Plan(plan.clone())) };
    let _ = shared.sender.broadcast(gossip::seal(&shared.key, &payload, now_unix_ms()).into()).await;
}

// A member announced by an admin: let it in once its token proves it.
fn admit_announced(shared: &Shared, member: &v1::Member) {
    let Some(id) = member.id.as_ref().and_then(|id| <[u8; 32]>::try_from(id.ed25519_public_key.as_slice()).ok()) else {
        return;
    };
    let Ok(id) = EndpointId::from_bytes(&id) else { return };
    // a member the cluster removed stays out, whoever announces it
    let agreed = shared.consensus.as_ref().map(|c| c.agreed().borrow().clone()).unwrap_or_default();
    if agreed.removed.contains(&id.to_string()) {
        return;
    }
    let revoked = |rid: &[u8]| agreed.revoked.contains(&hex(rid));
    if cluster::verify(shared.cluster.id, id, &member.membership_token, revoked).is_err() {
        return;
    }
    let addrs: Vec<SocketAddr> = member.direct_addresses.iter().filter_map(|addr| addr.parse().ok()).collect();
    let relay_url = member.device.as_ref().map_or("", |device| device.relay_url.as_str());
    shared.lookup.add_endpoint_info(reach(id, &addrs, relay_url, &shared.relays));
    let _ = shared.store.put_member(&Member {
        id,
        device: member.device.clone().unwrap_or_default(),
        token: member.membership_token.clone(),
        joined_unix_ms: member.joined_unix_ms,
        addrs,
    });
    // what the device said when it joined stands until it speaks for itself
    if let Some(device) = member.device.as_ref().filter(|device| !device.name.is_empty()) {
        shared.devices.lock().expect("devices").entry(id).or_insert_with(|| device.clone());
    }
    if shared.members.insert(id) {
        let _ = shared.events.send(Event::MemberJoined(id));
    }
}

// The join protocol of the service: admits through the open invitation, asking the owner through events.
#[derive(Debug, Clone)]
struct JoinHandler(Arc<OnceLock<Arc<Shared>>>);

impl ProtocolHandler for JoinHandler {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let Some(shared) = self.0.get() else { return Ok(()) };
        let invitation = shared.invitation.lock().expect("invitation").clone();
        let (Some(invitation), Some(key)) = (invitation, shared.cluster_key.as_ref()) else {
            connection.close(404u32.into(), b"no open invitation");
            return Ok(());
        };
        let members = shared.members.list();
        let name = shared.name.lock().expect("name").clone();
        let context = ClusterContext { key, name: &name, members: &members };
        let ask = |attempt: JoinAttempt| {
            let (tx, rx) = oneshot::channel();
            shared.answers.lock().expect("answers").insert(attempt.joiner, tx);
            let _ = shared.events.send(Event::JoinAttempt(attempt));
            async move { rx.await.unwrap_or(false) }
        };
        let Ok(Some(admitted)) = join::admit(connection, &invitation, &context, now_unix_ms, ask).await else {
            return Ok(());
        };
        let member = v1::Member {
            id: Some(node_id(admitted.joiner)),
            device: Some(admitted.device.clone()),
            membership_token: admitted.token.clone(),
            joined_unix_ms: now_unix_ms(),
            direct_addresses: admitted.device.direct_addresses.clone(),
        };
        admit_announced(shared, &member);
        let announcement = v1::GossipPayload { kind: Some(v1::gossip_payload::Kind::Member(member)) };
        let _ = shared.sender.broadcast(gossip::seal(&shared.key, &announcement, now_unix_ms()).into()).await;
        // the computers of the cluster agree on the new member
        if let Some(consensus) = shared.consensus.clone() {
            let command = Command::AddMember { id: hex(admitted.joiner.as_bytes()), token: admitted.token };
            tokio::spawn(async move {
                let _ = consensus.propose(command).await;
            });
        }
        Ok(())
    }
}
