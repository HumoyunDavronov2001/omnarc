# Upstream projects

Omnarc reuses code from these open-source projects. The table records the exact commit we studied; when code is
vendored, its directory gets its own `UPSTREAM.md` with the commit, the license text and the list of our changes,
and `NOTICE` gains its copyright line.

| Project | License | Commit studied | What we take | Status |
|---|---|---|---|---|
| [prima.cpp](https://github.com/OpenCPIL/prima.cpp) | MIT | `288df6a2cc21` (2026-09-01) | piped-ring parallelism, prefetching, device profiler, Halda scheduler | B1: windows and prefetching reimplemented on current llama.cpp; profiler and Halda in B3 |
| [llama.cpp / ggml](https://github.com/ggml-org/llama.cpp) | MIT | `620d2225cc43` (tag v0.7.0, 2026-10-20) | the engine and all backends | submodule `engine/third_party/llama.cpp` + [`engine/patches`](engine/patches/README.md) |
| [exo](https://github.com/exo-explore/exo) | Apache-2.0 | `35182424fef5` (2026-10-12) | discovery, placement, API adapters, dashboard components, MLX engine, prefill/decode split | studied; stages B2, B8 |
| [GPUStack](https://github.com/gpustack/gpustack) | Apache-2.0 | `6f2b17b60a53` (2026-10-12) | users, monitoring, vLLM/SGLang management | studied; stages B6, B8 |
| [Parallax](https://github.com/GradientHQ/parallax) | Apache-2.0 | — | internet-scale scheduling, KV management | stage B3 |
| [PowerInfer](https://github.com/Tiiny-AI/PowerInfer) | MIT | — | hot/cold neuron predictor and sparse kernels | stage B4 |
| [Petals](https://github.com/bigscience-workshop/petals) / hivemind | MIT | — | DHT, public network, node reputation | stage B6 |
| [distributed-llama](https://github.com/b4rtaz/distributed-llama) | MIT | — | CPU tensor parallelism | stage B4 |
| [Mooncake](https://github.com/kvcache-ai/Mooncake) | Apache-2.0 | — | distributed KV cache | stage B4 |
| [TOPLOC](https://github.com/PrimeIntellect-ai/toploc) | MIT | — | verifiable inference proofs | stage B6 |
| [prime-rl](https://github.com/PrimeIntellect-ai/prime-rl) / OpenDiLoCo | Apache-2.0 | — | distributed training | stage B7 |
| [iroh](https://github.com/n0-computer/iroh) | Apache-2.0 / MIT | — | P2P networking (dependency, not vendored) | stage B2 |

## What was taken and changed

**llama.cpp** is used unmodified as a submodule; the engine build applies [`engine/patches`](engine/patches/README.md):

- `0001-llama-add-layer-windows.patch` — a model loads only the layers `[layer_begin, layer_end)` (the output head
  is layer `n_layer`), and a context computes a window of them with hidden states in and out and a KV cache for the
  window's layers only. Architectures: llama, qwen3.

`engine/src/system.cpp` adapts `common_cpu_get_num_physical_cores` from llama.cpp `common/common.cpp`.

**prima.cpp** splits one model into windows of layers that run on the devices of a ring, in several rounds, and
reads the weights of the next window from disk while other devices compute (`src/llama.cpp`: `llama_decode_internal`,
`manage_graph_tensors`). prima.cpp does this in its own 2024 fork of llama.cpp; Omnarc puts the same mechanism on
current llama.cpp: windows are the patch above, one shard per window, and prefetching reads the window's byte ranges
of the GGUF file (`engine/src/system.cpp`, `om_shard_prefetch`), so it does not depend on how llama.cpp maps the
file. prima's "the head device keeps the first layers and the output" becomes the output head as its own window,
`[n_layer, n_layer + 1)`.
