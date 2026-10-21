# Engine host protocol — `BKE1`

On computers and servers the engine runs in its own process (PLAN.md §4): if it crashes, the agent keeps running
and moves the layers elsewhere. The agent, `omnarc-node`, starts the engine as a child process:

```
omnarc-engine --shm <name> --shm-size <bytes>
```

- **Requests** arrive on the engine's stdin and **responses** leave on its stdout, one at a time and in order. The
  engine's stderr is its log.
- **Bulk data** — token ids, hidden states, logits — lives in a shared-memory region created by the agent and
  opened by the engine by name: POSIX shared memory (`shm_open`) on Linux, Android and Apple platforms, a named
  file mapping on Windows. The first half of the region is the **input area**, the second half the **output
  area**. The agent removes the name as soon as the engine has opened it, so nothing is left behind after a crash.
- When the agent closes the engine's stdin, the engine exits. When the engine dies, the agent sees its stdout
  close.

Each request is written only after the response to the previous one is read, and the pipes order every write to
shared memory before the message that announces it, so the region needs no other locking.

## Messages

All integers are little-endian. A string is a `u32` byte length followed by UTF-8 bytes.

```
request   u32 length of the rest | u16 opcode | u16 0      | payload
response  u32 length of the rest | u16 status | u16 0      | payload
```

`status` is a `om_status` from [`include/omnarc/omnarc.h`](include/omnarc/omnarc.h). A response with a non-zero
status carries one string, the error message, as its payload.

| Op | Name | Request payload | Response payload |
|---:|---|---|---|
| 1 | `HELLO` | — | `u32` ABI version, string engine version, `u64` region size |
| 2 | `MODEL_INFO` | string path | model info |
| 3 | `OPEN` | string path, `u32` layer begin, `u32` layer end, `u32` backend, `u32` max tokens, from ABI 5 `u32` KV cache type (0 f16, 1 q8_0, 2 q4_0), from ABI 7 `u32` GPU (1 + its index in `DEVICES`, 0 every GPU) and `u32` layers on it (the window's last ones; 0 all) | `u32` shard, shard info |
| 4 | `CLOSE` | `u32` shard | — |
| 5 | `RUN` | `u32` shard, `u64` request, `u32` position, `u32` count, `u8` input, `u8` all logits, `u8` output, `u8` 0 | `u32` rows, `u32` values per row |
| 6 | `DROP` | `u32` shard, `u64` request | — |
| 7 | `TRUNCATE` | `u32` shard, `u64` request, `u32` position | — |
| 8 | `PREFETCH` | `u32` shard | — |
| 9 | `THREADS` | `u32` shard, `u32` threads | — |
| 10 | `TOKENIZE` | `u32` shard, `u8` add special, string text | `u32` count |
| 11 | `TOKEN_TEXT` | `u32` shard, `i32` token | string text, `u8` ends generation |
| 12 | `SAMPLER_OPEN` | `f32` temperature, `u32` top-k, `f32` top-p, `f32` min-p, `u64` seed | `u32` sampler |
| 13 | `SAMPLE` | `u32` sampler, `u32` vocabulary size | `i32` token |
| 14 | `SAMPLER_CLOSE` | `u32` sampler | — |
| 15 | `QUIT` | — | — |
| 16 | `DEVICES` | — | `u32` count, then per GPU: `u32` backend, `u64` memory in bytes, string name (ABI 3), from ABI 7 `u32` 1 when it shares the processor's memory and `u64` its free memory |
| 17 | `KV_SIZE` | `u32` shard, `u64` request | `u64` bytes the cache takes now (ABI 4) |
| 18 | `KV_READ` | `u32` shard, `u64` request | `u64` bytes written, the cache itself in the output half (ABI 4) |
| 19 | `KV_WRITE` | `u32` shard, `u64` request, `u64` bytes; the cache in the input half | — (ABI 4) |
| 20 | `CHAT_PROMPT` | `u32` shard, `u32` count, then count × (string role, string text) | string: the conversation in the model's chat template, ending where the answer begins (ABI 6) |
| 21 | `BENCH` | `u32` GPU index, `u32` milliseconds | `f64` bandwidth in bytes/s (one vector against a 64 MiB f16 matrix), `f64` FLOPS (256 vectors) (ABI 7) |

- **Model info:** string architecture, `u32` layer count, `u32` hidden size, `u32` vocabulary size, `u32` context
  length, `u64` file size, `u8` splittable.
- **Shard info:** model info, `u32` layer begin, `u32` layer end, `u32` backend.
- **`RUN`** reads its input from the input area: `count` token ids (`i32`) for a shard that starts at layer 0,
  otherwise `count × hidden` values in the input type. It writes its output to the output area: hidden states in
  the output type, or `f32` logits for a shard with the output head. Element types: `0` = `i32` token ids,
  `1` = `f32`, `2` = `f16`, `3` = `bf16` — the same 16-bit formats as tensor frames
  ([`protocol/FRAME.md`](../protocol/FRAME.md)), so a frame's payload goes between the network and the engine
  without conversion.
- **`TOKENIZE`** writes the token ids (`i32`) to the output area.
- **`SAMPLE`** reads `f32` logits from the start of the output area, which is where `RUN` left them.
