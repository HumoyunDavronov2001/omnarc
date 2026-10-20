# Tensor frame — `omnarc.v1`

Hidden states travel between pipeline stages as **tensor frames** on unidirectional QUIC streams
(WebRTC data channels for browser nodes). Control messages use Protobuf; tensor frames do not, so that a frame can be
written straight into a ggml buffer without parsing or copying.

A frame is a fixed **32-byte header** followed by the payload. All integers are little-endian.

| Offset | Size | Field | Meaning |
|---:|---:|---|---|
| 0 | 4 | `magic` | ASCII `OMT1` |
| 4 | 1 | `version` | `1` |
| 5 | 1 | `dtype` | element type of the hidden state (table below) |
| 6 | 2 | `flags` | bit field (table below) |
| 8 | 8 | `request_id` | request this frame belongs to |
| 16 | 4 | `step` | decoding step within the request (0 = prefill) |
| 20 | 4 | `position_start` | position of the first token in the frame |
| 24 | 2 | `token_count` | number of tokens in the frame (≥ 1) |
| 26 | 2 | `hidden_dim` | model hidden size (≥ 1) |
| 28 | 4 | `payload_len` | payload size in bytes |

## Element types

| `dtype` | Type | Bytes per element | Scales |
|---:|---|---:|---|
| 0 | f32 | 4 | — |
| 1 | f16 | 2 | — |
| 2 | bf16 | 2 | — |
| 3 | f8 (E4M3) | 1 | one f32 scale per token |
| 4 | i8 | 1 | one f32 scale per token |

For the 8-bit types the payload starts with `token_count` f32 scales, followed by the elements:

```
payload_len = token_count × hidden_dim × bytes_per_element      (f32, f16, bf16)
payload_len = token_count × 4 + token_count × hidden_dim          (f8, i8)
```

A receiver rejects a frame whose `payload_len` does not match this formula.

## Flags

| Bit | Name | Meaning |
|---:|---|---|
| 0 | `DRAFT` | the tokens are speculative drafts that still need verification |
| 1 | `LAST_STAGE` | the frame is addressed to the stage that produces logits |
| 2 | `REPLAY` | resent after a failover; the receiver may already hold this step |
| 3 | `MIRROR` | a copy for the replica of these layers: it runs the frame to keep its KV cache in step and forwards nothing |
| 4 | `TOKENS` | the payload is `token_count` little-endian i32 token ids, not a hidden state (`payload_len = token_count × 4`); only the window holding layer 0 takes tokens |
| 5–15 | — | reserved, must be zero |

A receiver rejects a frame with unknown `magic`, an unsupported `version` or `dtype`, reserved flag bits set,
`token_count = 0` or `hidden_dim = 0`.

Integrity and confidentiality come from the transport (QUIC with TLS 1.3, DTLS for WebRTC), so frames carry no
checksum. The reference implementation is `node/crates/omnarc-protocol/src/frame.rs`.

## Links

Between neighbors on a ring, frames travel on **links**: unidirectional QUIC streams (ALPN `omnarc/tensor/1`), one
for each window of layers that the sending device feeds on the receiving one. A link starts with an 8-byte header and
then carries frames back to back until the stream ends.

| Offset | Size | Field | Meaning |
|---:|---:|---|---|
| 0 | 4 | `magic` | ASCII `OML1` |
| 4 | 4 | `window_begin` | the first layer of the receiver's window that the frames feed |

Layers are numbered as in the engine: the repeating layers are `0 … n − 1` and the output head is layer `n`. A
receiver rejects a link whose `window_begin` is not the start of one of its windows. The reference implementation is
`node/crates/omnarc-network/src/lib.rs`.
