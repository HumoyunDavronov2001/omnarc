# Omnarc — product specification (v1.1, October 13, 2026)

> Every app, screen and function of Omnarc 1.0, each with the stage that builds it and its state today.
> **What** gets built and why — [REQUIREMENTS.md](REQUIREMENTS.md); **how** — [PLAN.md](PLAN.md); how the screens
> look — [design/concept.html](design/concept.html). An Uzbek reading copy: [docs/uz/SPETSIFIKATSIYA.md](docs/uz/SPETSIFIKATSIYA.md).

**State:** ✓ done · ◐ partly done · ○ not started. **Stage:** B0–B9 from [PLAN.md §11](PLAN.md); an item marked
"B? → Bn" has no stage in the plan yet, and Bn is the proposal (§16). **Req:** the requirement it implements
([REQUIREMENTS.md](REQUIREMENTS.md) §3–§6, PLAN sections as "P§"). **Design:** a mock-up of the screen exists in the
design concept (its section number), or "—".

## 1. The count

<!-- count:begin — written by .github/scripts/spec_count.py from the tables below; do not edit by hand -->
| Part | Screens | Functions | ✓ done | ◐ partly | ○ not started | Ready |
|---|---|---|---|---|---|---|
| Engine | 0 | 30 | 9 | 7 | 14 | 42 % |
| Node agent and API | 0 | 42 | 14 | 16 | 12 | 52 % |
| Panel | 24 | 7 | 5 | 12 | 14 | 35 % |
| Desktop app | 4 | 5 | 0 | 0 | 9 | 0 % |
| Android app | 7 | 8 | 0 | 2 | 13 | 7 % |
| iOS app | 8 | 6 | 0 | 2 | 12 | 7 % |
| Browser node | 3 | 2 | 0 | 0 | 5 | 0 % |
| Organization control | 0 | 7 | 0 | 0 | 7 | 0 % |
| Public network and relay | 0 | 7 | 0 | 0 | 7 | 0 % |
| Training | 0 | 4 | 0 | 0 | 4 | 0 % |
| Islands | 0 | 3 | 0 | 0 | 3 | 0 % |
| Benchmarks, sites and releases | 3 | 9 | 2 | 2 | 8 | 25 % |
| Design system and quality across all apps | 1 | 13 | 6 | 4 | 4 | 57 % |
| **All** | **50** | **143** | **36** | **45** | **112** | **30 %** |

"Ready" counts a partly done item as half. By stage — what each stage has to deliver:

| Stage | Items | ✓ | ◐ | ○ |
|---|---|---|---|---|
| B0 | 10 | 9 | 1 | 0 |
| B1 | 16 | 12 | 2 | 2 |
| B2 | 28 | 7 | 20 | 1 |
| B3 | 16 | 8 | 7 | 1 |
| B4 | 18 | 0 | 9 | 9 |
| B5 | 53 | 0 | 5 | 48 |
| B6 | 23 | 0 | 0 | 23 |
| B7 | 9 | 0 | 0 | 9 |
| B8 | 13 | 0 | 1 | 12 |
| B9 | 7 | 0 | 0 | 7 |
<!-- count:end -->

## 2. What Omnarc is made of

| Part | What people see | Runs on | Built in |
|---|---|---|---|
| Engine (`libomnarc`, `omnarc-engine`, `omnarc-gate`) | nothing — it computes | every device | B1, B4, B5, B7 |
| Node agent (`omnarc-node`) | a command line; the panel on a local port | computers, servers; inside the apps | B1–B3 |
| API | OpenAI, Claude Messages and Ollama endpoints for apps and agents | the node agent | B8 |
| Panel | the web interface of a cluster: the Ring and eight more sections | browser, desktop app | B2–B8 |
| Desktop app | tray ring, window with the panel, installers | Linux, Windows, macOS | B5 |
| Android app | Share my power, rules, assistant, widgets | Android 8+ | B5 |
| iOS app | Share my power, Night mode, Live Activity, assistant | iOS 17+ | B5 |
| Browser node | a tab that joins a cluster from a QR code | Chrome, Edge, Safari, Firefox | B5 |
| Organization control (`omnarc-control`) | sign-in, users, keys, quotas, schedules, audit (screens in the panel) | a server | B6 |
| Public network | directory, proofs, credits (screens in the panel) | relay and directory servers | B6 |
| Training | LoRA and RL jobs (screens in the panel) | GPU nodes; phones and browsers for RL answers | B7 |
| Islands | Mac groups over Thunderbolt 5, GPU servers with vLLM/SGLang | Macs, servers | B8 |
| Benchmarks, sites, releases | results site, docs site, app stores | the web | B1, B9 |
| Design system | tokens and components every screen is built from | all apps | B0 → |

## 3. Engine

| ID | What | Kind | Req | Stage | Design | State |
|---|---|---|---|---|---|---|
| EN-01 | Layer windows [a, b) on llama.cpp through a small patch set; the output head is layer n | function | D1 | B1 | — | ✓ |
| EN-02 | C ABI `omnarc.h` v2: model info, shard open/close/run, KV drop and truncate, tokenizer, sampler | function | D1 | B1 | — | ✓ |
| EN-03 | Hidden state into and out of a window; f16, bf16 or f32 between devices | function | D2 | B1 | — | ✓ |
| EN-04 | Reading the next window ahead from disk when the model does not fit in memory | function | D1 | B1 | — | ✓ |
| EN-05 | The engine in its own process (`omnarc-engine`), shared memory to the agent | function | P§4 | B1 | — | ✓ |
| EN-06 | Accuracy gate tool (`omnarc-gate`): KL, perplexity, top-1, exact mode, any split | function | P§4 | B1 | — | ✓ |
| EN-07 | Backends: CPU, Metal and Vulkan tested in CI; CUDA and Vulkan on Windows built | function | W5 | B1 | — | ✓ |
| EN-08 | Backends HIP/ROCm, OpenCL, SYCL and CANN built and gated | function | W5 | B5 | — | ○ |
| EN-09 | Architectures with windows: Llama, Qwen3 | function | D1 | B1 | — | ✓ |
| EN-10 | Architectures for the uses: the MoE families (Qwen3.5, DeepSeek-V4, GLM-5, Kimi) and Gemma 4 | function | D4 | B4 | — | ○ |
| EN-11 | Patches rebased weekly on the newest llama.cpp; sent upstream as pull requests | function | P§4 | B1 | — | ◐ |
| EN-12 | Continuous pipeline with speculative decoding (PipeSpec, FlowSpec, DiP-SD) | function | D3, Y1 | B4 | — | ◐ |
| EN-13 | Draft sources: MTP heads, EAGLE-3, a small model, n-grams; large draft trees for slow devices | function | D3 | B4 | — | ◐ |
| EN-14 | Hot and cold neurons (PowerInfer) | function | D4 | B4 | — | ○ |
| EN-15 | MoE experts placed by co-activation, predicted and fetched ahead; hot experts copied (EPLB) | function | D4 | B4 | — | ○ |
| EN-16 | Paged KV cache, radix prefix cache, q8/q4 KV | function | P§4 | B4 | — | ◐ |
| EN-17 | A replica is kept in step with the device that is answering, so it continues without recomputing: it is sent the same inputs and runs them, and a cache can also be handed over directly | function | R3, Y7 | B3 | — | ✓ |
| EN-18 | Ring attention: long context across the memory of many devices | function | Y9 | B4 | — | ○ |
| EN-19 | f8e4m3 activations with a per-token scale | function | T3 | B4 | — | ◐ |
| EN-20 | Continuous batching | function | D6 | B4 | — | ○ |
| EN-21 | Prefill and decode on different devices | function | D6 | B4 | — | ○ |
| EN-22 | Quantization per device from one checkpoint (Q8 on a strong device, Q4 on a weak one) | function | D6, Y5 | B4 | — | ○ |
| EN-23 | Accuracy mode: parallel paths on whole copies, short summaries between them, one composed answer | function | D8, Y10 | B4 | — | ◐ |
| EN-24 | NPU operations behind the accuracy gate (Hexagon, Apple Neural Engine, Intel/AMD NPU) | function | D5, Y3 | B5 | — | ○ |
| EN-25 | The engine inside the phone apps (in-process, through the Rust core) | function | Q2, Q3 | B5 | — | ○ |
| EN-26 | The engine in the browser: llama.cpp in WASM with ggml-webgpu | function | Q4 | B5 | — | ○ |
| EN-27 | One activation protocol across engines: ggml, MLX, WebGPU, vLLM/SGLang adapters | function | D2, Y5 | B8 | — | ◐ |
| EN-28 | Speech to text and text to speech | function | D7 | B7 | — | ○ |
| EN-29 | Images (stable-diffusion.cpp) | function | D7 | B7 | — | ○ |
| EN-30 | Embeddings | function | D7 | B7 | — | ○ |

## 4. Node agent and API

### 4.1 Program and service

| ID | What | Kind | Req | Stage | Design | State |
|---|---|---|---|---|---|---|
| NO-01 | `omnarc-node info`: the device as it announces itself | function | T1 | B0 | — | ✓ |
| NO-02 | `omnarc-node id`: the node key (Ed25519), created on first use | function | P§3 | B1 | — | ✓ |
| NO-03 | `omnarc-node ring`: a ring wired by hand — head and workers, time per output token | function | D1 | B1 | — | ✓ |
| NO-04 | A long-running service (systemd, launchd, Windows service) that serves the panel on a local port | function | T1 | B2 | — | ◐ |
| NO-05 | Creating and joining clusters from the command line (ticket, code) | function | P§3 | B2 | — | ◐ |
| NO-06 | One Rust program for Linux, Windows, macOS, Android and iOS | function | T1 | B5 | — | ◐ |
| NO-07 | Docker image and Helm chart for servers | function | P§6 | B9 | — | ○ |

### 4.2 Network

| ID | What | Kind | Req | Stage | Design | State |
|---|---|---|---|---|---|---|
| NO-10 | QUIC links dialed by node key (iroh); tensor frames on streams; only the expected neighbor is accepted | function | T2 | B1 | — | ✓ |
| NO-11 | NAT hole punching, and our own relay when punching fails | function | T2 | B2 | — | ◐ |
| NO-12 | Discovery: mDNS on the LAN, DNS/pkarr on the internet | function | T2 | B2 | — | ◐ |
| NO-13 | Pairing: QR ticket with a 10-minute secret, four-emoji confirmation on both screens | function | P§3 | B2 | 02 | ✓ |
| NO-14 | Cluster key and membership tokens (Biscuit) with node, user and admin rights | function | P§3 | B2 | — | ◐ |
| NO-15 | Telemetry once per second over gossip | function | P§3 | B2 | — | ◐ |
| NO-16 | Raft on 3–5 stable nodes: plan version, placement, keys | function | P§3 | B2 | — | ◐ |
| NO-17 | Latency and bandwidth of every link, measured live | function | T4 | B2 | — | ✓ |
| NO-18 | Every link type (Wi-Fi, Ethernet, USB-C, Thunderbolt, Wi-Fi Direct), several at once | function | T3, Y6 | B5 | — | ○ |
| NO-19 | WebRTC to browser nodes, signaling through the relay | function | Q4 | B5 | — | ○ |
| NO-20 | Message schemas `omnarc.v1` (device, pairing, telemetry, plan, cluster service) and the tensor frame | function | P§3 | B0 | — | ✓ |

### 4.3 Models

| ID | What | Kind | Req | Stage | Design | State |
|---|---|---|---|---|---|---|
| NO-30 | A model is downloaded from Hugging Face once, hashed with BLAKE3 and shared peer to peer (iroh-blobs) | function | T5 | B2 | — | ✓ |
| NO-31 | Each device fetches only its own tensors as byte ranges and resumes after a drop | function | T5 | B2 | — | ✓ |
| NO-32 | The models a cluster can run: catalog with sizes, quantizations and licenses | function | — | B8 | — | ○ |

### 4.4 Scheduler

| ID | What | Kind | Req | Stage | Design | State |
|---|---|---|---|---|---|---|
| NO-40 | Profiler: a 20-second benchmark of compute, memory bandwidth, disk and NPU when a device joins | function | R1 | B3 | — | ◐ |
| NO-41 | Placement with HiGHS (≤ 1 s for 64 devices; larger networks in groups) | function | R1, W6 | B3 | 04 | ✓ |
| NO-42 | Routing every request through the replicas (Parallax) | function | R1 | B3 | — | ◐ |
| NO-43 | Devices join and leave while models run; a precomputed backup plan takes over within 1 s | function | R2, R3 | B3 | — | ◐ |
| NO-51 | A device announces a budget (memory, cores or GPU share, hours) and the placement never exceeds it | function | Q1, Q7 | B3 | — | ◐ |
| NO-52 | A device too small for any window is left out of the plan instead of slowing it down | function | Q7, R6 | B3 | — | ✓ |
| NO-44 | Never slower: a model that fits on one device is not split over a slower link | function | R6, W11 | B3 | — | ✓ |
| NO-45 | The mode of each request: latency, throughput, capacity, accuracy | function | R4 | B3 | 01 | ◐ |
| NO-46 | Tensor-parallel groups only over links of ≤ 1 ms | function | R5 | B3 | — | ○ |
| NO-47 | Battery, heat, schedule and idle limits hand work to a replica | function | Q5 | B5 | 03 | ○ |
| NO-48 | Plan suggestions: where moving layers would make the ring faster | function | R1 | B3 | 01 | ✓ |
| NO-49 | Re-planning and KV copies modeled in TLA+ and checked in TLC | function | P§8 | B3 | — | ✓ |
| NO-50 | 1,000 random drop and join scenarios in a network simulation without a lost answer | function | P§8 | B2 | — | ◐ |

### 4.5 API

| ID | What | Kind | Req | Stage | Design | State |
|---|---|---|---|---|---|---|
| AP-01 | Control API (Connect RPC) for the panel and the apps, with a live stream | function | P§3 | B2 | — | ◐ |
| AP-02 | OpenAI-compatible: chat completions, completions, embeddings, models, streaming | function | A1 | B8 | — | ○ |
| AP-03 | Claude Messages compatible | function | A1 | B8 | — | ○ |
| AP-04 | Ollama compatible | function | A1 | B8 | — | ○ |
| AP-05 | Agent backend: long repeated instructions kept in the prefix cache, tool calls, structured output, long contexts | function | A4 | B8 | — | ○ |
| AP-06 | API keys (Biscuit) limited by model, quota and expiry | function | P§7 | B6 | — | ○ |
| AP-07 | Audio and image endpoints | function | D7 | B7 | — | ○ |
| AP-08 | Metrics (Prometheus) and traces (OpenTelemetry); nothing is sent anywhere by default | function | A3 | B4 | — | ◐ |

## 5. Panel

The panel is the web interface of a cluster. Every node serves it; the desktop app shows it in its window. It has
nine sections in a rail: Ring, Devices, Models, Chat, Observe, Training, Network, Organization, Settings.

| ID | What | Kind | Req | Stage | Design | State |
|---|---|---|---|---|---|---|
| PA-01 | Frame: the rail of sections, the top bar, the footer | screen | U1 | B2 | 01 | ◐ |
| PA-02 | First run | screen | U6 | B2 | — | ◐ |
| PA-03 | Ring (home) | screen | U1 | B2 | 01 | ◐ |
| PA-04 | Add a device | screen | U6 | B2 | 02 | ◐ |
| PA-05 | Devices | screen | — | B2 | 01 | ◐ |
| PA-06 | A device | screen | Q1, Q5 | B2 | — | ◐ |
| PA-07 | People (the family ring) | screen | K1, Q6 | B6 | — | ○ |
| PA-08 | Models: catalog | screen | — | B8 | — | ○ |
| PA-09 | Will it fit? and the placement plan | screen | — | B3 | 04 | ◐ |
| PA-10 | Running models and downloads | screen | T5 | B3 | — | ✓ |
| PA-11 | Use from apps and agents | screen | A4 | B8 | — | ○ |
| PA-12 | Chat | screen | D8 | B4 | — | ◐ |
| PA-13 | Observe: token path | screen | — | B4 | 05 | ◐ |
| PA-14 | Observe: links and traces | screen | T4 | B4 | 05 | ◐ |
| PA-15 | Training | screen | O1, O2 | B7 | — | ○ |
| PA-16 | Network: the public network | screen | A2, K3 | B6 | 06 | ○ |
| PA-17 | Credits | screen | K3 | B6 | 06 | ○ |
| PA-18 | Organization: sign-in | screen | K2 | B6 | — | ○ |
| PA-19 | Organization: users and roles | screen | K2 | B6 | — | ○ |
| PA-20 | Organization: keys and quotas | screen | A3 | B6 | — | ○ |
| PA-21 | Organization: schedules and policy | screen | K2 | B6 | — | ○ |
| PA-22 | Organization: audit and usage | screen | K2, A3 | B6 | — | ○ |
| PA-23 | Settings | screen | U9 | B2 | — | ◐ |
| PA-24 | Command palette (⌘K) | screen | U7 | B8 | 01 | ○ |
| PA-30 | Notifications: a device left and its replica took over, a device joined, a model is ready, errors | function | P§9 | B3 | — | ◐ |
| PA-31 | Plan suggestion card with Apply and Dismiss | function | R1 | B3 | 01 | ✓ |
| PA-32 | The ring for everyone: a table view, keyboard focus from arc to arc, a spoken summary | function | U4 | B2 | — | ✓ |
| PA-33 | Several models as nested rings; arcs grouped beyond 128 devices | function | U1 | B4 | — | ○ |
| PA-34 | The living ring in the panel: intro, tilt, a new arc when a device joins | function | U11 | B2 | 01 | ✓ |
| PA-35 | Live data over a Connect stream | function | P§9 | B2 | — | ✓ |
| PA-36 | A shortcut for every action | function | U7 | B8 | — | ○ |

**PA-01 Frame.** *Shows:* the logo, the nine sections with icons, the cluster name and a health chip ("6 devices · all
healthy"), search, the Add device button; at the bottom, the public network switch and the credits. *Actions:* move
between sections; open ⌘K; add a device. *States:* the Organization section only appears in organization clusters.

**PA-02 First run.** *Shows:* the device found (name, backend, memory), a 20-second benchmark, the clusters already on
the LAN, the share slider, the living ring that grows a new arc. *Actions:* create a cluster or join one; set the
share. *States:* no WebGPU or backend found → CPU only, said plainly; ≤ 60 s from start to the arc.

**PA-03 Ring.** *Shows:* every device as an arc (length = share of layers, color = load), tokens moving around the
ring at the measured speed, draft tokens as faint sparks; in the center the tokens per second, the model and the mode;
thin outer markers for charging, temperature and battery; cards for draft acceptance, active requests, memory and
energy; the device list with busy time and temperature; the token path of the last 120 ms; a plan suggestion.
*Actions:* switch the mode (Latency, Throughput, Capacity, Accuracy); choose a model; open a device; switch to the
table view. *States:* no model running → "Choose a model"; one device → one full circle; a device leaving → its arc
fades and its replica lights up.

**PA-04 Add a device.** *Shows:* on the computer a QR code and a code with a 10-minute timer; on both screens the same
four emoji; at the end the new arc and what it took ("Took 6 layers · Vulkan · 8 GB · 0.4 ms over USB-C").
*Actions:* scan or type the code; confirm "Yes, they match" or "No, they're different". *States:* expired code; the
symbols differ → the device is refused; ≤ 30 s in total.

**PA-05 Devices.** *Shows:* a table of devices — name, kind, backend, memory share, layers, temperature, battery,
link to the neighbor, state. *Actions:* sort, filter, open a device, remove a device. *States:* offline devices
greyed out with the time they were last seen.

**PA-06 A device.** *Shows:* its backend and compute units, memory, temperature, battery, its layers, the links to its
neighbors with latency and bandwidth, the benchmark result. *Actions:* set the share (memory %, GPU %, CPU cores), the
schedule and "only while idle"; run the benchmark again; remove. *States:* at a battery or heat limit, a banner says
its work moved to a replica.

**PA-07 People.** *Shows:* the members of a family or a team, each with their devices, and the phones that only use the
assistant by voice. *Actions:* invite a member with a QR code; remove a member; decide who may add devices. *States:*
each member's chats stay on their own device; the panel says so.

**PA-08 Models: catalog.** *Shows:* models with size, quantization, license and whether they fit the cluster; search
on Hugging Face. *Actions:* filter by size, task (text, speech, images, embeddings) and license; open Will it fit?

**PA-09 Will it fit?** *Shows:* the memory the model needs against the memory of the cluster, the placement plan drawn
on the ring before anything is downloaded, the expected speed range from the device benchmarks. *Actions:* launch;
choose another quantization. *States:* does not fit → how much memory is missing and which device would close the gap.

**PA-10 Running models and downloads.** *Shows:* each running model with its ring, its mode and its load; downloads
with the progress of every device's slice. *Actions:* stop, restart, change the mode; pause a download.

**PA-11 Use from apps and agents.** *Shows:* the cluster's API address, the keys, and ready settings to copy for
OpenClaw, OpenCode and other agents and for the OpenAI, Claude and Ollama SDKs. *Actions:* create a key limited by
model, quota and expiry; revoke a key; test a call. *States:* requests per key and tokens used.

**PA-12 Chat.** *Shows:* a conversation with the cluster; under each answer the devices that computed it, colored per
device; in accuracy mode, how many devices agreed. *Actions:* choose the model and the mode; stop; copy; ask by voice.
*States:* the model is still loading → the answer waits with a progress line.

**PA-13 Observe: token path.** *Shows:* a timeline of the last seconds with compute, network, draft and idle time per
device, side by side with a plain pipeline; the idle share. *Actions:* pause, zoom, pick a request. *States:* the
slowest stage is marked.

**PA-14 Observe: links and traces.** *Shows:* the matrix of link latency and bandwidth between devices; the traces of
single requests. *Actions:* open a link or a request; export a trace.

**PA-15 Training.** *Shows:* LoRA and RL jobs, their rounds, the loss curve, the nodes taking part. *Actions:* start a
job (base model, data, rounds), pause, stop, download the adapter. *States:* only GPU nodes with PyTorch train; phones
and browsers produce RL answers.

**PA-16 Network.** *Shows:* the globe of public nodes and the flows between clusters; numbers of nodes, countries and
clusters; the rules (raw text stays on your device, every answer is proven, credits in a public log). *Actions:*
join or leave the public network; choose what the device gives.

**PA-17 Credits.** *Shows:* the balance, what was earned this month, the signed receipts and the public log entry of
each. *Actions:* verify a receipt; spend credits on public models.

**PA-18 Organization: sign-in.** *Shows:* sign-in through the organization's OIDC or LDAP/Active Directory.
*States:* wrong role → what the user may do and whom to ask.

**PA-19 Organization: users and roles.** *Shows:* users with their roles (admin, user, node owner). *Actions:* invite,
change a role, remove.

**PA-20 Organization: keys and quotas.** *Shows:* API keys and quotas per user, team and model. *Actions:* create,
limit, revoke.

**PA-21 Organization: schedules and policy.** *Shows:* when office computers work for the cluster (for example at
night) and what they may give. *Actions:* set working hours and limits for groups of computers; allow models.

**PA-22 Organization: audit and usage.** *Shows:* the audit log and tokens used per user, team and model. *Actions:*
filter and export.

**PA-23 Settings.** *Shows:* language (English and the seven translations), theme (dark, light), security (this node's
key, trusted devices), updates, the note that nothing is sent anywhere. *Actions:* change them.

**PA-24 Command palette.** *Shows:* every action of the panel with its shortcut. *Actions:* type to find and run.

## 6. Desktop app

| ID | What | Kind | Req | Stage | Design | State |
|---|---|---|---|---|---|---|
| DE-01 | Tray: a small ring filled by the device's load, and its menu | screen | Q1, U8 | B5 | 03 | ○ |
| DE-02 | The window: the panel inside the app | screen | — | B5 | 01 | ○ |
| DE-03 | First run in the app | screen | U6 | B5 | — | ○ |
| DE-04 | Share and schedule | screen | Q1 | B5 | — | ○ |
| DE-10 | The agent runs inside the app; memory stays free for the model (Tauri, ~10 MB) | function | P§2 | B5 | — | ○ |
| DE-11 | Starts at login and keeps working in the background | function | Q1 | B5 | — | ○ |
| DE-12 | Signed automatic updates | function | P§2 | B5 | — | ○ |
| DE-13 | System notifications | function | — | B5 | — | ○ |
| DE-14 | Installers: MSI, DMG (notarized), AppImage, deb, rpm, Flatpak, Homebrew, winget, Nix | function | P§6 | B9 | — | ○ |

**DE-01 Tray.** *Shows:* the ring icon filled by the load. *Actions (menu):* open Omnarc; pause or resume sharing;
quick share level; quit.

**DE-03 First run.** The same steps as the panel's first run, plus "start at login".

**DE-04 Share and schedule.** *Shows:* sliders for memory %, GPU % and CPU cores; working hours; "only while idle".
*Actions:* change them; the limits are always visible.

## 7. Android app

The app has four tabs — Contribute, Cluster, Chat, Settings — like the design concept's phone.

| ID | What | Kind | Req | Stage | Design | State |
|---|---|---|---|---|---|---|
| AN-01 | First run | screen | U6 | B5 | — | ○ |
| AN-02 | Contribute: Share my power | screen | P§9 | B5 | 03 | ○ |
| AN-03 | Join: scan the QR code, confirm the symbols | screen | P§3 | B5 | 02 | ○ |
| AN-04 | Rules: when it runs | screen | Q5 | B5 | 03 | ○ |
| AN-05 | Cluster: the ring on the phone | screen | U1 | B5 | — | ○ |
| AN-06 | Chat and assistant: text, and voice for weak phones | screen | Q6 | B5 | — | ○ |
| AN-07 | Settings | screen | U9 | B5 | — | ○ |
| AN-10 | Foreground service while charging: `specialUse` on API ≥ 34, `dataSync` below | function | Q2 | B5 | — | ○ |
| AN-11 | Quick Settings tile | function | U8 | B5 | — | ○ |
| AN-12 | Glance widget | function | U8 | B5 | — | ○ |
| AN-13 | Live status in the notification | function | U8 | B5 | — | ○ |
| AN-14 | Vulkan, OpenCL and Hexagon NPU | function | Q2 | B5 | — | ○ |
| AN-15 | The Rust core in the app: it compiles for Android in CI; the UniFFI bindings to Kotlin are not written | function | T1 | B5 | — | ◐ |
| AN-16 | Android 8 to 16 (API 26–36); Maestro flows | function | W4 | B5 | — | ○ |
| AN-17 | Material 3 Expressive with the shared tokens | function | U2 | B5 | — | ◐ |

**AN-01 First run.** *Shows:* what the app does, the notification and camera permissions. *Actions:* join a cluster
(scan) or use the phone only as an assistant.

**AN-02 Contribute.** *Shows:* "Your power is in the cluster", the cluster, its layers and model; charging, Wi-Fi and
temperature; today's tokens, energy and credits; one big button, Share my power. *Actions:* start or stop sharing.
*States:* waiting for its rules (for example, not charging) → which rule is not met.

**AN-03 Join.** *Shows:* the camera, then the cluster found ("Home cluster · 5 devices"), then the four emoji.
*Actions:* join; confirm the symbols match or not.

**AN-04 Rules.** *Shows:* only while charging (recommended), only on Wi-Fi, only when the screen is off, minimum
battery 40 %, temperature limit 40 °C, memory share, schedule. *Actions:* change each rule. *States:* at a limit the
phone hands its work to a backup device — the screen says so.

**AN-05 Cluster.** *Shows:* the ring with this phone's arc highlighted, the model, the speed.

**AN-06 Chat and assistant.** *Shows:* the conversation; a hold-to-speak button; the answer is spoken back. A phone
with ≤ 4 GB of memory works only as this client and computes nothing (Q6). *Actions:* speak, type, stop.

**AN-07 Settings.** *Shows:* language, theme, the cluster and its key, about.

## 8. iOS app

The same four tabs as on Android; iOS does not allow long background work, so the phone contributes in Night mode.

| ID | What | Kind | Req | Stage | Design | State |
|---|---|---|---|---|---|---|
| IO-01 | First run | screen | U6 | B5 | — | ○ |
| IO-02 | Contribute: Share my power | screen | P§9 | B5 | 03 | ○ |
| IO-03 | Join: scan the QR code, confirm the symbols | screen | P§3 | B5 | 02 | ○ |
| IO-04 | Rules: when it runs | screen | Q5 | B5 | 03 | ○ |
| IO-05 | Night mode: an almost-black screen with the time and the ring while charging | screen | Q3 | B5 | 03 | ○ |
| IO-06 | Cluster: the ring on the phone | screen | U1 | B5 | — | ○ |
| IO-07 | Chat and assistant: text, and voice for weak phones | screen | Q6 | B5 | — | ○ |
| IO-08 | Settings | screen | U9 | B5 | — | ○ |
| IO-10 | Live Activity on the Lock Screen and in the Dynamic Island | function | U8 | B5 | 03 | ○ |
| IO-11 | Widget (WidgetKit) | function | U8 | B5 | — | ○ |
| IO-12 | Siri and Shortcuts: "Share my power" | function | U8 | B5 | — | ○ |
| IO-13 | Metal and the Neural Engine | function | Q3 | B5 | — | ○ |
| IO-14 | The Rust core in the app: it compiles for iOS in CI; the XCFramework with UniFFI bindings to Swift is not built | function | T1 | B5 | — | ◐ |
| IO-15 | Liquid Glass (iOS 26) with the shared tokens | function | U2 | B5 | — | ◐ |

**IO-05 Night mode.** *Shows:* the time, the date, the ring, "Contributing · 4 layers · 31 °C". *Actions:* tap for
normal mode. *States:* not charging → Night mode ends and the work moves to a replica.

**IO-10 Live Activity.** *Shows:* overnight progress on the Lock Screen ("41,300 tokens · 6 h 12 min") and the ring in
the Dynamic Island.

The other iOS screens match the Android ones (§7).

## 9. Browser node

| ID | What | Kind | Req | Stage | Design | State |
|---|---|---|---|---|---|---|
| BR-01 | Join page opened from the QR code | screen | Q4, W8 | B5 | — | ○ |
| BR-02 | Contributing | screen | Q4 | B5 | — | ○ |
| BR-03 | No internet and no cached page | screen | Q4 | B5 | — | ○ |
| BR-10 | Installable web app (PWA) from the project's HTTPS address, cached; ≤ 60 KB page | function | U10 | B5 | — | ○ |
| BR-11 | WebRTC to LAN nodes; signaling through the relay | function | Q4 | B5 | — | ○ |

**BR-01 Join page.** *Shows:* which cluster invites the tab, whether this browser has WebGPU, how much memory it can
give. *Actions:* join; ≤ 30 s from the scan. *States:* no WebGPU → the tab can still join for RL answers on CPU or is
told it cannot help.

**BR-02 Contributing.** *Shows:* the tab's arc on the ring, its tokens per second, "keep this tab open". *Actions:* stop.

**BR-03 No internet.** *Shows:* that a browser node needs the page cached once; how to install the app instead.

## 10. Organization control

The screens are in the panel (PA-18 to PA-22); this is the service behind them.

| ID | What | Kind | Req | Stage | Design | State |
|---|---|---|---|---|---|---|
| OR-01 | `omnarc-control` service (Python, FastAPI, PostgreSQL 17) built on GPUStack's server | function | K2 | B6 | — | ○ |
| OR-02 | Sign-in through OIDC and LDAP/Active Directory | function | K2 | B6 | — | ○ |
| OR-03 | Roles and permissions (RBAC) | function | K2 | B6 | — | ○ |
| OR-04 | Quotas and token accounting | function | A3 | B6 | — | ○ |
| OR-05 | Working-hours schedules for groups of computers | function | K2 | B6 | — | ○ |
| OR-06 | Audit log | function | K2 | B6 | — | ○ |
| OR-07 | Central policy: what devices may give, which models are allowed | function | K2 | B6 | — | ○ |

## 11. Public network and relay

| ID | What | Kind | Req | Stage | Design | State |
|---|---|---|---|---|---|---|
| PU-01 | `omnarc-relay`: relay, WebRTC signaling, directory | function | T2 | B2 | — | ○ |
| PU-02 | Directory of public nodes; DHT discovery | function | K3, T2 | B6 | 06 | ○ |
| PU-03 | The first and last layers always run on the requester's device | function | K3 | B6 | 06 | ○ |
| PU-04 | A TOPLOC proof with every answer | function | K3 | B6 | 06 | ○ |
| PU-05 | 1–2 % of the work recomputed on a random node; ratings; cheating nodes removed | function | K3 | B6 | — | ○ |
| PU-06 | Credits: compute counted, receipts signed with Ed25519, written to a transparency log | function | K3 | B6 | 06 | ○ |
| PU-07 | 1,000+ nodes in containers with the real protocol, and at least 20 real devices | function | W7 | B6 | — | ○ |

## 12. Training

| ID | What | Kind | Req | Stage | Design | State |
|---|---|---|---|---|---|---|
| TR-01 | LoRA fine-tuning with DiLoCo over the internet on GPU nodes (PyTorch) | function | O1 | B7 | — | ○ |
| TR-02 | RL rollouts: phones and browsers answer, a trainer learns | function | O2 | B7 | — | ○ |
| TR-03 | LoRA for a low-resource language on 4+ nodes over the internet (the B7 gate) | function | O1 | B7 | — | ○ |
| TR-04 | Jobs with checkpoints that survive nodes leaving | function | O1 | B7 | — | ○ |

## 13. Islands

| ID | What | Kind | Req | Stage | Design | State |
|---|---|---|---|---|---|---|
| IS-01 | MLX island: Macs joined over Thunderbolt 5 RDMA (from exo) | function | Y5 | B8 | — | ○ |
| IS-02 | GPU island: NVIDIA and AMD servers with vLLM or SGLang (from GPUStack) | function | Y5 | B8 | — | ○ |
| IS-03 | Throughput on an NVIDIA server no worse than vLLM's | function | W9 | B8 | — | ○ |

## 14. Benchmarks, sites and releases

| ID | What | Kind | Req | Stage | Design | State |
|---|---|---|---|---|---|---|
| BE-01 | CI on six platforms, the engine backends, the weekly llama.cpp rebase | function | P§8 | B0 | — | ✓ |
| BE-02 | B1 gate tools: model downloads with checksums, the accuracy gate script, prima.cpp built for comparison | function | W10 | B1 | — | ✓ |
| BE-03 | B1 gate: accuracy on Llama 3.x 8B, Qwen3 32B and R1-Distill 70B | function | P§11 | B1 | — | ◐ |
| BE-04 | B1 gate: a 70B model across Linux and Windows computers | function | P§11 | B1 | — | ○ |
| BE-05 | B1 gate: time per output token no worse than prima.cpp's | function | P§11 | B1 | — | ○ |
| BE-06 | `omnarc-bench`: W1–W13 against exo, prima.cpp, Parallax, distributed-llama, llama.cpp RPC, Mesh LLM, colibri | function | W1–W13 | B9 | — | ○ |
| BE-07 | 24-hour fuzzing of the frame parser, protobuf and the GGUF reader | function | P§8 | B2 | — | ◐ |
| BE-08 | Results site | screen | W10 | B9 | — | ○ |
| BE-09 | Docs site (Astro Starlight, 8 languages, offline search) | screen | U9 | B9 | — | ○ |
| BE-10 | Project site with the browser-node page on its own HTTPS domain | screen | Q4 | B5 | — | ○ |
| BE-11 | Signed releases: cosign signatures, SBOM, SLSA provenance | function | P§2 | B9 | — | ○ |
| BE-12 | Stores: Google Play, F-Droid, App Store and TestFlight, Homebrew, winget, Flathub, Docker, Helm | function | P§12 | B9 | — | ○ |

## 15. Design system and quality across all apps

| ID | What | Kind | Req | Stage | Design | State |
|---|---|---|---|---|---|---|
| DS-01 | Tokens from one source into CSS, Swift, Kotlin and TypeScript; the contrast gate | function | U2 | B0 | 07 | ✓ |
| DS-02 | Self-hosted fonts: Inter and IBM Plex Mono | function | U3 | B0 | 07 | ✓ |
| DS-03 | Noto Sans SC, Arabic and Devanagari, loaded per language | function | U3 | B5 | — | ○ |
| DS-04 | Components built and tested (unit, visual, axe): Button, Card, Chip, Field, Icon, Kbd, Logo, Meter, Ring, Status, Switch, LivingRing | function | U2, U4 | B0 | 07 | ✓ |
| DS-05 | Components the screens still need: Tabs, Table, Dialog, Toast, Menu, Select, Slider, Tooltip, Progress, Empty state, Skeleton, QR code, Emoji check, Code block, Message, Voice button, Agreement badge | function | U2 | B2–B8 | — | ◐ |
| DS-06 | Charts: uPlot series, the token-path timeline, the link matrix | function | U4 | B4 | 05 | ○ |
| DS-07 | The globe: MapLibre GL, deck.gl and offline Protomaps tiles | function | A2 | B6 | 06 | ○ |
| DS-08 | The living ring (WebGL2) and its SVG fallback | function | U11 | B0 | 01 | ✓ |
| DS-09 | Design concept page and README pictures | screen | — | B0 | — | ✓ |
| XC-01 | Translations into Chinese, Spanish, Hindi, Arabic (right to left), French, Portuguese and Russian | function | U9 | B5 | — | ○ |
| XC-02 | WCAG 2.2 AA, keyboard only, reduced motion, charts safe for color-vision deficiency | function | U4, U5 | B0 → | 07 | ◐ |
| XC-03 | Budgets: panel JS ≤ 180 KB, browser page ≤ 60 KB, 60 fps with 1,000 devices, LCP ≤ 1.5 s, cold start ≤ 1 s | function | U10 | B2 → | — | ◐ |
| XC-04 | Encryption on every link; models checked with BLAKE3 | function | P§7 | B2 | — | ◐ |
| XC-05 | Nothing sent anywhere by default | function | P§2 | B0 → | — | ✓ |

## 16. Items the plan has not placed in a stage yet

None. An item that the plan does not place yet is marked "B? → Bn" in its table and listed here until it is agreed.
The first count (October 13, 2026) found six, and the same day they joined the plan (PLAN.md v3.2, §11):

| ID | Item | Stage | Why there |
|---|---|---|---|
| PA-12 | Chat | B4 | the first stage whose answers show their devices and accuracy mode |
| PA-23 | Settings | B2 | the first panel needs language and theme |
| PA-30 | Notifications | B3 | devices join and leave live from B3 |
| BE-07 | 24-hour fuzzing | B2 | the frame parser and protobuf go on the network in B2 |
| DS-03 | Noto fonts | B5 | the apps ship translated |
| XC-01 | Translations | B5 | the apps ship translated; the docs follow in B9 |

## 17. The three uses — what delivers each

| Use | Engine | Agent and API | Screens | Proven by |
|---|---|---|---|---|
| Agents on your own model | EN-10, EN-15, EN-16, EN-21 | NO-30, NO-31, NO-41, NO-44, AP-02–AP-05 | PA-09, PA-11 | W13 |
| The family ring | EN-25, EN-28 | NO-13, NO-14, NO-47 | PA-07, AN-06, IO-07 | the B7 gate |
| Accuracy mode | EN-23 | NO-45 | PA-03, PA-12 | W12 |
