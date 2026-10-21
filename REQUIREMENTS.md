# Omnarc — turning every device in the world into one AI

> *Omnarc* = *omni* (all) + *arc*: every device is an arc of one ring. Open source, Apache-2.0. Version 3.2 (October 13, 2026).
> Upstream source code lives in `upstream/`. All reused code is MIT or Apache-2.0 (table in §7).
> **How it gets built** (architecture, technologies, design system, stages) — [PLAN.md](PLAN.md). Every screen and
> function with its state — [SPEC.md](SPEC.md).
> An Uzbek reading copy of these documents is kept in [docs/uz/](docs/uz/).

## 1. The idea

Billions of phones, computers, tablets and TVs sit idle most of the day. Together they hold more compute than any
data center. **Omnarc is an open system that gathers that idle power into one AI network.**

It works at three scales:

| Scale | Who | Example |
|---|---|---|
| **Personal** | your own devices | 2 computers + 3 phones at home = a 70B model |
| **Organization** | an office, a university, a ministry | 500 office PCs become one sovereign AI cluster at night |
| **Public** | volunteers around the world | people who connect a device earn credits to use the network |

It covers inference (answering), training (fine-tuning, RL) and many model types (text, speech, images, embeddings).

### What people use it for

Three uses come first. Each one needs a cluster: one device cannot give it, and a paid cloud gives it only on its own
terms.

1. **Agents on your own model.** Coding and personal agents (OpenClaw, OpenCode and others) run on a frontier-class
   open model — Qwen3.5-397B, DeepSeek-V4-Flash or GLM-5.2 class — spread over the owner's computers: no per-token
   bill, no rate limit, no account that can be closed. At 4 bits such a model needs about 150–400 GB of memory. One
   computer can only stream it from disk: colibri runs GLM-5.2 at 0.05–0.1 tokens/s on a 25 GB laptop and
   1.83 tokens/s with 128 GB (its README, October 2026). A cluster keeps the whole model in memory (A4, W13).
2. **The family ring.** The devices of a family, a class or a team share one assistant that listens and speaks in
   their own language, including languages that small models handle poorly. A weak phone uses it by voice without
   computing anything (Q6); every device the family adds lets a stronger model fit. No subscription, and the data
   stays with the family. Where the internet is cut, nearby devices keep the ring going over a local network or
   Wi-Fi Direct (Y6).
3. **Accuracy mode.** For a question where a mistake is costly, several devices work on it in parallel and check each
   other; the answer shows how many of them agreed (Y10, W12).

## 2. World-class competitors and their limits

| System | Strength | Limit |
|---|---|---|
| exo | Mac clusters, RDMA, tensor parallelism | Mac only; no phones, no Windows; LAN only |
| prima.cpp | 70B on weak devices, Wi-Fi, disk offloading | CUDA only; devices are listed by hand in advance; Android only through Termux |
| GPUStack | managing NVIDIA servers | Linux servers only; no phones, no Macs |
| Parallax (Gradient) | heterogeneous GPUs over the internet, two-phase scheduler | GPUs and Macs only; no phones, no browsers |
| Petals | BitTorrent-style public network | inactive since 2024; GPUs only |
| distributed-llama | tensor parallelism on CPUs | identical devices only, LAN |
| Prime Intellect / Psyche | training over the internet | data-center GPUs only; no inference |
| Mesh LLM (July 2026) | splits a model across computers on a LAN or over the internet (iroh, Rust); OpenAI-compatible API | macOS and Linux; no phones yet, no browsers, no training; in a public test, splitting a model that already fit on one Mac made it 3–5× slower (68 → 21 → 12–13 tokens/s on 1 → 2 → 3 Macs over Wi-Fi) |
| colibri (July 2026) | 744B MoE models on one machine, streaming experts from disk (pure C) | one machine only, bound by its disk: GLM-5.2 at 0.05–0.1 tokens/s on a 25 GB laptop, 1.83 tokens/s with 128 GB |

**None of them does three things at once: any device (phone, browser, NPU), any scale (from one home to the whole
world), and inference together with training.** Omnarc brings all three together.

## 3. Ten ideas that make it world-class

### Y1. More devices make a single answer faster too
In a plain pipeline a token visits the devices one by one while the rest wait.
**Solution:** a continuous pipeline plus speculative decoding. Every device works on a different draft token at the
same moment, so the pipeline never sits idle. Published results: PipeSpec — up to 2.54×, with speed growing as the
pipeline gets deeper; FlowSpec — 1.36–1.77×; DiP-SD (2026).
Slow devices use large draft trees (the SpecExec approach): one verification pass accepts 10–20 tokens.

### Y2. Compute only what is active
- **MoE experts are spread across devices.** DeepSeek V3 (671B) uses only 37B parameters per token, and Kimi K2 (1T)
  only 32B. That is why a trillion-parameter model runs on many ordinary devices.
  The experts needed by the next layer are predicted and prefetched.
- **"Hot" and "cold" neurons are separated** (PowerInfer, MIT). Frequently used neurons live on a fast GPU,
  rarely used ones on a slower device.

### Y3. Every compute unit in a device works
CPU, GPU and NPU work together on one device: Qualcomm Hexagon, Apple Neural Engine, Intel/AMD NPUs.
None of the three original projects uses NPUs. An NPU only runs operations that pass an accuracy gate: each result is
compared with the CPU result, and if the difference exceeds the threshold, that operation moves to the GPU or CPU.
(The llama.cpp Hexagon backend is still experimental and produces wrong output on some models — which is exactly why
the gate exists.)

### Y4. Join without installing anything
Scanning a QR code is enough: a browser tab becomes a cluster node through WebGPU (llama.cpp compiled to WASM +
ggml-webgpu — the same GGUF format as every other device). Any laptop, tablet, TV or a friend's phone can join
without installing anything.

### Y5. One activation protocol, many engines
Different engines work inside one pipeline: ggml (CUDA, Vulkan, Metal, OpenCL, CPU), MLX, WebGPU, vLLM/SGLang.
They speak one standard protocol: the hidden-state tensor (fp16/fp8), positions and a request ID. Each device gets
its layers converted from the same model checkpoint into its own format. A strong device can take Q8 while a weak
phone takes Q4.

### Y6. Every kind of link, compressed traffic
- **Link types:** Wi-Fi, Ethernet, USB-C cable (phone↔computer: lower and steadier latency than Wi-Fi; bandwidth
  depends on the phone model and is measured on connect), Thunderbolt/RDMA, Wi-Fi Direct.
- **Several paths at once:** Wi-Fi and USB are used together.
- **Compression:** activations travel as fp8/int8, cutting traffic 2–4×.
- **No single leader:** devices find each other through a DHT, so the network scales to thousands of nodes.
  It works over the internet: QUIC, NAT hole punching, and a relay when punching fails.

### Y7. A system that does not stop
- Every slice of layers has a replica on another device.
- The KV cache is continuously copied to a neighbor. If a device leaves, the answer is not recomputed from scratch:
  the replica continues from the same point within one second.
- The prefix cache is shared across the whole cluster (the Mooncake approach, Apache-2.0).

### Y8. Training on the same network
- **LoRA fine-tuning** with DiLoCo: each node trains ~500 steps independently, then sends only a compressed
  difference. An ordinary internet connection is enough.
- **RL rollouts** (the INTELLECT-2 approach): inference nodes generate answers at their own pace and a trainer node
  learns from them.
- **Limit:** full pre-training from scratch is not a goal — it needs data-center networking.
- **Who trains:** training runs only on GPU nodes that can run PyTorch (CUDA, ROCm, Apple MPS).
  Phone and browser nodes generate answers for RL (inference).

### Y9. Long context across the memory of many devices
A phone's KV cache fills up quickly. For very long inputs the sequence is split across devices (ring attention),
so the context length grows with the number of devices.

### Y10. More devices make an answer more accurate too
Accuracy can be bought with compute. PaCoRe (January 2026) let an open 8B model reason along many parallel paths that
exchange short messages — about two million tokens per problem — and it scored 94.5% on HMMT 2025, above GPT-5
(93.2%).
**Solution:** in accuracy mode, devices that each hold a whole copy of a model reason along different paths at the same
time, exchange short summaries and compose one answer; the answer shows how many devices agreed. Only short messages
cross the network, so phones and internet links work as well as a LAN.

## 4. Trust and privacy (decisions for the public network)

- **Raw text never leaves the device.** On the public network the first and last layers (embedding and output)
  always run on the requester's own device.
- **Private requests stay in personal or organization clusters.** Text can be partly reconstructed from hidden
  states, so full privacy exists only in those clusters. The panel states this rule openly.
- **Honesty checks.** Every answer carries a TOPLOC proof: the top-k values of the last hidden state. The proof is
  verified ~100× cheaper than recomputation and catches a swapped model, prompt or precision. In addition, 1–2% of
  intermediate-layer work is recomputed on another random node (with a tolerance for different hardware). On a
  mismatch the node's rating drops; repeated mismatches remove it from the network.
- **Credits.** Compute contributed by a device (tokens × layers) is counted and an equal amount of usage credit is
  granted. No blockchain: receipts are signed with Ed25519 and written to a transparency log (C2SP tlog-tiles) that
  anyone can verify.
- **Encryption.** All traffic is encrypted. A device joins a cluster with a QR code or a one-time code.

## 5. Numbered requirements

**Engine (D)**
- D1 — The prima.cpp core (piped-ring, prefetching, mmap) is ported to the current llama.cpp.
- D2 — The Y5 activation protocol; adapters for ggml, MLX, WebGPU and vLLM/SGLang.
- D3 — Y1: continuous pipeline + speculative decoding + large draft trees.
- D4 — Y2: distributing and predicting MoE experts; hot/cold neurons.
- D5 — Y3: CPU+GPU+NPU working together, with the accuracy gate.
- D6 — Dynamic batching, prefill/decode disaggregation (exo `disaggregated`), per-device quantization.
- D7 — Beyond text: speech-to-text, text-to-speech, images, embeddings. Small models are given to phones whole.
- D8 — Y10: accuracy mode — parallel reasoning on whole copies of a model, short messages between the paths, one
  answer composed from them together with the number of devices that agreed.

**Network (T)**
- T1 — The node agent is a single Rust binary for Linux, Windows, macOS, Android and iOS.
- T2 — Discovery through a DHT (plus mDNS on the LAN), QUIC, NAT hole punching, relays.
- T3 — Y6: every link type, multipath transfer, fp8/int8 compression.
- T4 — Latency and bandwidth of every link are measured live.
- T5 — Models are distributed peer-to-peer: each device fetches only its own slice; the internet is used once.

**Scheduler (R)**
- R1 — Built on Halda (prima, ILP) and Parallax's two-phase scheduler. It accounts for memory, compute, disk,
  links, battery, heat and energy (joules per token).
- R2 — When a device joins or leaves at runtime, the plan is rebuilt without stopping.
- R3 — Y7: replica layers, KV cache copies, recovery within ≤ 1 s.
- R4 — The mode is picked automatically per request: latency / throughput / capacity / accuracy.
- R5 — Tensor-parallel groups form only over ≤ 1 ms links; between groups, pipelining.
- R6 — Never slower: a model that fits on one device is never split over a slower link. Extra devices hold whole
  copies instead — more people served at once, accuracy mode (Y10), prefill on a GPU or NPU device (D6). Layers are
  split only when a model does not fit.

**Devices (Q)**
- Q1 — **Every device gives what its owner decides, and nothing more.** This is the same rule for all of them
  -- phone, laptop, workstation, server, browser tab -- and it is not a concession to small devices. An owner
  names a budget: memory, how many cores or how much of the GPU, and the hours it applies. A 2 GB phone may
  offer 500 MB and one core; a 64 GB workstation may offer 8 GB because its owner is working; an office machine
  may offer everything after 19:00 and nothing before. The device is held to that budget whatever it could
  otherwise do. A tray app on Linux, Windows and macOS and the phone apps (Q2, Q3) all show the same control.
- Q2 — Android (minSdk 26): native app with Vulkan/OpenCL/Hexagon. Foreground service type while charging:
  `specialUse` on API ≥ 34, `dataSync` on API < 34 — because from Android 15, `dataSync` is stopped after
  6 hours per day.
- Q3 — iOS (17+): native app with Metal/ANE. iOS does not allow long background compute, so there is a
  **Night mode**: the phone is charging, the app is open, the screen is almost black.
- Q4 — Browser node: llama.cpp (WASM) + WebGPU, joined with a QR code, no install. WebGPU only works on HTTPS pages,
  so the page is loaded from the project domain and cached. On a network with no internet at all, a browser node
  works only if the page was cached before.
- Q5 — Battery and heat limits: a device hands its work to a replica when it reaches them.
- Q6 — No device is too small to be let in. A budget of 500 MB and one core is a real contribution, and the
  scheduler gives such a device a window that fits inside it, or no window at all when a window would not help.
  Holding no layers stays a valid choice -- a device can be a client that only sends speech or text and plays
  the answer -- but that is its owner's choice under Q1, not a rule about what the hardware is.
- Q7 — Work is placed by what each device **offers**, never by how many devices there are and never by what
  they could do if asked for everything. The budget of Q1 enters the placement as a hard constraint: the plan
  may use less of a device than offered, never more. Two devices offering 500 MB each are not one device
  offering 1 GB, and the profiler is what tells them apart. A device whose budget cannot help the plan is left
  out of it rather than slowing everyone down (R6).

**Scale (K)**
- K1 — Personal cluster, including the family ring (§1): the members of a family or a team share one assistant, and
  each member's chat history stays on that member's device.
- K2 — Organization cluster: working-hours schedule, sign-in through Active Directory/LDAP, central policy.
- K3 — Public network: the trust, privacy and credit rules of §4.

**Training (O)**
- O1 — LoRA fine-tuning (DiLoCo / OpenDiLoCo).
- O2 — RL rollouts (prime-rl).

**API and panel (A)**
- A1 — Compatible with the OpenAI, Claude Messages and Ollama APIs (exo `api/adapters`).
- A2 — A live world map: nodes, flows, the layers on each device, energy.
- A3 — Users, keys, token accounting, Prometheus (GPUStack).
- A4 — Agent backend: OpenClaw, OpenCode and other agents connect through A1. The prefix cache keeps their long,
  repeated instructions, so a turn computes only its new tokens; tool calls, structured output and long contexts work.

**UI/UX (U)** — details in [PLAN v3.2 §9](docs/history/PLAN-v3.2.md)
- U1 — The **Ring** home screen. Every device is one arc; the arc length equals its share of layers and its color
  shows the current load. Tokens travel around the ring at their real measured speed; draft tokens are visible too.
- U2 — One design system: tokens are built from a single JSON file (W3C DTCG) into CSS, Swift and Kotlin.
  Two themes (dark by default, light). A single accent color — turquoise.
- U3 — Fonts: Inter + IBM Plex Mono for Latin and Cyrillic; Noto Sans SC, Noto Sans Arabic and Noto Sans Devanagari
  for Chinese, Arabic and Hindi, loaded only for those languages. A 9-step type scale; any other size is forbidden.
- U4 — WCAG 2.2 AA: all text ≥ 4.5:1; status colors always come with an icon and a label; chart colors pass
  color-vision-deficiency checks; every chart has a table view; the whole app works from the keyboard.
- U5 — Motion uses only `transform`/`opacity`; reduced-motion mode turns particles off.
- U6 — Adding a device ≤ 30 s (QR + four-symbol confirmation); first run ≤ 60 s.
- U7 — A ⌘K command palette and a keyboard shortcut for every action.
- U8 — Platform experience: the ring in the desktop tray; iOS Live Activity, Dynamic Island, widget, Siri;
  Android Quick Settings tile, Glance widget; iOS Night mode.
- U9 — Languages: English is the source language; translations into Chinese (Simplified), Spanish, Hindi, Arabic,
  French, Portuguese and Russian. Arabic is laid out right to left; dates and numbers use each language's own formats.
- U10 — Budgets:
  - initial panel JS ≤ 180 KB, browser-node page ≤ 60 KB;
  - 60 fps with 1,000 devices;
  - mobile cold start ≤ 1 s.
- U11 — The **living ring** (a WebGL2 particle ring) shows the system's mechanism live: arc = device,
  gold comet = token, gap = network. It has an intro scene, a 3D tilt and an add-a-device interaction. It is static
  in reduced-motion mode and falls back to SVG without WebGL2. Details: [PLAN v3.2 §9.3](docs/history/PLAN-v3.2.md).

## 6. World-class criteria (measured in the open)

| # | Criterion |
|---|---|
| W1 | Single-request speed grows with the number of devices (measured on 2 → 4 → 8 → 16 devices) |
| W2 | On the same hardware, faster than exo, prima.cpp, Parallax, distributed-llama and Mesh LLM on a 70B model |
| W3 | A 1T MoE (Kimi K2 class) runs on a set of ordinary devices with ≤ 32 GB of memory each |
| W4 | Six node types: Linux, Windows, macOS, Android, iOS, browser |
| W5 | Compute units: CUDA, ROCm/Vulkan, Metal, OpenCL, NPU |
| W6 | If a node drops at runtime, work continues within ≤ 1 s without recomputation |
| W7 | Public network: 1,000+ nodes (in containers, with the real protocol) + at least 20 real devices |
| W8 | Joining from a browser via QR ≤ 30 s |
| W9 | Throughput on an NVIDIA server is no worse than vLLM's |
| W10 | Every benchmark script is open; anyone can reproduce the results |
| W11 | Adding a device never makes a request slower: for a model that fits on one device, TPOT on 1 → 2 → 4 devices over Wi-Fi is never worse than on the best single device |
| W12 | Accuracy mode: 8 devices running the same small model (≤ 4B parameters) score at least 5 points higher than one device on a public maths benchmark (AIME class), with ≤ 1 MB of network traffic per question |
| W13 | Agents on a frontier-class open model: OpenClaw and OpenCode finish a fixed task set on a ≥ 280B open MoE spread over ordinary computers (≤ 64 GB of memory each), faster end to end than colibri and Mesh LLM on the same computers |

## 7. Open-source code we reuse

| Project | License | What we take |
|---|---|---|
| prima.cpp (OpenCPIL) | MIT | piped-ring, Halda, mmap/prefetching |
| llama.cpp / ggml | MIT | the core engine, every backend, Hexagon |
| exo | Apache-2.0 | discovery, placement, API adapters, panel, MLX, disaggregation |
| GPUStack | Apache-2.0 | users, monitoring, vLLM/SGLang management |
| Parallax | Apache-2.0 | internet-scale scheduler, KV management |
| PowerInfer | MIT | hot/cold neurons |
| Petals / hivemind | MIT | DHT, public network, node reputation |
| web-llm | Apache-2.0 | comparison only (the browser node uses llama.cpp WebGPU) |
| distributed-llama | MIT | CPU tensor parallelism |
| Mooncake | Apache-2.0 | distributed KV cache |
| prime-rl / OpenDiLoCo | Apache-2.0 | training |

## 8. Stages

B0–B9. The work and the gate of every stage are in [PLAN v3.2 §11](docs/history/PLAN-v3.2.md); v4 stages S0–S4 are in [PLAN.md §4](PLAN.md). The UI is built together with every
stage (U1–U11), never left for the end.

| Stage | Requirements |
|---|---|
| B0 Foundation | U2, U3, U4 (design system, components), CI, protocol v1 |
| B1 Engine | D1, D2 |
| B2 Network | T1–T5, U1, U6, U11 |
| B3 Scheduler | R1–R6 |
| B4 Speed and accuracy | D3, D4, D6, D8, Y9 |
| B5 Devices | Q1–Q6, D5, U3 (the Noto fonts), U8, U9 (the translations), U10 |
| B6 Scale | K1–K3 |
| B7 Modalities and training | D7, O1, O2 |
| B8 Islands and API | A1–A4, U7 |
| B9 World benchmark | W1–W13, U9 (docs in 8 languages) |

## 9. GitHub

- Repository: `HumoyunDavronov2001/omnarc`.
- `NOTICE` lists every author from §7.
- One pull request per stage. CI builds Linux, Windows, macOS and Android and runs the benchmarks.
