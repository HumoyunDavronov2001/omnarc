//! Drives a real `omnarc-engine` process. Built only with the `engine-tests` feature, and it needs:
//!
//! - `OMNARC_ENGINE`: the engine executable from the engine build, e.g. `build/engine/omnarc-engine`;
//! - `OMNARC_TEST_MODEL`: a small llama-architecture GGUF model, e.g. `stories15M-q8_0.gguf`.
//!
//! ```text
//! OMNARC_ENGINE=... OMNARC_TEST_MODEL=... cargo test -p omnarc-engine-host --features engine-tests
//! ```

use std::path::PathBuf;

use omnarc_engine_host::{Backend, DType, Engine, Error, Gpu, Input, KvType, SamplerParams, Status, Step};

fn paths() -> (PathBuf, PathBuf) {
    let get = |name: &str| {
        PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("set {name} to run the engine tests")))
    };
    (get("OMNARC_ENGINE"), get("OMNARC_TEST_MODEL"))
}

fn tokens_step(request: u64, position: u32, tokens: &[i32]) -> Step<'_> {
    Step { request, position, input: Input::Tokens(tokens), all_logits: false, output: DType::F32 }
}

fn argmax(values: &[f32]) -> usize {
    values.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map_or(0, |(i, _)| i)
}

#[test]
fn split_shards_match_the_whole_model() {
    let (exe, model) = paths();
    let mut engine = Engine::spawn(&exe, 64 << 20).unwrap();
    assert!(engine.abi_version() >= 2);

    let info = engine.model_info(&model).unwrap();
    assert!(info.splittable, "{info:?}");
    let n = info.layer_count;
    let whole = engine.open(&model, 0..n + 1, Backend::Cpu, 256).unwrap();
    let first = engine.open(&model, 0..n / 2, Backend::Cpu, 256).unwrap();
    let rest = engine.open(&model, n / 2..n, Backend::Cpu, 256).unwrap();
    let head = engine.open(&model, n..n + 1, Backend::Cpu, 256).unwrap();
    assert!(whole.takes_tokens() && whole.gives_logits());
    assert!(first.takes_tokens() && !first.gives_logits());
    assert!(!rest.takes_tokens() && !rest.gives_logits());
    assert!(!head.takes_tokens() && head.gives_logits());

    let prompt = engine.tokenize(whole.id, "Once upon a time, there was a little girl named", true).unwrap();
    assert!(prompt.len() > 5);

    // the prompt through the whole model, then through the three shards with the hidden state passed as f32
    let reference = engine.run(&whole, &tokens_step(1, 0, &prompt)).unwrap().to_f32();
    assert_eq!(reference.len(), info.vocab_size as usize);

    let count = u32::try_from(prompt.len()).unwrap();
    let hidden = engine.run(&first, &tokens_step(2, 0, &prompt)).unwrap().data.to_vec();
    let step = |data| Step {
        request: 2,
        position: 0,
        input: Input::Hidden { dtype: DType::F32, count, data },
        all_logits: false,
        output: DType::F32,
    };
    let hidden = engine.run(&rest, &step(&hidden)).unwrap().data.to_vec();
    let split = engine.run(&head, &step(&hidden)).unwrap().to_f32();
    assert_eq!(reference, split, "on the CPU the split run is bit for bit the whole run");

    // greedy generation: the whole model and the shards pick the same tokens; hidden states travel as f16
    let sampler = engine.sampler_open(SamplerParams::default()).unwrap();
    let mut next = i32::try_from(argmax(&reference)).unwrap();
    let mut text = String::new();
    for position in (count..).take(8) {
        let reference = engine.run(&whole, &tokens_step(1, position, &[next])).unwrap().to_f32();

        let hidden = engine
            .run(&first, &Step { output: DType::F16, ..tokens_step(2, position, &[next]) })
            .unwrap()
            .data
            .to_vec();
        let input = Input::Hidden { dtype: DType::F16, count: 1, data: &hidden };
        let step = Step { request: 2, position, input, all_logits: false, output: DType::F16 };
        let hidden = engine.run(&rest, &step).unwrap().data.to_vec();
        let input = Input::Hidden { dtype: DType::F16, count: 1, data: &hidden };
        engine.run(&head, &Step { input, ..step }).unwrap();
        let token = engine.sample(sampler, info.vocab_size).unwrap();

        assert_eq!(usize::try_from(token).unwrap(), argmax(&reference));
        text.push_str(&engine.token_text(whole.id, token).unwrap().0);
        next = token;
    }
    assert_ne!(text.trim(), "");

    for shard in [&whole, &first, &rest, &head] {
        engine.drop_request(shard.id, 1).unwrap();
        engine.drop_request(shard.id, 2).unwrap();
        engine.close(shard.id).unwrap();
    }
    engine.sampler_close(sampler).unwrap();
    engine.quit().unwrap();
}

#[test]
fn the_engine_names_the_gpus_it_can_use() {
    let (exe, _) = paths();
    let mut engine = Engine::spawn(&exe, 1 << 20).unwrap();
    assert!(engine.abi_version() >= 3);
    // a build for the processor names none; a GPU build names each GPU with its backend
    let devices = engine.devices().unwrap();
    for (i, device) in devices.iter().enumerate() {
        assert!(!matches!(device.backend, Backend::Cpu | Backend::Unspecified), "{device:?}");
        assert!(!device.name.is_empty(), "{device:?}");
        // NO-40: each one is measured
        let bench = engine.bench(u32::try_from(i).unwrap(), std::time::Duration::from_millis(300)).unwrap();
        assert!(bench.bandwidth_bytes_per_s > 1e8 && bench.flops > 1e9, "{device:?} {bench:?}");
    }
    // a GPU that is not there is neither measured nor opened on
    let missing = u32::try_from(devices.len()).unwrap();
    let error = engine.bench(missing, std::time::Duration::from_millis(10)).unwrap_err();
    assert!(matches!(error, Error::Engine { status: Status::InvalidArgument, .. }), "{error}");
    let (_, model) = paths();
    let gpu = Some(Gpu { index: missing, layers: 1 });
    let error = engine.open_on(&model, 0..2, Backend::Unspecified, 64, KvType::F16, gpu).unwrap_err();
    assert!(matches!(error, Error::Engine { status: Status::Unsupported, .. }), "{error}");
    engine.quit().unwrap();
}

/// Part of a window on a GPU computes what the processor computes (ABI 7); without a GPU there is nothing to compare.
#[test]
fn part_of_a_window_on_a_gpu_computes_the_same() {
    let (exe, model) = paths();
    let mut engine = Engine::spawn(&exe, 1 << 22).unwrap();
    if engine.devices().unwrap().is_empty() {
        return;
    }
    let n = engine.model_info(&model).unwrap().layer_count;
    let tokens = [1, 450, 4996, 17354];
    let mut logits = Vec::new();
    for gpu in [None, Some(Gpu { index: 0, layers: n / 2 })] {
        let backend = if gpu.is_none() { Backend::Cpu } else { Backend::Unspecified };
        let shard = engine.open_on(&model, 0..n + 1, backend, 64, KvType::F16, gpu).unwrap();
        let out = engine.run(&shard, &tokens_step(1, 0, &tokens)).unwrap().to_f32();
        logits.push(argmax(&out));
        engine.close(shard.id).unwrap();
    }
    assert_eq!(logits[0], logits[1]);
    engine.quit().unwrap();
}

#[test]
fn errors_are_reported() {
    let (exe, model) = paths();
    let mut engine = Engine::spawn(&exe, 1 << 20).unwrap();
    let n = engine.model_info(&model).unwrap().layer_count;

    let error = engine.open(&model, 0..n + 2, Backend::Cpu, 64).unwrap_err();
    assert!(matches!(error, Error::Engine { status: Status::InvalidArgument, .. }), "{error}");
    let error = engine.model_info(std::path::Path::new("no-such-model.gguf")).unwrap_err();
    assert!(matches!(error, Error::Engine { status: Status::Io, .. }), "{error}");

    // the engine keeps working after errors, and a crash shows up as an I/O error
    engine.model_info(&model).unwrap();
    engine.kill().unwrap();
    assert!(matches!(engine.model_info(&model), Err(Error::Io(_))));
}

#[test]
fn a_replica_continues_an_answer_from_the_kv_cache() {
    // EN-17: the cache of a request is handed to a replica of the same layers, and the replica carries the
    // answer on from the same token. Without it a device leaving would mean reading the prompt again, which
    // is the interruption the cluster promises never to have (Y7).
    let (exe, model) = paths();
    let mut engine = Engine::spawn(&exe, 1 << 22).unwrap();
    assert!(engine.abi_version() >= 4);
    let info = engine.model_info(&model).unwrap();
    let n = info.layer_count;

    let live = engine.open(&model, 0..n + 1, Backend::Cpu, 256).unwrap();
    let replica = engine.open(&model, 0..n + 1, Backend::Cpu, 256).unwrap();

    let prompt = engine.tokenize(live.id, "Once upon a time, there was a little girl named", true).unwrap();
    let logits = engine.run(&live, &tokens_step(1, 0, &prompt)).unwrap().to_f32();
    let next = i32::try_from(argmax(&logits)).unwrap();
    let position = u32::try_from(prompt.len()).unwrap();

    // the cache grows with the answer, so its size is asked for right before it is handed over
    let size = engine.kv_size(live.id, 1).unwrap();
    assert!(size > 0, "a request that has run holds a cache");
    let cache = engine.kv_read(live.id, 1).unwrap();
    assert_eq!(u64::try_from(cache.len()).unwrap(), size);

    // the replica knows nothing of this request until the cache arrives, under an id of its own
    engine.kv_write(replica.id, 7, &cache).unwrap();
    assert_eq!(engine.kv_size(replica.id, 7).unwrap(), size);

    let on_live = engine.run(&live, &tokens_step(1, position, &[next])).unwrap().to_f32();
    let on_replica = engine.run(&replica, &tokens_step(7, position, &[next])).unwrap().to_f32();
    assert_eq!(on_live, on_replica, "the replica continues the answer instead of recomputing it");

    // the same step on a shard that never got the cache differs, so the equality above means something
    let bare = engine.open(&model, 0..n + 1, Backend::Cpu, 256).unwrap();
    let on_bare = engine.run(&bare, &tokens_step(9, position, &[next])).unwrap().to_f32();
    assert_ne!(on_live, on_bare, "an empty cache cannot stand in for the one that was handed over");

    // a cache from a different layer window is refused rather than quietly giving wrong answers
    if n > 2 {
        let other = engine.open(&model, 0..n / 2, Backend::Cpu, 256).unwrap();
        let error = engine.kv_write(other.id, 7, &cache).unwrap_err();
        assert!(matches!(error, Error::Engine { .. }), "{error}");
        engine.close(other.id).unwrap();
    }

    let unknown = engine.kv_size(replica.id, 404).unwrap_err();
    assert!(matches!(unknown, Error::Engine { status: Status::InvalidArgument, .. }), "{unknown}");

    for shard in [&live, &replica, &bare] {
        engine.close(shard.id).unwrap();
    }
    engine.quit().unwrap();
}

#[test]
fn a_shard_holds_its_cache_in_eight_bits_too() {
    let (engine_path, model) = paths();
    let mut engine = Engine::spawn(&engine_path, 16 << 20).unwrap();
    // the first two layers only: enough to run the cache, little enough for a large model on a small machine
    let shard = match engine.open_with_kv(&model, 0..2, Backend::Cpu, 256, KvType::Q8_0) {
        Ok(shard) => shard,
        Err(e) => {
            // a model whose attention heads are not a multiple of the 32-value blocks cannot quantize its cache
            assert!(e.to_string().contains("does not divide"), "{e}");
            engine.quit().unwrap();
            return;
        }
    };
    let tokens = engine.tokenize(shard.id, "Once upon a time", true).unwrap();
    let step = Step { request: 1, position: 0, input: Input::Tokens(&tokens), all_logits: false, output: DType::F32 };
    let out = engine.run(&shard, &step).unwrap();
    let hidden = shard.model.hidden_dim as usize;
    assert_eq!(out.data.len(), tokens.len() * hidden * 4, "a hidden state for every token");
    // and the next token reads the 8-bit cache
    let next = Step {
        request: 1,
        position: u32::try_from(tokens.len()).unwrap(),
        input: Input::Tokens(&[tokens[0]]),
        all_logits: false,
        output: DType::F32,
    };
    assert_eq!(engine.run(&shard, &next).unwrap().data.len(), hidden * 4);
    engine.quit().unwrap();
}

#[test]
fn a_conversation_takes_the_models_own_template() {
    let (engine_path, model) = paths();
    let mut engine = Engine::spawn(&engine_path, 16 << 20).unwrap();
    let shard = engine.open(&model, 0..1, Backend::Cpu, 64).unwrap();
    let chat = [("system", "Be brief."), ("user", "Hello"), ("assistant", "Hi."), ("user", "How are you?")];
    match engine.chat_prompt(shard.id, &chat).unwrap() {
        // the model's own words around each message, ending where the answer begins
        Some(prompt) => {
            assert!(prompt.contains("Be brief.") && prompt.contains("How are you?"), "{prompt}");
            assert!(prompt.find("Hello").unwrap() < prompt.find("How are you?").unwrap());
        }
        // a model without a chat template, such as stories15M: the caller asks with the plain text instead
        None => eprintln!("this model has no chat template"),
    }
    engine.quit().unwrap();
}
