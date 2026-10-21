//! `omnarc-node` — the agent that runs on every device.
//!
//! - `omnarc-node info` describes this device in the same shape (`omnarc.v1.DeviceInfo`) that it announces to a
//!   cluster.
//! - `omnarc-node id` prints this node's id, the public half of its key.
//! - `omnarc-node ring` runs windows of a model as one node of a ring of devices given by hand (stage B1).
//! - `omnarc-node create`, `invite`, `join` and `members` form a cluster (stage B2, `protocol/CLUSTER.md`).
//! - `omnarc-node serve` runs the device's service: gossip with the cluster, and devices invited while it runs.
//! - `omnarc-node model` downloads a model once, adds, lists, fetches and exports models: a window of layers travels
//!   from another member, not from the internet (`protocol/CLUSTER.md` §4).

use std::io::Write as _;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::{Parser, Subcommand};
use omnarc_engine_host::{Backend, DType};
use omnarc_network::ticket::Ticket;
use omnarc_network::{EndpointId, Peer};
use omnarc_node::pairing::{self, InviteOptions};
use omnarc_node::ring;
use omnarc_node::state::Store;
use omnarc_protocol::v1::{DeviceInfo, Platform};
use omnarc_store::Window;
use sysinfo::System;

#[derive(Debug, Parser)]
#[command(name = "omnarc-node", version, about = "The Omnarc agent that runs on every device")]
struct Cli {
    /// The node's data directory: its key, the cluster state (default: the user's data directory, omnarc).
    #[arg(long, global = true)]
    data: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Describe this device as it would announce itself to a cluster.
    Info {
        /// Print JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Print this node's id, creating its key if needed.
    Id {
        /// The key file (default: the user's data directory, omnarc/node.key).
        #[arg(long)]
        key: Option<PathBuf>,
    },
    /// Run windows of a model as one node of a ring. The node with a prompt is the head: it holds layer 0 and the
    /// output head and generates; the others serve until stopped.
    Ring(Box<RingArgs>),
    /// Create a cluster with this device as its first member and admin.
    Create {
        /// The cluster's name.
        #[arg(long, default_value = "Home cluster")]
        name: String,
        /// A relay for devices on other networks: `https://relay.example.com`, with `?token=SECRET` when it wants one;
        /// may be given more than once.
        #[arg(long)]
        relay: Vec<String>,
    },
    /// The relays this device meets the cluster's devices on other networks through.
    #[command(subcommand)]
    Relay(RelayCommand),
    /// Run this device's service whenever the device is on: a systemd user unit, a launch agent, or a task at sign-in.
    #[command(subcommand)]
    Service(ServiceCommand),
    /// Invite one device: show a ticket as a QR code, then confirm the four symbols when the device asks to join.
    /// While `serve` runs, use `serve --invite` instead.
    Invite {
        /// Where this node listens for the new device: the port `serve` uses, so the ticket stays right for it.
        #[arg(long, default_value = "0.0.0.0:7700")]
        listen: SocketAddr,
        /// Minutes the ticket stays valid, at most 10.
        #[arg(long, default_value_t = 10)]
        minutes: u64,
        /// A relay for devices on other networks; may be given more than once.
        #[arg(long)]
        relay: Vec<String>,
    },
    /// Join a cluster with the ticket another device shows.
    Join {
        /// The ticket: the text under the QR code, starting with OMNARC:.
        ticket: String,
        /// This device's name in the cluster (default: its host name).
        #[arg(long)]
        name: Option<String>,
    },
    /// List the members of this device's cluster.
    Members,
    /// Remove a member from the cluster: the computers of the cluster agree, then let it in no more. Admins only;
    /// this device's service must run.
    Remove {
        /// The member: its device name, or the start of its id.
        member: String,
        /// The control API of this device's service.
        #[arg(long, default_value = "127.0.0.1:7600")]
        api: SocketAddr,
    },
    /// Run this device's service: talk to the cluster and show what its devices do; Ctrl-C stops it.
    Serve {
        /// Where the service listens.
        #[arg(long, default_value = "0.0.0.0:7700")]
        listen: SocketAddr,
        /// Open an invitation for one device right away and show its QR code.
        #[arg(long)]
        invite: bool,
        /// Where the control API and the panel listen; only this device can reach the default.
        #[arg(long, default_value = "127.0.0.1:7600")]
        api: SocketAddr,
        /// The panel's built files (web/apps/panel/build), served at the API's address.
        #[arg(long)]
        panel: Option<PathBuf>,
    },
    /// Models on this device: add a file, list them, fetch a window of layers from another member, export it.
    #[command(subcommand)]
    Model(ModelCommand),
}

#[derive(Debug, Subcommand)]
enum ServiceCommand {
    /// Install it and start it now.
    Install {
        /// Where the service listens.
        #[arg(long, default_value = "0.0.0.0:7700")]
        listen: SocketAddr,
        /// Where the control API and the panel listen.
        #[arg(long, default_value = "127.0.0.1:7600")]
        api: SocketAddr,
        /// The panel's built files (web/apps/panel/build).
        #[arg(long)]
        panel: Option<PathBuf>,
        /// Show what would be done, and do nothing.
        #[arg(long)]
        print: bool,
    },
    /// Stop it and remove it.
    Uninstall {
        /// Show what would be done, and do nothing.
        #[arg(long)]
        print: bool,
    },
}

#[derive(Debug, Subcommand)]
enum RelayCommand {
    /// List them.
    List,
    /// Add one: `https://relay.example.com`, with `?token=SECRET` when it wants one; `serve` uses it from its next start.
    Add {
        /// The relay's address.
        url: String,
    },
    /// Remove one.
    Remove {
        /// The relay's address, with or without its token.
        url: String,
    },
}

#[derive(Debug, Subcommand)]
enum ModelCommand {
    /// Download a model once, from Hugging Face, check it against its published SHA-256 and add it; the other members
    /// then fetch its layers from this device. A download that broke off resumes. Stop `serve` on this device first.
    Pull {
        /// The repository, such as bartowski/Meta-Llama-3.1-8B-Instruct-GGUF.
        repo: String,
        /// The file in it, such as Meta-Llama-3.1-8B-Instruct-Q4_K_M.gguf.
        file: String,
        /// Where the file is kept (default: the node's data directory, downloads).
        #[arg(long)]
        to: Option<PathBuf>,
        /// Another source that answers like Hugging Face.
        #[arg(long, default_value = omnarc_node::download::HUGGING_FACE)]
        source: String,
    },
    /// Add a GGUF file. It stays where it is — do not change it afterwards — and the other members can fetch it
    /// while `serve` runs.
    Add {
        /// The GGUF file.
        path: PathBuf,
    },
    /// List the models on this device, whole or in part.
    List,
    /// Fetch a window of layers of a model another member has; only those bytes travel, and an interrupted fetch
    /// resumes. Stop `serve` on this device first.
    Fetch {
        /// The model's hash, as `model list` shows it on the device that has it.
        hash: omnarc_store::Hash,
        /// The window of layers, e.g. 0:40; layer N of an N-layer model is the output head.
        #[arg(long)]
        layers: Window,
        /// The member to fetch from, by name or the start of its id (default: each member in turn).
        #[arg(long)]
        from: Option<String>,
        /// The name to keep the model under (default: the name in its header).
        #[arg(long)]
        name: Option<String>,
    },
    /// Write a window of a model to a file the engine loads: the model's full length, holes outside the window.
    Export {
        /// The model's name or hash.
        model: String,
        /// The window of layers, e.g. 0:40.
        #[arg(long)]
        layers: Window,
        /// The file to write.
        #[arg(long)]
        out: PathBuf,
    },
}

#[derive(Debug, clap::Args)]
struct RingArgs {
    /// The GGUF model; every node of the ring uses the same file.
    #[arg(long)]
    model: PathBuf,
    /// This node's windows in ring order, e.g. `0:20,40:60,80:81`; layer N of an N-layer model is the output head.
    #[arg(long, value_delimiter = ',', required = true)]
    layers: Vec<ring::Window>,
    /// The next node of the ring: `<node id>@<ip:port>`.
    #[arg(long)]
    next: Peer,
    /// The ids connections are accepted from; give it more than once for a replica that must also accept
    /// the node it stands in for. Connections from anyone else are refused.
    #[arg(long, required = true)]
    prev: Vec<EndpointId>,
    /// The device that takes the next node's place if it leaves: `<node id>@<ip:port>`.
    #[arg(long)]
    next_replica: Option<Peer>,
    /// Where this node listens for the previous node.
    #[arg(long, default_value = "0.0.0.0:7700")]
    listen: SocketAddr,
    /// Only mirror: run the frames a live node sends to keep this node's KV cache in step, and connect to no
    /// next node. A node started this way is the replica of the windows it holds.
    #[arg(long)]
    mirror_only: bool,
    /// The device holding a replica of this node's windows: `<node id>@<ip:port>`. It is sent a copy of every
    /// frame this node receives, so it can carry an answer on from the same token if this node leaves.
    #[arg(long)]
    replica: Option<Peer>,
    /// The prompt; given only on the head.
    #[arg(long)]
    prompt: Option<String>,
    /// Tokens to generate (head).
    #[arg(long, default_value_t = 64)]
    generate: u32,
    /// Backend for this node's layers: auto, cpu, cuda, hip, vulkan, metal, opencl, sycl.
    #[arg(long, default_value = "auto", value_parser = parse_backend)]
    backend: Backend,
    /// KV cache capacity in tokens.
    #[arg(long, default_value_t = 4096)]
    max_tokens: u32,
    /// Prompt tokens per prefill step.
    #[arg(long, default_value_t = 256)]
    chunk: u32,
    /// The KV cache's element type: f16, q8 or q4 (`q8_0`, `q4_0`); the same on every node.
    #[arg(long, default_value = "f16", value_parser = parse_kv)]
    kv: omnarc_engine_host::KvType,
    /// Threads for this node's layers; 0: all physical cores.
    #[arg(long, default_value_t = 0)]
    threads: u32,
    /// Head: tokens guessed ahead and checked by the ring in one round (greedy sampling only); 0: none.
    #[arg(long, default_value_t = 4)]
    draft: u32,
    /// Element type of the hidden state between nodes: f16, bf16, f32 or f8; the same on every node.
    #[arg(long, default_value = "f16", value_parser = parse_wire)]
    wire: DType,
    /// The key file (default: the user's data directory, omnarc/node.key).
    #[arg(long)]
    key: Option<PathBuf>,
    /// The omnarc-engine executable (default: next to omnarc-node).
    #[arg(long)]
    engine: Option<PathBuf>,
}

fn parse_kv(name: &str) -> Result<omnarc_engine_host::KvType, String> {
    omnarc_engine_host::KvType::from_name(name)
        .ok_or_else(|| format!("unknown KV cache type {name}: f16, q8_0 or q4_0"))
}

fn parse_backend(name: &str) -> Result<Backend, String> {
    Ok(match name {
        "auto" => Backend::Unspecified,
        "cpu" => Backend::Cpu,
        "cuda" => Backend::Cuda,
        "hip" => Backend::Hip,
        "vulkan" => Backend::Vulkan,
        "metal" => Backend::Metal,
        "opencl" => Backend::Opencl,
        "sycl" => Backend::Sycl,
        other => return Err(format!("unknown backend {other}")),
    })
}

fn parse_wire(name: &str) -> Result<DType, String> {
    match name {
        "f16" => Ok(DType::F16),
        "bf16" => Ok(DType::Bf16),
        "f32" => Ok(DType::F32),
        // 8 bits and a scale per token: half of f16's bytes on every link (EN-19)
        "f8" => Ok(DType::F8E4M3),
        other => Err(format!("unknown wire format {other}; use f16, bf16, f32 or f8")),
    }
}

fn ring(args: RingArgs, data: &Path) -> Result<(), ring::BoxError> {
    let config = ring::Config {
        model: args.model,
        windows: args.layers,
        listen: args.listen,
        next: args.next,
        next_replica: args.next_replica,
        prev: args.prev,
        key: args.key.unwrap_or_else(|| data.join("node.key")),
        engine: args.engine.unwrap_or_else(ring::default_engine_path),
        backend: args.backend,
        max_tokens: args.max_tokens,
        chunk: args.chunk.max(1),
        kv: args.kv,
        wire: args.wire,
        mirror_only: args.mirror_only,
        replica: args.replica,
        threads: args.threads,
        draft: args.draft,
        prompt: args.prompt,
        generate: args.generate,
    };
    let runtime = tokio::runtime::Runtime::new()?;
    if config.prompt.is_none() {
        return runtime.block_on(ring::serve(config));
    }
    let report = runtime.block_on(ring::generate(config))?;
    eprintln!(
        "omnarc: {} prompt tokens, {} generated; first token after {:.0} ms",
        report.prompt_tokens,
        report.generated.len(),
        report.ttft.as_secs_f64() * 1e3
    );
    if let Some(tpot) = report.tpot {
        eprintln!(
            "omnarc: {:.1} ms per output token ({:.2} tokens/s)",
            tpot.as_secs_f64() * 1e3,
            1.0 / tpot.as_secs_f64()
        );
    }
    Ok(())
}

fn platform() -> Platform {
    if cfg!(target_os = "android") {
        Platform::Android
    } else if cfg!(target_os = "ios") {
        Platform::Ios
    } else if cfg!(target_os = "linux") {
        Platform::Linux
    } else if cfg!(target_os = "windows") {
        Platform::Windows
    } else if cfg!(target_os = "macos") {
        Platform::Macos
    } else {
        Platform::Unspecified
    }
}

fn describe() -> DeviceInfo {
    let mut sys = System::new();
    sys.refresh_cpu_all();
    sys.refresh_memory();
    let cpu_model = sys.cpus().first().map(|c| c.brand().trim().to_owned()).unwrap_or_default();
    // cores, not the threads the processor runs on them
    let cores = System::physical_core_count().unwrap_or_else(|| sys.cpus().len());
    DeviceInfo {
        name: System::host_name().unwrap_or_default(),
        platform: platform() as i32,
        os_version: System::long_os_version().unwrap_or_default(),
        cpu_model,
        cpu_cores: u32::try_from(cores).unwrap_or(u32::MAX),
        memory_total_bytes: sys.total_memory(),
        accelerators: accelerators(&ring::default_engine_path()),
        agent_version: env!("CARGO_PKG_VERSION").to_owned(),
        ..DeviceInfo::default()
    }
}

/// The GPUs the engine at `engine` can run layers on, as the engine itself names them: a device says it computes
/// with what Omnarc can use, not with every graphics card it has. None without an engine, or when the engine does
/// not answer within ten seconds (a GPU driver can hang; the service starts all the same).
fn accelerators(engine: &Path) -> Vec<omnarc_protocol::v1::Accelerator> {
    if !engine.is_file() {
        return Vec::new();
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let engine = engine.to_path_buf();
    std::thread::spawn(move || {
        let devices = omnarc_engine_host::Engine::spawn(&engine, 1 << 16).and_then(|mut e| {
            let devices = e.devices();
            let _ = e.quit();
            devices
        });
        let _ = tx.send(devices);
    });
    match rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(devices)) => devices
            .into_iter()
            .map(|device| omnarc_protocol::v1::Accelerator {
                backend: device.backend as i32,
                name: device.name,
                memory_bytes: device.memory_bytes,
                ..Default::default()
            })
            .collect(),
        _ => Vec::new(),
    }
}

#[allow(clippy::cast_precision_loss)] // display only
fn gib(bytes: u64) -> f64 {
    bytes as f64 / f64::from(1u32 << 30)
}

fn data_dir(data: Option<PathBuf>) -> PathBuf {
    data.unwrap_or_else(|| ring::default_key_path().parent().map(Path::to_path_buf).unwrap_or_default())
}

fn qr_code(text: &str) -> String {
    qrcode::QrCode::new(text.as_bytes()).map_or_else(
        |_| String::new(),
        |code| code.render::<qrcode::render::unicode::Dense1x2>().quiet_zone(true).build(),
    )
}

fn platform_name(platform: i32) -> &'static str {
    match Platform::try_from(platform).unwrap_or(Platform::Unspecified) {
        Platform::Linux => "Linux",
        Platform::Windows => "Windows",
        Platform::Macos => "macOS",
        Platform::Android => "Android",
        Platform::Ios => "iOS",
        Platform::Browser => "Browser",
        Platform::Unspecified => "—",
    }
}

fn ask_owner(attempt: &omnarc_network::join::JoinAttempt) -> bool {
    println!();
    println!("{} ({}) asks to join.", attempt.device.name, platform_name(attempt.device.platform));
    println!("Its screen shows:  {}", pairing::format_emoji(&attempt.emoji));
    print!("Do the symbols match? [y/N] ");
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer).is_ok() && answer.trim().eq_ignore_ascii_case("y")
}

fn print_ticket(ticket: &Ticket) {
    let text = ticket.encode();
    println!("{}", qr_code(&text));
    println!("On the new device run:\n\n  omnarc-node join {text}\n");
}

async fn serve(
    data: &Path,
    listen: SocketAddr,
    invite: bool,
    api: SocketAddr,
    panel: Option<PathBuf>,
) -> Result<(), ring::BoxError> {
    use omnarc_node::service::{Config, Event, Service};
    // a device in no cluster yet: the panel creates one or joins one, then the service starts
    if Store::open(data)?.cluster()?.is_none() {
        let created = setup(data, api, panel.clone()).await?;
        if !created {
            return Ok(());
        }
    }
    let config = Config {
        data: data.to_path_buf(),
        listen,
        device: describe(),
        telemetry_every: Duration::from_secs(1),
        lan: true,
    };
    let service = Service::start(config).await?;
    let mut events = service.events();
    let listener = tokio::net::TcpListener::bind(api).await.map_err(|e| format!("the control API on {api}: {e}"))?;
    let has_panel = panel.is_some();
    tokio::spawn(axum::serve(listener, omnarc_api::router(service.api(), panel)).into_future());
    if has_panel {
        println!("The panel: http://{api}");
    }
    println!(
        "{} — {} members. This device is {}.",
        service.cluster().name,
        service.members().len(),
        service.id().fmt_short()
    );
    if invite {
        print_ticket(&service.invite(Duration::from_secs(600), vec![])?);
        println!("Waiting for the device (the ticket is valid for 10 min)...");
    }
    let names = |id: EndpointId, service: &Service| {
        service.devices().get(&id).map_or_else(|| id.fmt_short().to_string(), |device| device.name.clone())
    };
    let mut summary = tokio::time::interval(Duration::from_secs(10));
    // each other device is reported once, when it first tells what it is
    let mut announced = std::collections::HashSet::new();
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            _ = summary.tick() => {
                let devices = service.devices();
                let telemetry = service.telemetry();
                let links = telemetry.get(&service.id()).map(|t| t.links.clone()).unwrap_or_default();
                for (id, telemetry) in &telemetry {
                    let name = devices.get(id).map_or_else(|| id.fmt_short().to_string(), |device| device.name.clone());
                    let link = links
                        .iter()
                        .find(|link| link.peer.as_ref().is_some_and(|p| p.ed25519_public_key == id.as_bytes()))
                        .map(describe_link)
                        .unwrap_or_default();
                    println!("  {:<24} CPU {:>3.0} %   memory {:.1} GiB   {link}", name, telemetry.cpu_load * 100.0, gib(telemetry.memory_used_bytes));
                }
            }
            event = events.recv() => match event {
                Ok(Event::Device(id, device)) => {
                    if id != service.id() && announced.insert(id) {
                        println!("{} ({}) is in the cluster.", device.name, platform_name(device.platform));
                    }
                }
                Ok(Event::MemberJoined(id)) => println!("A new device joined: {}.", names(id, &service)),
                Ok(Event::MemberLeft(id)) => println!("{} left.", names(id, &service)),
                Ok(Event::MemberRemoved(id)) => println!("{} was removed from the cluster.", id.fmt_short()),
                Ok(Event::ClusterChanged) => println!("The cluster's name or relays changed."),
                Ok(Event::PlanChanged(plan)) if plan.stages.is_empty() => println!("The model stopped."),
                Ok(Event::PlanChanged(plan)) => println!(
                    "Plan {}: {} on {} device(s).",
                    plan.version,
                    plan.model.as_ref().map_or("a model", |m| m.name.as_str()),
                    plan.stages.len()
                ),
                Ok(Event::Neighbor { id, up }) => println!("{} {}.", names(id, &service), if up { "connected" } else { "disconnected" }),
                Ok(Event::JoinAttempt(attempt)) => {
                    // the owner answers here when someone sits at this terminal, otherwise in the panel
                    if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
                        let joiner = attempt.joiner;
                        let matches = tokio::task::spawn_blocking(move || ask_owner(&attempt)).await.unwrap_or(false);
                        if !service.answer_join(joiner, matches) {
                            println!("The device stopped waiting before the answer.");
                        }
                    } else {
                        println!("{} asks to join: compare the symbols in the panel.", attempt.device.name);
                    }
                }
                Ok(Event::Telemetry(..)) | Err(_) => {}
            },
        }
    }
    println!("Stopping.");
    service.shutdown().await;
    Ok(())
}

/// A link as the summary shows it: `Wi-Fi 3 ms 420 Mbit/s`.
fn describe_link(link: &omnarc_protocol::v1::LinkSample) -> String {
    use omnarc_protocol::v1::LinkKind;
    let kind = match LinkKind::try_from(link.kind).unwrap_or(LinkKind::Unspecified) {
        LinkKind::Wifi => "Wi-Fi ",
        LinkKind::Ethernet => "Ethernet ",
        LinkKind::Usb => "USB ",
        LinkKind::Thunderbolt => "Thunderbolt ",
        LinkKind::Relay => "relay ",
        LinkKind::Webrtc => "WebRTC ",
        LinkKind::Unspecified => "",
    };
    let bandwidth =
        if link.bandwidth_mbps > 0.0 { format!(" {:.0} Mbit/s", link.bandwidth_mbps) } else { String::new() };
    format!("{kind}{:.1} ms{bandwidth}", link.rtt_ms)
}

/// Removes a member through the running service, which keeps the cluster's Raft.
async fn remove(member: &str, api: SocketAddr) -> Result<(), ring::BoxError> {
    use omnarc_api::client::call;
    use omnarc_protocol::v1;
    let got: v1::GetClusterResponse = call(api, "GetCluster", &v1::GetClusterRequest {}).await?;
    let devices = got.cluster.unwrap_or_default().devices;
    let hex = |id: &v1::NodeId| id.ed25519_public_key.iter().fold(String::new(), |out, b| out + &format!("{b:02x}"));
    let matches: Vec<&DeviceInfo> = devices
        .iter()
        .filter(|d| {
            d.name.eq_ignore_ascii_case(member)
                || (member.len() >= 4 && d.id.as_ref().is_some_and(|id| hex(id).starts_with(member)))
        })
        .collect();
    let device = match matches[..] {
        [one] => one,
        [] => return Err(format!("no member is called {member}").into()),
        _ => return Err(format!("{} members answer to {member}: give the start of its id", matches.len()).into()),
    };
    let request = v1::RemoveMemberRequest { member: device.id.clone() };
    let _: v1::RemoveMemberResponse = call(api, "RemoveMember", &request).await?;
    println!("Removed {}. The computers of the cluster let it in no more.", device.name);
    Ok(())
}

fn service_command(data: &Path, command: ServiceCommand) -> Result<(), ring::BoxError> {
    use omnarc_node::autostart;
    let (plan, print) = match command {
        ServiceCommand::Install { listen, api, panel, print } => {
            let exe = std::env::current_exe()?;
            let data = std::path::absolute(data)?;
            let mut args: Vec<String> = vec![
                "--data".into(),
                data.display().to_string(),
                "serve".into(),
                "--listen".into(),
                listen.to_string(),
                "--api".into(),
                api.to_string(),
            ];
            if let Some(panel) = panel {
                args.extend(["--panel".into(), std::path::absolute(panel)?.display().to_string()]);
            }
            (autostart::install(&exe, &args)?, print)
        }
        ServiceCommand::Uninstall { print } => (autostart::uninstall()?, print),
    };
    if print {
        print!("{}", plan.describe());
        return Ok(());
    }
    plan.apply()?;
    println!("Done.");
    if !plan.note.is_empty() {
        println!("{}", plan.note);
    }
    Ok(())
}

fn relay_command(data: &Path, command: RelayCommand) -> Result<(), ring::BoxError> {
    use omnarc_network::relay::Relay;
    let store = Store::open(data)?;
    store.cluster()?.ok_or("this device belongs to no cluster")?;
    let mut relays = store.relays()?;
    // the same relay, whatever token is written with it
    let same = |a: &str, b: &str| a.parse::<Relay>().ok().map(|r| r.url) == b.parse::<Relay>().ok().map(|r| r.url);
    match command {
        RelayCommand::List => {
            if relays.is_empty() {
                println!("No relays: devices meet only on one network. Add one with omnarc-node relay add <url>.");
            }
            for relay in &relays {
                let parsed: Relay = relay.parse()?;
                let token = if parsed.token.is_some() { " (with a token)" } else { "" };
                println!("{}{token}", parsed.url);
            }
        }
        RelayCommand::Add { url } => {
            url.parse::<Relay>()?;
            relays.retain(|r| !same(r, &url));
            relays.push(url);
            store.set_relays(&relays)?;
            println!("Added. Restart omnarc-node serve to use it; new tickets name it.");
        }
        RelayCommand::Remove { url } => {
            let before = relays.len();
            relays.retain(|r| !same(r, &url));
            if relays.len() == before {
                return Err(format!("{url} is not a relay of this device").into());
            }
            store.set_relays(&relays)?;
            println!("Removed. Restart omnarc-node serve to stop using it.");
        }
    }
    Ok(())
}

/// Serves the panel and the setup API until this device creates or joins a cluster: whether it did, or was stopped.
async fn setup(data: &Path, api: SocketAddr, panel: Option<PathBuf>) -> Result<bool, ring::BoxError> {
    use std::future::IntoFuture as _;
    let setup = omnarc_node::setup::Setup::new(data, describe())?;
    let listener = tokio::net::TcpListener::bind(api).await.map_err(|e| format!("the control API on {api}: {e}"))?;
    if panel.is_some() {
        println!("This device belongs to no cluster yet. Create one or join one in the panel: http://{api}");
    } else {
        println!("This device belongs to no cluster yet: run omnarc-node create, or omnarc-node join <ticket>.");
    }
    let server = tokio::spawn(axum::serve(listener, omnarc_api::router(setup.clone(), panel)).into_future());
    let created = tokio::select! {
        () = setup.done() => true,
        _ = tokio::signal::ctrl_c() => false,
    };
    // the panel's open streams end with the server; it reconnects to the service on the same address
    server.abort();
    let _ = server.await;
    Ok(created)
}

fn cluster_command(data: &Path, command: Command) -> Result<(), ring::BoxError> {
    match command {
        Command::Create { name, relay } => {
            let cluster = pairing::create(data, &name, describe(), &relay)?;
            println!("Created {} (cluster {}).", cluster.name, &cluster.id.to_hex()[..16]);
            println!("Add a device with: omnarc-node invite");
        }
        Command::Invite { listen, minutes, relay } => {
            let options = InviteOptions { listen, ttl: Duration::from_secs(minutes.clamp(1, 10) * 60), relays: relay };
            let runtime = tokio::runtime::Runtime::new()?;
            let member = runtime.block_on(pairing::invite(
                data,
                options,
                |ticket: &Ticket| {
                    print_ticket(ticket);
                    println!("Waiting for the device (the ticket is valid for {} min)...", minutes.clamp(1, 10));
                },
                |attempt| {
                    let answer = ask_owner(&attempt);
                    async move { answer }
                },
            ))?;
            println!("{} joined the cluster.", member.device.name);
        }
        Command::Join { ticket, name } => {
            let ticket = Ticket::decode(&ticket)?;
            let mut device = describe();
            if let Some(name) = name {
                device.name = name;
            }
            let runtime = tokio::runtime::Runtime::new()?;
            let cluster = runtime.block_on(pairing::join(data, &ticket, device, |emoji| {
                println!("Check that the inviting device shows:  {}", pairing::format_emoji(&emoji));
                println!("Waiting for its owner to confirm...");
            }))?;
            println!("Joined {}.", cluster.name);
        }
        Command::Members => {
            let store = Store::open(data)?;
            let cluster = store.cluster()?.ok_or("this device belongs to no cluster")?;
            println!("{} — cluster {}", cluster.name, &cluster.id.to_hex()[..16]);
            for member in store.members()? {
                let name = if member.device.name.is_empty() { "(not heard from yet)" } else { &member.device.name };
                println!("  {:<24} {:<8} {}", name, platform_name(member.device.platform), member.id.fmt_short());
            }
        }
        Command::Serve { listen, invite, api, panel } => {
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(serve(data, listen, invite, api, panel))?;
        }
        Command::Relay(command) => relay_command(data, command)?,
        Command::Service(command) => service_command(data, command)?,
        Command::Remove { member, api } => {
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(remove(&member, api))?;
        }
        Command::Model(command) => {
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(model_command(data, command))?;
        }
        Command::Info { .. } | Command::Id { .. } | Command::Ring(_) => unreachable!("not a cluster command"),
    }
    Ok(())
}

/// A size to show: `42.52 GB`, `12.4 MB`, `3 KB`.
fn size(bytes: u64) -> String {
    #[allow(clippy::cast_precision_loss)] // a size to show
    let b = bytes as f64;
    if b >= 1e9 {
        format!("{:.2} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.1} MB", b / 1e6)
    } else {
        format!("{:.0} KB", b / 1e3)
    }
}

/// Prints `what` and how far it got, on one line, at most every half second.
fn progress_line(what: &str) -> impl FnMut(u64, Option<u64>) {
    let mut shown: Option<std::time::Instant> = None;
    let what = what.to_owned();
    move |done, total| {
        if shown.is_none_or(|at| at.elapsed() >= Duration::from_millis(500)) || total == Some(done) {
            shown = Some(std::time::Instant::now());
            match total {
                Some(total) => print!("\r{what} {} of {}   ", size(done), size(total)),
                None => print!("\r{what} {}   ", size(done)),
            }
            let _ = std::io::stdout().flush();
        }
    }
}

/// Downloads a model once and adds it to this device's store.
async fn pull_model(
    data: &Path,
    repo: &str,
    file: &str,
    to: Option<PathBuf>,
    source: &str,
) -> Result<(), ring::BoxError> {
    // the store is opened first: the service must not hold it
    let store = omnarc_node::models::open(data).await?;
    let dir = to.unwrap_or_else(|| data.join("downloads"));
    let pulled = omnarc_node::download::pull(source, repo, file, &dir, progress_line("Downloaded")).await;
    let pulled = match pulled {
        Ok(pulled) => pulled,
        Err(e) => {
            store.shutdown().await;
            return Err(e);
        }
    };
    println!();
    if pulled.verified {
        println!("{} matches its published SHA-256.", pulled.path.display());
    } else {
        println!("{} — the source published no SHA-256; it is {}.", pulled.path.display(), pulled.sha256);
    }
    let total = pulled.bytes;
    let mut line = progress_line("Hashing");
    let model = store.add(&pulled.path, |done| line(done, Some(total))).await;
    store.shutdown().await;
    let model = model?;
    println!();
    println!("Added {} — {} layers, {}.", model.name, model.layout.layers, size(model.layout.file_len));
    println!("Hash {}", model.hash);
    Ok(())
}

async fn model_command(data: &Path, command: ModelCommand) -> Result<(), ring::BoxError> {
    use omnarc_node::models;
    match command {
        ModelCommand::Pull { repo, file, to, source } => pull_model(data, &repo, &file, to, &source).await?,
        ModelCommand::Add { path } => {
            let total = std::fs::metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?.len();
            let store = models::open(data).await?;
            let mut line = progress_line("Hashing");
            let model = store.add(&path, |done| line(done, Some(total))).await;
            store.shutdown().await;
            let model = model?;
            println!();
            println!("Added {} — {} layers, {}.", model.name, model.layout.layers, size(model.layout.file_len));
            println!("Hash {}", model.hash);
            println!("The other members fetch its layers from this device while omnarc-node serve runs.");
        }
        ModelCommand::List => {
            let store = models::open(data).await?;
            let list = store.list().await;
            for (name, _) in list.as_deref().unwrap_or_default() {
                match store.model(name).await {
                    Ok(model) => {
                        let missing = store.missing(&model, model.layout.all()).await.unwrap_or(model.layout.file_len);
                        let have = if missing == 0 {
                            "whole".to_owned()
                        } else {
                            format!("{} here", size(model.layout.file_len.saturating_sub(missing)))
                        };
                        println!(
                            "{name:<40} {:>3} layers {:>10}  {have:<16} {}",
                            model.layout.layers,
                            size(model.layout.file_len),
                            model.hash
                        );
                    }
                    Err(e) => println!("{name:<40} {e}"),
                }
            }
            if list.as_ref().is_ok_and(Vec::is_empty) {
                println!("No models on this device yet: omnarc-node model add <file.gguf>");
            }
            store.shutdown().await;
            list?;
        }
        ModelCommand::Fetch { hash, layers, from, name } => {
            let mut sender = None;
            let mut line = progress_line("Fetched");
            let (model, from) = models::fetch(data, hash, layers, from.as_deref(), name.as_deref(), |id, done| {
                if sender != Some(id) {
                    sender = Some(id);
                    println!("Fetching the layers {layers} from {}...", id.fmt_short());
                }
                line(done, None);
            })
            .await?;
            println!();
            println!(
                "The layers {layers} of {} are on this device ({}), from {}.",
                model.name,
                size(model.layout.bytes(layers)?),
                from.fmt_short()
            );
            println!(
                "Write them for the engine: omnarc-node model export {} --layers {layers} --out <file>",
                model.name
            );
        }
        ModelCommand::Export { model, layers, out } => {
            let store = models::open(data).await?;
            let result = async {
                let model = store.model(&model).await?;
                store.export(&model, layers, &out).await?;
                Ok::<_, ring::BoxError>(model)
            }
            .await;
            store.shutdown().await;
            let model = result?;
            println!(
                "Wrote the layers {layers} of {} to {} ({} of data in a {} file).",
                model.name,
                out.display(),
                size(model.layout.bytes(layers)?),
                size(model.layout.file_len)
            );
            println!("Run them: omnarc-node ring --model {} --layers {layers} ...", out.display());
        }
    }
    Ok(())
}

fn main() {
    let cli = Cli::parse();
    let data = data_dir(cli.data);
    match cli.command {
        command @ (Command::Create { .. }
        | Command::Invite { .. }
        | Command::Join { .. }
        | Command::Members
        | Command::Serve { .. }
        | Command::Relay(_)
        | Command::Service(_)
        | Command::Remove { .. }
        | Command::Model(_)) => {
            if let Err(e) = cluster_command(&data, command) {
                eprintln!("omnarc-node: {e}");
                std::process::exit(1);
            }
        }
        Command::Id { key } => match ring::node_id(&key.unwrap_or_else(|| data.join("node.key"))) {
            Ok(id) => println!("{id}"),
            Err(e) => {
                eprintln!("omnarc-node: {e}");
                std::process::exit(1);
            }
        },
        Command::Ring(args) => {
            if let Err(e) = ring(*args, &data) {
                eprintln!("omnarc-node: {e}");
                std::process::exit(1);
            }
        }
        Command::Info { json } => {
            let info = describe();
            let platform = Platform::try_from(info.platform).unwrap_or(Platform::Unspecified);
            if json {
                let value = serde_json::json!({
                    "name": info.name,
                    "platform": platform.as_str_name(),
                    "os_version": info.os_version,
                    "cpu_model": info.cpu_model,
                    "cpu_cores": info.cpu_cores,
                    "memory_total_bytes": info.memory_total_bytes,
                    "accelerators": info.accelerators.iter().map(|unit| serde_json::json!({
                        "backend": Backend::try_from(unit.backend).unwrap_or(Backend::Unspecified).as_str_name(),
                        "name": unit.name,
                        "memory_bytes": unit.memory_bytes,
                    })).collect::<Vec<_>>(),
                    "agent_version": info.agent_version,
                });
                println!("{value:#}");
            } else {
                println!("Device       {}", info.name);
                println!("Platform     {}", platform.as_str_name());
                println!("OS           {}", info.os_version);
                println!("CPU          {} ({} cores)", info.cpu_model, info.cpu_cores);
                println!("Memory       {:.1} GiB", gib(info.memory_total_bytes));
                for unit in &info.accelerators {
                    let backend = Backend::try_from(unit.backend).unwrap_or(Backend::Unspecified);
                    println!("GPU          {} ({:?}, {:.1} GiB)", unit.name, backend, gib(unit.memory_bytes));
                }
                if info.accelerators.is_empty() {
                    println!("GPU          none that this engine can use: it computes on the processor");
                }
                println!("Agent        {}", info.agent_version);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_this_device() {
        let info = describe();
        assert!(info.cpu_cores >= 1);
        assert!(info.memory_total_bytes > 0);
        assert_ne!(info.platform, Platform::Unspecified as i32);
        assert_eq!(info.agent_version, env!("CARGO_PKG_VERSION"));
    }
}
