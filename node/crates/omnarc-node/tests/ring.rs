//! A ring of two nodes on this machine generates exactly what one device generates. Built only with the
//! `engine-tests` feature; needs `OMNARC_ENGINE` (the engine executable) and `OMNARC_TEST_MODEL` (a small
//! llama-architecture GGUF model, e.g. stories15M-q8_0.gguf).

use std::net::{SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};

use omnarc_engine_host::{Backend, DType, Engine, Input, SamplerParams, Step};
use omnarc_network::Peer;
use omnarc_node::ring::{self, Config, Window};

const PROMPT: &str = "Once upon a time, there was a little girl named Lily.";
const GENERATE: u32 = 24;

fn env_path(name: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("set {name} to run the ring test")))
}

fn free_port() -> SocketAddr {
    UdpSocket::bind("127.0.0.1:0").unwrap().local_addr().unwrap()
}

/// Greedy generation on one device: the whole model in one shard.
fn generate_whole(engine_path: &Path, model: &Path, layers: u32) -> Vec<i32> {
    let mut engine = Engine::spawn(engine_path, 16 << 20).unwrap();
    let whole = engine.open(model, 0..layers + 1, Backend::Cpu, 512).unwrap();
    let mut tokens = engine.tokenize(whole.id, PROMPT, true).unwrap();
    let sampler = engine.sampler_open(SamplerParams::default()).unwrap();
    let mut position = 0;
    let mut generated = Vec::new();
    while generated.len() < GENERATE as usize {
        let step = Step { request: 1, position, input: Input::Tokens(&tokens), all_logits: false, output: DType::F32 };
        engine.run(&whole, &step).unwrap();
        position += u32::try_from(tokens.len()).unwrap();
        let token = engine.sample(sampler, whole.model.vocab_size).unwrap();
        generated.push(token);
        if engine.token_text(whole.id, token).unwrap().1 {
            break;
        }
        tokens = vec![token];
    }
    engine.quit().unwrap();
    generated
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_ring_generates_what_one_device_generates() {
    let engine = env_path("OMNARC_ENGINE");
    let model = env_path("OMNARC_TEST_MODEL");
    let layers = {
        let mut probe = Engine::spawn(&engine, 1 << 20).unwrap();
        let info = probe.model_info(&model).unwrap();
        probe.quit().unwrap();
        info.layer_count
    };

    let dir = std::env::temp_dir().join(format!("omnarc-ring-test-{}", std::process::id()));
    let (head_key, worker_key) = (dir.join("head.key"), dir.join("worker.key"));
    let head_id = ring::node_id(&head_key).unwrap();
    let worker_id = ring::node_id(&worker_key).unwrap();
    let (head_addr, worker_addr) = (free_port(), free_port());

    let config = |windows: Vec<Window>, listen, next, prev, key, prompt| Config {
        model: model.clone(),
        windows,
        listen,
        next,
        next_replica: None,
        prev: vec![prev],
        key,
        engine: engine.clone(),
        backend: Backend::Cpu,
        mirror_only: false,
        replica: None,
        max_tokens: 512,
        chunk: 8, // a prompt of several chunks
        wire: DType::F32,
        threads: 0,
        draft: 4,
        prompt,
        generate: GENERATE,
    };
    let half = layers / 2;
    let worker = config(
        vec![Window(half..layers)],
        worker_addr,
        Peer { id: head_id, addr: head_addr },
        head_id,
        worker_key,
        None,
    );
    let head = config(
        vec![Window(0..half), Window(layers..layers + 1)],
        head_addr,
        Peer { id: worker_id, addr: worker_addr },
        worker_id,
        head_key,
        Some(PROMPT.to_owned()),
    );

    let worker_task = tokio::spawn(ring::serve(worker));
    let report = ring::generate(head).await.unwrap();
    worker_task.abort();

    let expected = tokio::task::spawn_blocking(move || generate_whole(&engine, &model, layers)).await.unwrap();
    assert_eq!(report.generated, expected, "ring: {:?}", report.text);
    assert!(report.prompt_tokens > 8);
    assert!(report.tpot.is_some());
    let _ = std::fs::remove_dir_all(dir);
}

/// EN-17: the live node sends its replica a copy of every frame it receives, the replica runs it to keep its KV
/// cache in step and forwards nothing. The answer must be exactly what the ring gives without a replica -- a
/// mirror that changed the answer would be worse than no mirror at all.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[cfg_attr(not(feature = "engine-tests"), ignore = "set OMNARC_ENGINE and OMNARC_TEST_MODEL")]
async fn a_replica_mirrors_the_frames_without_changing_the_answer() {
    let engine = env_path("OMNARC_ENGINE");
    let model = env_path("OMNARC_TEST_MODEL");
    let layers = {
        let mut probe = Engine::spawn(&engine, 1 << 20).unwrap();
        let info = probe.model_info(&model).unwrap();
        probe.quit().unwrap();
        info.layer_count
    };

    let dir = std::env::temp_dir().join(format!("omnarc-mirror-test-{}", std::process::id()));
    let (head_key, worker_key, replica_key, head_replica_key) =
        (dir.join("head.key"), dir.join("worker.key"), dir.join("replica.key"), dir.join("head-replica.key"));
    let head_id = ring::node_id(&head_key).unwrap();
    let worker_id = ring::node_id(&worker_key).unwrap();
    let replica_id = ring::node_id(&replica_key).unwrap();
    let head_replica_id = ring::node_id(&head_replica_key).unwrap();
    let (head_addr, worker_addr, replica_addr, head_replica_addr) =
        (free_port(), free_port(), free_port(), free_port());
    let half = layers / 2;

    let base = |windows: Vec<Window>, listen, next, prev, key, prompt, mirror_only, replica| ring::Config {
        model: model.clone(),
        windows,
        listen,
        next,
        next_replica: None,
        prev: vec![prev],
        key,
        engine: engine.clone(),
        backend: Backend::Cpu,
        mirror_only,
        replica,
        max_tokens: 512,
        chunk: 8,
        wire: DType::F32,
        threads: 0,
        draft: 4,
        prompt,
        generate: GENERATE,
    };

    // the replica holds the worker's window and only mirrors
    let replica = base(
        vec![Window(half..layers)],
        replica_addr,
        Peer { id: worker_id, addr: worker_addr },
        worker_id,
        replica_key,
        None,
        true,
        None,
    );
    let worker = base(
        vec![Window(half..layers)],
        worker_addr,
        Peer { id: head_id, addr: head_addr },
        head_id,
        worker_key,
        None,
        false,
        Some(Peer { id: replica_id, addr: replica_addr }),
    );
    // the head holds layer 0, so its replica is kept in step with token frames, not hidden states
    let head_replica = base(
        vec![Window(0..half), Window(layers..layers + 1)],
        head_replica_addr,
        Peer { id: head_id, addr: head_addr },
        head_id,
        head_replica_key,
        None,
        true,
        None,
    );
    let head = base(
        vec![Window(0..half), Window(layers..layers + 1)],
        head_addr,
        Peer { id: worker_id, addr: worker_addr },
        worker_id,
        head_key,
        Some(PROMPT.to_owned()),
        false,
        Some(Peer { id: head_replica_id, addr: head_replica_addr }),
    );

    let head_replica_task = tokio::spawn(ring::serve(head_replica));
    let replica_task = tokio::spawn(ring::serve(replica));
    let worker_task = tokio::spawn(ring::serve(worker));
    let report = ring::generate(head).await.unwrap();
    worker_task.abort();
    replica_task.abort();
    head_replica_task.abort();

    let expected = tokio::task::spawn_blocking(move || generate_whole(&engine, &model, layers)).await.unwrap();
    assert_eq!(report.generated, expected, "with a replica mirroring: {:?}", report.text);
    assert!(report.tpot.is_some());
    let _ = std::fs::remove_dir_all(dir);
}

/// NO-43: the device holding a window leaves while the model is running. Its replica, kept in step by
/// EN-17, takes its place and the answer finishes without being computed again. A ring that broke here
/// would either hang or give a different answer, so the assertion is the whole answer.
#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
#[cfg_attr(not(feature = "engine-tests"), ignore = "set OMNARC_ENGINE and OMNARC_TEST_MODEL")]
async fn a_replica_takes_over_when_the_device_holding_a_window_leaves() {
    let engine = env_path("OMNARC_ENGINE");
    let model = env_path("OMNARC_TEST_MODEL");
    let layers = {
        let mut probe = Engine::spawn(&engine, 1 << 20).unwrap();
        let info = probe.model_info(&model).unwrap();
        probe.quit().unwrap();
        info.layer_count
    };

    let dir = std::env::temp_dir().join(format!("omnarc-failover-test-{}", std::process::id()));
    let (head_key, worker_key, spare_key) = (dir.join("head.key"), dir.join("worker.key"), dir.join("spare.key"));
    let head_id = ring::node_id(&head_key).unwrap();
    let worker_id = ring::node_id(&worker_key).unwrap();
    let spare_id = ring::node_id(&spare_key).unwrap();
    let (head_addr, worker_addr, spare_addr) = (free_port(), free_port(), free_port());
    let half = layers / 2;

    let base = |windows: Vec<Window>, listen, next, next_replica, prev: Vec<_>, key, prompt, mirror_only, replica| {
        ring::Config {
            model: model.clone(),
            windows,
            listen,
            next,
            next_replica,
            prev,
            key,
            engine: engine.clone(),
            backend: Backend::Cpu,
            mirror_only,
            replica,
            max_tokens: 512,
            chunk: 8,
            wire: DType::F32,
            threads: 0,
            draft: 4,
            prompt,
            generate: GENERATE,
        }
    };

    // the spare accepts the worker (its mirrors) and the head (which turns to it when the worker goes)
    let spare = base(
        vec![Window(half..layers)],
        spare_addr,
        Peer { id: head_id, addr: head_addr },
        None,
        vec![worker_id, head_id],
        spare_key,
        None,
        true,
        None,
    );
    let worker = base(
        vec![Window(half..layers)],
        worker_addr,
        Peer { id: head_id, addr: head_addr },
        None,
        vec![head_id],
        worker_key,
        None,
        false,
        Some(Peer { id: spare_id, addr: spare_addr }),
    );
    let head = base(
        vec![Window(0..half), Window(layers..layers + 1)],
        head_addr,
        Peer { id: worker_id, addr: worker_addr },
        Some(Peer { id: spare_id, addr: spare_addr }),
        vec![worker_id, spare_id],
        head_key,
        Some(PROMPT.to_owned()),
        false,
        None,
    );

    let spare_task = tokio::spawn(ring::serve(spare));
    let worker_task = tokio::spawn(ring::serve(worker));
    let head_task = tokio::spawn(ring::generate(head));

    // let the answer get going, then take the worker away mid-sentence
    tokio::time::sleep(std::time::Duration::from_millis(350)).await;
    worker_task.abort();

    let report = head_task.await.unwrap().unwrap();
    spare_task.abort();

    let expected = tokio::task::spawn_blocking(move || generate_whole(&engine, &model, layers)).await.unwrap();
    assert_eq!(report.generated, expected, "after the takeover: {:?}", report.text);
    assert_eq!(report.generated.len(), GENERATE as usize, "the answer finished");
    let _ = std::fs::remove_dir_all(dir);
}
