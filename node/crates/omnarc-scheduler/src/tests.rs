use std::time::Instant;

use proptest::prelude::*;

use super::*;

const MB: u64 = 1 << 20;

// Llama 3.1 8B at Q4_K_M, 4k context: about 145 MB of weights and 16 MB of KV cache per layer
fn llama_8b() -> Model {
    Model { layers: 32, layer_bytes: 145 * MB, kv_bytes_per_layer: 16 * MB, head_bytes: 280 * MB, tail_bytes: 280 * MB }
}

fn device(id: u32, offered_mb: u64, seconds_per_layer: f64) -> Device<u32> {
    Device { id, offered_bytes: offered_mb * MB, seconds_per_layer, link_seconds: 0.002 }
}

fn assert_valid(model: &Model, devices: &[Device<u32>], plan: &Placement<u32>) {
    let mut next = 0;
    for (i, stage) in plan.stages.iter().enumerate() {
        assert_eq!(stage.layers.start, next, "windows are contiguous and in ring order");
        assert!(stage.layers.end > stage.layers.start, "no empty window");
        next = stage.layers.end;
        let d = devices.iter().find(|d| d.id == stage.node).expect("a stage is a device");
        let used = model.window_bytes(stage.layers.end - stage.layers.start, i == 0);
        assert!(used <= d.offered_bytes, "device {} offers {} and was given {used}", d.id, d.offered_bytes);
    }
    assert_eq!(next, model.layers, "every layer is placed once");
    for r in &plan.replicas {
        assert!(plan.stages.iter().all(|s| s.node != r.node), "a replica stands on a device without a stage");
        assert!(plan.stages.iter().any(|s| s.layers == r.layers), "a replica copies a stage's window");
    }
}

#[test]
fn a_model_that_fits_on_one_device_is_not_split() {
    let model = llama_8b();
    let devices = [device(1, 8_000, 0.004), device(2, 16_000, 0.002), device(3, 2_000, 0.001)];
    let plan = place(&model, &devices).unwrap();
    assert_eq!(plan.stages, vec![Stage { node: 2, layers: 0..32 }], "the fastest device that holds all of it");
    assert_eq!(plan.replicas, vec![Replica { node: 1, layers: 0..32 }], "another that holds it all is a copy");
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
    assert_eq!(plan.stages, vec![Stage { node: 2, layers: 0..28 }, Stage { node: 1, layers: 28..32 }]);
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
