//! Property tests: every valid header survives encode → decode, and decoding arbitrary bytes never panics.

use omnarc_protocol::frame::{DType, Flags, FrameHeader, HEADER_LEN};
use omnarc_protocol::v1;
use proptest::prelude::*;
use prost::Message;

fn dtype() -> impl Strategy<Value = DType> {
    prop_oneof![Just(DType::F32), Just(DType::F16), Just(DType::Bf16), Just(DType::F8E4M3), Just(DType::I8)]
}

fn flags() -> impl Strategy<Value = Flags> {
    (any::<bool>(), any::<bool>(), any::<bool>()).prop_map(|(draft, last, replay)| {
        let mut f = Flags::empty();
        if draft {
            f = f.union(Flags::DRAFT);
        }
        if last {
            f = f.union(Flags::LAST_STAGE);
        }
        if replay {
            f = f.union(Flags::REPLAY);
        }
        f
    })
}

proptest! {
    #[test]
    fn header_roundtrip(
        dtype in dtype(), flags in flags(), request_id in any::<u64>(), step in any::<u32>(),
        position in any::<u32>(), tokens in 1u16..=4096, hidden in 1u16..=16384,
    ) {
        let header = FrameHeader::new(dtype, flags, request_id, step, position, tokens, hidden).unwrap();
        prop_assert_eq!(FrameHeader::decode(&header.encode()).unwrap(), header);
    }

    #[test]
    fn decoding_arbitrary_bytes_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..64)) {
        let _ = FrameHeader::decode(&bytes);
    }
}

#[test]
fn header_is_32_bytes() {
    assert_eq!(HEADER_LEN, 32);
}

#[test]
fn control_messages_roundtrip() {
    let plan = v1::PlacementPlan {
        version: 3,
        model: Some(v1::ModelRef {
            name: "Qwen3-32B".into(),
            quantization: "Q4_K_M".into(),
            layer_count: 64,
            hidden_dim: 5120,
            ..Default::default()
        }),
        mode: v1::RunMode::Latency as i32,
        stages: vec![v1::Stage {
            layer_begin: 0,
            layer_end: 24,
            backend: v1::Backend::Cuda as i32,
            ..Default::default()
        }],
        replicas: vec![],
        context: 4096,
        // the plan for the first stage's device leaving travels with the plan
        backups: vec![v1::BackupPlan {
            gone: Some(v1::NodeId { ed25519_public_key: vec![7; 32] }),
            plan: Some(v1::PlacementPlan { version: 4, ..Default::default() }),
        }],
        ..Default::default()
    };
    let bytes = plan.encode_to_vec();
    assert_eq!(v1::PlacementPlan::decode(bytes.as_slice()).unwrap(), plan);
}
