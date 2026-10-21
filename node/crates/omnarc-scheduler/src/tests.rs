use std::time::Instant;

use proptest::prelude::*;

use super::*;

const MB: u64 = 1 << 20;

// Llama 3.1 8B at Q4_K_M, 4k context: about 145 MB of weights and 16 MB of KV cache per layer
fn llama_8b() -> Model {
    Model { layers: 32, layer_bytes: 145 * MB, kv_bytes_per_layer: 16 * MB, head_bytes: 280 * MB, tail_bytes: 280 * MB }
}

fn device(id: u32, offered_mb: u64, seconds_per_layer: f64) -> Device<u32> {
    Device {
        id,
        offered_bytes: offered_mb * MB,
        seconds_per_layer,
        link_seconds: 0.002,
        gpu: None,
        held: 0.0,
        fetch_seconds_per_byte: 0.0,
    }
}

fn assert_valid(model: &Model, devices: &[Device<u32>], plan: &Placement<u32>) {
    let mut next = 0;
    for (i, stage) in plan.stages.iter().enumerate() {
        assert_eq!(stage.layers.start, next, "windows are contiguous and in ring order");
        assert!(stage.layers.end > stage.layers.start, "no empty window");
        next = stage.layers.end;
        let d = devices.iter().find(|d| d.id == stage.node).expect("a stage is a device");
        assert_holds(model, d, stage.layers.end - stage.layers.start, stage.gpu_layers, i == 0);
    }
    assert_eq!(next, model.layers, "every layer is placed once");
    for r in &plan.replicas {
        assert!(plan.stages.iter().all(|s| s.node != r.node), "a replica stands on a device without a stage");
        assert!(plan.stages.iter().any(|s| s.layers == r.layers), "a replica copies a stage's window");
    }
}

// Q1 with the GPU: its layers (and on the head the output) within what it offers, the rest within the memory.
fn assert_holds(model: &Model, d: &Device<u32>, layers: u32, gpu_layers: u32, head: bool) {
    let per_layer = model.layer_bytes + model.kv_bytes_per_layer;
    let ends = if head { model.head_bytes + model.tail_bytes } else { 0 };
    let Some(gpu) = d.gpu else {
        assert_eq!(gpu_layers, 0, "device {} has no GPU", d.id);
        let used = model.window_bytes(layers, head);
        assert!(used <= d.offered_bytes, "device {} offers {} and was given {used}", d.id, d.offered_bytes);
        return;
    };
    assert!(gpu_layers <= layers);
    let tail = if head && gpu_layers > 0 { model.tail_bytes } else { 0 };
    let on_gpu = u64::from(gpu_layers) * per_layer + tail;
    assert!(on_gpu <= gpu.offered_bytes, "GPU of {} offers {} and was given {on_gpu}", d.id, gpu.offered_bytes);
    let in_memory = if gpu.shares_memory {
        u64::from(layers) * per_layer + ends
    } else {
        u64::from(layers - gpu_layers) * per_layer + ends - tail
    };
    assert!(in_memory <= d.offered_bytes, "device {} offers {} and was given {in_memory}", d.id, d.offered_bytes);
}

fn with_gpu(mut d: Device<u32>, offered_mb: u64, seconds_per_layer: f64, shares_memory: bool) -> Device<u32> {
    d.gpu = Some(Gpu { offered_bytes: offered_mb * MB, seconds_per_layer, shares_memory });
    d
}

#[test]
fn a_gpu_lets_one_device_hold_what_its_memory_alone_cannot() {
    // 5.7 GB of model: 3 GB of memory and a 4 GB GPU hold it, and the GPU makes it the quicker device (R6)
    let model = llama_8b();
    let devices = [with_gpu(device(1, 3_000, 0.002), 4_000, 0.0002, false), device(2, 8_000, 0.002)];
    let plan = place(&model, &devices).unwrap();
    assert_valid(&model, &devices, &plan);
    // (4000 - 280 for the output) / 161 per layer
    assert_eq!(plan.stages, vec![Stage { node: 1, layers: 0..32, gpu_layers: 23 }]);
    assert_eq!(plan.replicas, vec![Replica { node: 2, layers: 0..32, gpu_layers: 0 }]);
    assert!(plan.seconds_per_token < 32.0 * 0.002, "the GPU's layers are quicker");
}

#[test]
fn a_split_puts_on_each_gpu_only_what_it_offers() {
    // neither device holds the model alone, so it is split; the GPU takes layers within its 1 GB
    let model = llama_8b();
    let devices = [with_gpu(device(1, 2_000, 0.002), 1_000, 0.0002, false), device(2, 3_000, 0.002)];
    let plan = place(&model, &devices).unwrap();
    assert_valid(&model, &devices, &plan);
    let on_gpu: u32 = plan.stages.iter().filter(|s| s.node == 1).map(|s| s.gpu_layers).sum();
    assert!(on_gpu > 0, "a GPU quicker than the processor is used");
}

#[test]
fn a_gpu_slower_than_the_processor_takes_nothing_memory_does_not_force() {
    let model = llama_8b();
    let devices = [with_gpu(device(1, 8_000, 0.002), 4_000, 0.004, false)];
    let plan = place(&model, &devices).unwrap();
    assert_eq!(plan.stages[0].gpu_layers, 0);
}

#[test]
fn an_integrated_gpu_adds_speed_but_no_room() {
    let model = llama_8b();
    // its 4 GB are the device's own 3 GB: the model does not fit
    let small = [with_gpu(device(1, 3_000, 0.002), 4_000, 0.0002, true)];
    assert!(matches!(place(&model, &small), Err(Error::DoesNotFit { .. })));
    // with the room it is used for speed
    let large = [with_gpu(device(1, 8_000, 0.002), 4_000, 0.0002, true)];
    let plan = place(&model, &large).unwrap();
    assert_valid(&model, &large, &plan);
    assert!(plan.stages[0].gpu_layers > 0);
}

#[test]
fn sixty_four_devices_with_gpus_are_placed_within_a_second() {
    let model = Model { layers: 80, ..llama_8b() };
    let devices: Vec<_> = (0..64)
        .map(|i| {
            let d = device(i, 700 + u64::from(i % 7) * 100, 0.002 + f64::from(i % 5) * 0.0005);
            if i % 3 == 0 { with_gpu(d, 2_000, 0.0003, i % 6 == 0) } else { d }
        })
        .collect();
    let started = std::time::Instant::now();
    let plan = place(&model, &devices).unwrap();
    assert!(started.elapsed() < std::time::Duration::from_secs(1), "took {:?}", started.elapsed());
    assert_valid(&model, &devices, &plan);
}

fn holding(mut d: Device<u32>, held: f64, fetch_seconds_per_byte: f64) -> Device<u32> {
    d.held = held;
    d.fetch_seconds_per_byte = fetch_seconds_per_byte;
    d
}

#[test]
fn devices_that_hold_the_model_are_preferred_over_one_that_must_fetch_it_slowly() {
    // device 3 alone holds the model and is the quickest, but it has none of it and 1 MB/s to fetch it: 95 minutes
    // before the first answer. Devices 1 and 2 hold it already and split it at once.
    let model = llama_8b();
    let devices = [
        holding(device(1, 3_000, 0.002), 1.0, 0.0),
        holding(device(2, 3_000, 0.002), 1.0, 0.0),
        holding(device(3, 8_000, 0.0015), 0.0, 1e-6),
    ];
    let plan = place(&model, &devices).unwrap();
    assert_valid(&model, &devices, &plan);
    assert!(plan.stages.iter().all(|s| s.node != 3), "nothing waits on the slow fetch: {:?}", plan.stages);
}

#[test]
fn a_quick_fetch_does_not_stop_the_quickest_device_from_holding_it_all() {
    // the same device 3 on a gigabyte a second: its fetch is a few seconds, and alone it is quicker than any split
    let model = llama_8b();
    let devices = [
        holding(device(1, 3_000, 0.002), 1.0, 0.0),
        holding(device(2, 3_000, 0.002), 1.0, 0.0),
        holding(device(3, 8_000, 0.0015), 0.0, 1e-9),
    ];
    let plan = place(&model, &devices).unwrap();
    assert_eq!(plan.stages, vec![Stage { node: 3, layers: 0..32, gpu_layers: 0 }]);
}

#[test]
fn a_model_that_fits_on_one_device_is_not_split() {
    let model = llama_8b();
    let devices = [device(1, 8_000, 0.004), device(2, 16_000, 0.002), device(3, 2_000, 0.001)];
    let plan = place(&model, &devices).unwrap();
    assert_eq!(
        plan.stages,
        vec![Stage { node: 2, layers: 0..32, gpu_layers: 0 }],
        "the fastest device that holds all of it"
    );
    assert_eq!(
        plan.replicas,
        vec![Replica { node: 1, layers: 0..32, gpu_layers: 0 }],
        "another that holds it all is a copy"
    );
    assert_eq!(plan.left_out, vec![(3, LeftOut::NotNeeded)], "a faster device too small for it is not used");
}

#[test]
fn what_a_device_offers_is_a_hard_limit() {
    let model = llama_8b();
    // a 2 GB phone offering 500 MB, a laptop offering 3 GB, a workstation offering 3 GB of its 64
    let devices = [device(1, 3_000, 0.003), device(2, 500, 0.02), device(3, 3_000, 0.001)];
    let plan = place(&model, &devices).unwrap();
    assert_valid(&model, &devices, &plan);
}

#[test]
fn a_device_too_small_for_one_layer_is_left_out() {
    let model = llama_8b();
    let devices = [device(1, 3_000, 0.003), device(2, 100, 0.0001), device(3, 3_000, 0.001)];
    let plan = place(&model, &devices).unwrap();
    assert!(plan.left_out.contains(&(2, LeftOut::TooSmall)));
    assert!(plan.stages.iter().all(|s| s.node != 2));
}

#[test]
fn a_slow_device_takes_only_what_the_fast_ones_cannot() {
    let model = llama_8b();
    // the fast devices hold all but about three layers between them
    let devices = [device(1, 2_600, 0.001), device(2, 2_000, 0.05), device(3, 2_600, 0.001)];
    let plan = place(&model, &devices).unwrap();
    assert_valid(&model, &devices, &plan);
    let slow = plan.stages.iter().find(|s| s.node == 2).expect("the slow device is needed");
    assert!(slow.layers.len() <= 4, "the slow device holds {} layers", slow.layers.len());
}

#[test]
fn the_head_is_wherever_its_extras_fit() {
    let model = llama_8b();
    // the fastest device is too small for the embedding and the output, yet holds four layers
    let devices = [device(1, 700, 0.001), device(2, 5_100, 0.002)];
    let plan = place(&model, &devices).unwrap();
    assert_valid(&model, &devices, &plan);
    assert_eq!(
        plan.stages,
        vec![Stage { node: 2, layers: 0..28, gpu_layers: 0 }, Stage { node: 1, layers: 28..32, gpu_layers: 0 }]
    );
}

#[test]
fn devices_that_together_offer_too_little_get_an_answer_not_a_plan() {
    let model = llama_8b();
    let devices = [device(1, 1_000, 0.001), device(2, 1_000, 0.001)];
    assert!(matches!(place(&model, &devices), Err(Error::DoesNotFit { .. })));
}

#[test]
fn every_stage_gets_a_replica_where_one_fits() {
    let model = llama_8b();
    let devices = [device(1, 3_200, 0.001), device(2, 3_200, 0.001), device(3, 3_200, 0.002), device(4, 3_200, 0.002)];
    let plan = place(&model, &devices).unwrap();
    assert_valid(&model, &devices, &plan);
    assert_eq!(plan.stages.len(), 2);
    assert_eq!(plan.replicas.len(), 2, "two spare devices stand by for the two windows: {plan:?}");
}

#[test]
fn a_backup_plan_is_ready_for_every_stage_leaving() {
    let model = llama_8b();
    let devices = [device(1, 3_200, 0.001), device(2, 3_200, 0.001), device(3, 3_200, 0.002)];
    let plan = place(&model, &devices).unwrap();
    let backups = backups(&model, &devices, &plan);
    assert_eq!(backups.len(), plan.stages.len());
    for (gone, backup) in &backups {
        assert!(backup.stages.iter().all(|s| s.node != *gone));
        let rest: Vec<_> = devices.iter().filter(|d| d.id != *gone).cloned().collect();
        assert_valid(&model, &rest, backup);
    }
}

#[test]
fn sixty_four_devices_are_placed_within_a_second() {
    // Llama 3.1 70B at Q4_K_M: 80 layers of about 520 MB
    let model = Model {
        layers: 80,
        layer_bytes: 520 * MB,
        kv_bytes_per_layer: 40 * MB,
        head_bytes: 1_100 * MB,
        tail_bytes: 1_100 * MB,
    };
    let devices: Vec<_> =
        (0..64u32).map(|i| device(i, 1_200 + u64::from(i % 7) * 700, 0.002 + f64::from(i % 5) * 0.001)).collect();
    let started = Instant::now();
    let plan = place(&model, &devices).unwrap();
    let took = started.elapsed();
    assert_valid(&model, &devices, &plan);
    assert!(took.as_secs_f64() <= 1.0, "placing 64 devices took {took:?}");
}

#[test]
fn a_network_larger_than_a_group_is_planned_within_a_second() {
    // Llama 3.1 70B on 200 devices of 2-6 GB: the fastest that hold it are planned, the rest stand by or wait
    let model = Model {
        layers: 80,
        layer_bytes: 520 * MB,
        kv_bytes_per_layer: 40 * MB,
        head_bytes: 1_100 * MB,
        tail_bytes: 1_100 * MB,
    };
    let devices: Vec<_> =
        (0..200u32).map(|i| device(i, 2_000 + u64::from(i % 5) * 1_000, 0.002 + f64::from(i % 7) * 0.001)).collect();
    let started = Instant::now();
    let plan = place(&model, &devices).unwrap();
    let took = started.elapsed();
    assert_valid(&model, &devices, &plan);
    assert!(plan.stages.len() <= GROUP);
    assert_eq!(
        plan.stages.len() + plan.replicas.len() + plan.left_out.len(),
        devices.len(),
        "every device is accounted for"
    );
    assert!(took.as_secs_f64() <= 1.0, "placing 200 devices took {took:?}");
}

#[test]
fn throughput_balances_the_stages_latency_favors_the_fastest() {
    let model = llama_8b();
    // the fast device holds most of the model, the slower one the rest
    let devices = [device(1, 5_000, 0.001), device(2, 3_500, 0.002)];
    let slowest = |plan: &Placement<u32>| {
        plan.stages
            .iter()
            .map(|s| f64::from(s.layers.end - s.layers.start) * if s.node == 1 { 0.001 } else { 0.002 })
            .fold(0.0, f64::max)
    };
    let latency = place_for(&model, &devices, Objective::Latency).unwrap();
    let throughput = place_for(&model, &devices, Objective::Throughput).unwrap();
    assert_valid(&model, &devices, &latency);
    assert_valid(&model, &devices, &throughput);
    assert!(slowest(&throughput) < slowest(&latency), "{throughput:?} against {latency:?}");
    assert!(latency.seconds_per_token <= throughput.seconds_per_token);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn no_plan_ever_exceeds_an_offer(
        offers in prop::collection::vec((0u64..6_000, 1u32..50), 1..9),
    ) {
        let model = llama_8b();
        let devices: Vec<_> = offers
            .iter()
            .enumerate()
            .map(|(i, &(mb, t))| device(u32::try_from(i).unwrap(), mb, f64::from(t) / 1000.0))
            .collect();
        match place(&model, &devices) {
            Ok(plan) => assert_valid(&model, &devices, &plan),
            Err(Error::DoesNotFit { .. }) => {
                // a sure fit would have been found: every device keeping room for the head's extras and still
                // holding all the layers between them
                let one = model.layer_bytes + model.kv_bytes_per_layer;
                let ends = model.head_bytes + model.tail_bytes;
                let sure: u64 = devices.iter().map(|d| d.offered_bytes.saturating_sub(ends) / one).sum();
                prop_assert!(sure < u64::from(model.layers), "a plan existed for {offers:?}");
            }
            Err(e) => prop_assert!(false, "{e}"),
        }
    }
}
