//! A cluster runs a model on its own (B3): a desktop, a laptop and a phone form a cluster, each owner names a
//! budget, and the desktop starts a model that fits on none of them alone. The placement splits it between the
//! desktop and the laptop within their budgets, leaves the phone out as too small, and every device takes its part
//! without being told more. A prompt asked on the phone is answered by the ring — exactly what one device holding the
//! whole model answers. Built only with the `engine-tests` feature; needs `OMNARC_ENGINE` and `OMNARC_TEST_MODEL`
//! (a small llama-architecture GGUF model, e.g. stories15M-q8_0.gguf).

use std::path::{Path, PathBuf};
use std::time::Duration;

use n0_future::StreamExt as _;
use omnarc_engine_host::{Backend, DType, Engine, Input, SamplerParams, Step};
use omnarc_node::pairing;
use omnarc_node::service::{Config, Event, Service};
use omnarc_protocol::v1;

const PROMPT: &str = "Once upon a time, there was a little girl named Lily.";
const GENERATE: u32 = 24;
const CONTEXT: u32 = 512;

fn env_path(name: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("set {name} to run the cluster test")))
}

fn data_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("omnarc-cluster-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn device(name: &str) -> v1::DeviceInfo {
    let platform = if name == "Phone" { v1::Platform::Android } else { v1::Platform::Linux };
    v1::DeviceInfo { name: name.into(), platform: platform as i32, cpu_cores: 2, ..Default::default() }
}

async fn start(data: &Path, name: &str) -> Service {
    let config = Config {
        data: data.to_path_buf(),
        listen: "127.0.0.1:0".parse().unwrap(),
        device: device(name),
        telemetry_every: Duration::from_millis(200),
        lan: false,
    };
    Service::start(config).await.unwrap_or_else(|e| panic!("starting {name}: {e}"))
}

async fn admit(admin: &Service, data: &Path, name: &str) {
    let ticket = admin.invite(Duration::from_secs(60), vec![]).unwrap();
    let mut events = admin.events();
    let joining = tokio::spawn({
        let (data, name) = (data.to_path_buf(), name.to_owned());
        async move { pairing::join(&data, &ticket, device(&name), |_| {}).await.map_err(|e| e.to_string()) }
    });
    loop {
        if let Ok(Event::JoinAttempt(attempt)) = events.recv().await {
            assert!(admin.answer_join(attempt.joiner, true));
            break;
        }
    }
    joining.await.unwrap().unwrap();
}

async fn eventually(what: &str, check: impl Fn() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
    while !check() {
        assert!(tokio::time::Instant::now() < deadline, "timed out waiting until {what}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// The text one device answers with the whole model, greedily.
fn answer_alone(engine_path: &Path, model: &Path) -> String {
    let mut engine = Engine::spawn(engine_path, 16 << 20).unwrap();
    let layers = engine.model_info(model).unwrap().layer_count;
    let whole = engine.open(model, 0..layers + 1, Backend::Cpu, CONTEXT).unwrap();
    let mut tokens = engine.tokenize(whole.id, PROMPT, true).unwrap();
    let sampler = engine.sampler_open(SamplerParams::default()).unwrap();
    let (mut position, mut text, mut generated) = (0, String::new(), 0);
    while generated < GENERATE {
        let step = Step { request: 1, position, input: Input::Tokens(&tokens), all_logits: false, output: DType::F32 };
        engine.run(&whole, &step).unwrap();
        position += u32::try_from(tokens.len()).unwrap();
        let token = engine.sample(sampler, whole.model.vocab_size).unwrap();
        generated += 1;
        let (piece, end) = engine.token_text(whole.id, token).unwrap();
        text.push_str(&piece);
        if end {
            break;
        }
        tokens = vec![token];
    }
    engine.quit().unwrap();
    text
}

fn budget(bytes: u64) -> v1::Budget {
    v1::Budget { memory_bytes: bytes, ..Default::default() }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn a_cluster_places_a_model_by_what_each_device_offers_and_answers_from_anywhere() {
    let engine = env_path("OMNARC_ENGINE");
    let model_file = env_path("OMNARC_TEST_MODEL");
    let expected = answer_alone(&engine, &model_file);

    let (a, b, c) = (data_dir("desktop"), data_dir("laptop"), data_dir("phone"));
    pairing::create(&a, "Home cluster", device("Desktop"), &[]).unwrap();
    let desktop = start(&a, "Desktop").await;
    admit(&desktop, &b, "Laptop").await;
    let laptop = start(&b, "Laptop").await;
    admit(&desktop, &c, "Phone").await;
    let phone = start(&c, "Phone").await;
    let model = desktop.models().add(&model_file, |_| {}).await.unwrap();

    // budgets that hold the model only together: the desktop half of the layers with the head's extras, the laptop
    // the other half, the phone less than one layer
    let shape = omnarc_node::service::shape(&model, CONTEXT, omnarc_engine_host::KvType::F16);
    let half = shape.layers / 2;
    let reserve = omnarc_node::service::ENGINE_RESERVE;
    let room = u64::midpoint(shape.layer_bytes, shape.kv_bytes_per_layer);
    let desktop_offer = reserve + shape.window_bytes(half, true) + room;
    let laptop_offer = reserve + shape.window_bytes(shape.layers - half, false) + room;
    assert!(desktop_offer - reserve < shape.whole_bytes() && laptop_offer - reserve < shape.whole_bytes());
    desktop.api().set_budget(budget(desktop_offer)).unwrap();
    laptop.api().set_budget(budget(laptop_offer)).unwrap();
    phone.api().set_budget(budget(reserve + shape.layer_bytes / 2)).unwrap();
    let offers = |id| {
        let telemetry = desktop.telemetry();
        let device = desktop.devices();
        telemetry.get(&id).map(|t| t.memory_offered_bytes)
            == Some(match id {
                id if id == desktop.id() => desktop_offer,
                id if id == laptop.id() => laptop_offer,
                _ => reserve + shape.layer_bytes / 2,
            })
            && device.get(&id).is_some_and(|d| d.accelerators.iter().any(|a| a.measured_bandwidth_gbps > 0.0))
    };
    eventually("the desktop hears every budget and profile", || {
        [desktop.id(), laptop.id(), phone.id()].into_iter().all(offers)
    })
    .await;

    let started = desktop
        .api()
        .run_model(v1::RunModelRequest { context: CONTEXT, ..Default::default() })
        .await
        .unwrap_or_else(|e| panic!("{}", e.message));
    let plan = started.plan.unwrap();
    let owner = |s: &v1::Stage| s.node.as_ref().unwrap().ed25519_public_key.clone();
    assert_eq!(plan.stages.len(), 2, "split between the desktop and the laptop: {plan:?}");
    assert_eq!(
        owner(&plan.stages[0]),
        desktop.id().as_bytes().to_vec(),
        "only the desktop has room for the head's extras"
    );
    assert_eq!(owner(&plan.stages[1]), laptop.id().as_bytes().to_vec());
    assert_eq!(started.left_out.len(), 1);
    assert_eq!(started.left_out[0].reason, "too_small");
    assert_eq!(started.left_out[0].node.as_ref().unwrap().ed25519_public_key, phone.id().as_bytes().to_vec());

    // nobody is told anything more: the laptop fetches its layers from the desktop and takes its place
    let working = |service: &Service| {
        let me = service.id();
        service.telemetry().get(&me).is_some_and(|t| t.stages.iter().any(|s| s.plan_version == plan.version))
    };
    eventually("the desktop and the laptop run their windows", || working(&desktop) && working(&laptop)).await;
    eventually("the phone hears the plan", || phone.api().snapshot().plans.iter().any(|p| p.version == plan.version))
        .await;

    // the phone holds no layer and still asks: the head answers
    let (text, done) =
        answer(&phone, v1::AskRequest { prompt: PROMPT.into(), max_tokens: GENERATE, ..Default::default() }).await;
    assert_eq!(text, expected, "the ring answers what one device holding the whole model answers");
    let done = done.expect("the answer ends with its timings");
    assert!(done.generated_tokens > 0 && done.ttft_ms > 0.0, "{done:?}");

    // a conversation: a model without a chat template answers its last message as plain text
    let messages = [("user", "Hello"), ("assistant", "Hi."), ("user", PROMPT)]
        .map(|(role, content)| v1::ChatMessage { role: role.into(), content: content.into() })
        .to_vec();
    let (chat, _) = answer(&phone, v1::AskRequest { messages, max_tokens: GENERATE, ..Default::default() }).await;
    assert_eq!(chat, expected);

    // accuracy mode: three sampled paths, and the answer most of them agree on
    let (_, done) =
        answer(&phone, v1::AskRequest { prompt: PROMPT.into(), max_tokens: 8, paths: 3, ..Default::default() }).await;
    let done = done.expect("accuracy mode ends with its count");
    assert_eq!(done.paths, 3);
    assert!((1..=3).contains(&done.agreed), "{done:?}");

    desktop.api().stop_model().await.unwrap();
    eventually("the laptop stops", || !working(&laptop)).await;

    phone.shutdown().await;
    laptop.shutdown().await;
    desktop.shutdown().await;
    for dir in [a, b, c] {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// The answer `service` gives to `request`, and its timings.
async fn answer(service: &Service, request: v1::AskRequest) -> (String, Option<v1::AskDone>) {
    let mut steps = service.api().ask(request);
    let (mut text, mut done) = (String::new(), None);
    while let Some(step) = tokio::time::timeout(Duration::from_secs(120), steps.next()).await.expect("an answer") {
        match step.unwrap_or_else(|e| panic!("{}", e.message)).step {
            Some(v1::ask_response::Step::Piece(piece)) => text.push_str(&piece),
            Some(v1::ask_response::Step::Done(d)) => done = Some(d),
            None => {}
        }
    }
    (text, done)
}

async fn ask(service: &Service) -> String {
    answer(service, v1::AskRequest { prompt: PROMPT.into(), max_tokens: GENERATE, ..Default::default() }).await.0
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn when_a_device_of_the_ring_leaves_its_standby_takes_its_place() {
    let engine = env_path("OMNARC_ENGINE");
    let model_file = env_path("OMNARC_TEST_MODEL");
    let expected = answer_alone(&engine, &model_file);

    let (a, b, c) = (data_dir("leave-desktop"), data_dir("leave-laptop"), data_dir("leave-server"));
    pairing::create(&a, "Home cluster", device("Desktop"), &[]).unwrap();
    let desktop = start(&a, "Desktop").await;
    admit(&desktop, &b, "Laptop").await;
    let laptop = start(&b, "Laptop").await;
    admit(&desktop, &c, "Server").await;
    let server = start(&c, "Server").await;
    let model = desktop.models().add(&model_file, |_| {}).await.unwrap();

    // the desktop holds the head and half the layers; the laptop and the server each the other half: one of them
    // runs it and the other stands by
    let shape = omnarc_node::service::shape(&model, CONTEXT, omnarc_engine_host::KvType::F16);
    let half = shape.layers / 2;
    let reserve = omnarc_node::service::ENGINE_RESERVE;
    let room = u64::midpoint(shape.layer_bytes, shape.kv_bytes_per_layer);
    let head_offer = reserve + shape.window_bytes(half, true) + room;
    let half_offer = reserve + shape.window_bytes(shape.layers - half, false) + room;
    desktop.api().set_budget(budget(head_offer)).unwrap();
    laptop.api().set_budget(budget(half_offer)).unwrap();
    server.api().set_budget(budget(half_offer)).unwrap();
    eventually("the desktop hears every budget and profile", || {
        let (telemetry, devices) = (desktop.telemetry(), desktop.devices());
        [desktop.id(), laptop.id(), server.id()].iter().all(|id| {
            telemetry.get(id).is_some_and(|t| t.memory_offered_bytes > 0 && t.memory_offered_bytes < 1 << 30)
                && devices.get(id).is_some_and(|d| d.accelerators.iter().any(|a| a.measured_bandwidth_gbps > 0.0))
        })
    })
    .await;

    let plan = desktop
        .api()
        .run_model(v1::RunModelRequest { context: CONTEXT, ..Default::default() })
        .await
        .unwrap_or_else(|e| panic!("{}", e.message))
        .plan
        .unwrap();
    assert_eq!(plan.stages.len(), 2, "{plan:?}");
    assert_eq!(plan.replicas.len(), 1, "the spare computer stands by for the second window: {plan:?}");
    assert_eq!(plan.backups.len(), 1, "a backup is solved for the second window's device leaving: {plan:?}");
    let key = |n: &Option<v1::NodeId>| n.as_ref().unwrap().ed25519_public_key.clone();
    let (running, standby) =
        if key(&plan.stages[1].node) == laptop.id().as_bytes().to_vec() { (laptop, server) } else { (server, laptop) };
    assert_eq!(key(&plan.replicas[0].node), standby.id().as_bytes().to_vec());
    let serving = |service: &Service, version: u64| {
        let me = service.id();
        service.telemetry().get(&me).is_some_and(|t| t.stages.iter().any(|s| s.plan_version >= version))
    };
    eventually("every device takes its part", || {
        serving(&desktop, plan.version) && serving(&running, plan.version) && serving(&standby, plan.version)
    })
    .await;
    assert_eq!(ask(&desktop).await, expected);

    // the device running the second window leaves: the cluster switches to the plan solved for it
    let gone = running.id();
    let leaving = std::time::Instant::now();
    running.shutdown().await;
    eventually("the backup plan takes over", || {
        desktop
            .api()
            .snapshot()
            .plans
            .iter()
            .any(|p| p.version > plan.version && p.stages.iter().all(|s| key(&s.node) != gone.as_bytes().to_vec()))
    })
    .await;
    // NO-43: a device that says it leaves is answered by the plan solved for it within a second
    let took = leaving.elapsed();
    eprintln!("the backup plan took over {took:?} after the device began to leave");
    assert!(took <= Duration::from_secs(1), "the backup plan took {took:?}");
    eventually("the standby runs the window now", || serving(&standby, plan.version + 1)).await;
    assert_eq!(ask(&desktop).await, expected, "the answer is the same without the device that left");

    standby.shutdown().await;
    desktop.shutdown().await;
    for dir in [a, b, c] {
        let _ = std::fs::remove_dir_all(dir);
    }
}
