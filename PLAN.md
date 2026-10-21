# Omnarc — the plan (v4, October 22, 2026)

v4 replaces v3.2 ([docs/history/PLAN-v3.2.md](docs/history/PLAN-v3.2.md); section numbers in code comments, such as
"PLAN §5", refer to it). v3.2 planned ten stages of our own engine, ring, scheduler and network. v4 starts from
what was measured on our own devices and from what others have already built better.

## 0. Why the plan changed

On October 21, 2026, one ordinary Windows PC running plain llama.cpp generated 8.2 tokens/s with Llama 3.1 8B. The same
PC and a laptop running Omnarc's own ring generated 0.37 tokens/s with Qwen3-14B, after a 122-second wait for the
first token ([bench/reality](bench/reality/README.md)). Ten days of work had built the most difficult parts first:
our own engine patch, ring, live takeover, MILP scheduler and TLA+ models. None of it was measured against the
simplest baseline.

Five lessons decide everything below:

1. **Splitting a model that fits on one device only slows it down.** Our ring and Mesh LLM's public test both show
   it (68 → 21 → 12 tokens/s).
2. **Speed is physics.** Generation reads every active weight once per token, so its speed is about the bytes read
   per token divided by memory bandwidth. Adding devices adds capacity and concurrency, never single-answer speed.
   Only mixture-of-experts models make a large model fast on ordinary machines, because they read a small part
   of their weights per token.
3. **Formulas lie, measurements do not.** Our GPU formula promised 11 layers and 5 fitted. Our speed estimate was
   0.35 s per token and the result was 2.69 s.
4. **What others built is further along.** llama.cpp already fits a model to device memory (`-fit`), moves MoE
   experts to the CPU (`-ncmoe`), batches requests, caches prompts, serves an OpenAI API and a web UI, and ships
   binaries for every OS and GPU. Mesh LLM (Apache-2.0, v0.75, Rust, iroh) already splits models across macOS,
   Linux and Windows machines, fetches only each machine's GGUF fragments, connects over NAT through relays and
   forms private meshes with invite tokens.
5. **A user sees the product, not the architecture.** Nobody saw the errors: they lived in each machine's log.
   Installing on a new Windows PC took days of cross-compiling.

## 1. Rules

- **R-1 Take, do not rewrite.** For every part, the best existing open-source project is used as it is. Omnarc writes
  only the glue and what does not exist anywhere, and sends improvements upstream before keeping a fork.
- **R-2 No claim without a measurement.** Every number in a document, the README or the UI comes from a file in
  [bench/](bench/): hardware, versions, the command and the result. Targets are set from measurements, not before them.
- **R-3 The baseline.** Every feature is measured against plain llama.cpp on the same devices. A feature that is
  slower or less reliable is not shipped.
- **R-4 The golden path first.** Install, join, choose a model, ask. Nothing else is built until a newcomer gets
  through it without a terminal.
- **R-5 Errors are part of the product.** Every failure on any device shows up in the panel in plain words.

## 2. What Omnarc is

> **Your own computers run a model that none of them could run alone, and serve everyone at once — private,
> offline, with no cloud bill.**

What Omnarc promises: capacity (larger models than any one device holds), concurrency (more people served
together), privacy and offline work, and the speed it measured, shown before a model starts.

What is never promised:
- a faster single answer than the strongest device can give alone;
- speed comparable to a data centre;
- splitting a model that fits on one device;
- any number that has not been measured.

## 3. Build on, do not build

| Need | Taken from | What Omnarc adds |
|---|---|---|
| Inference on every OS and GPU | **llama.cpp** official releases: `llama-server` (OpenAI API, batching, prompt cache, `-fit`, `-ncmoe`, speculative decoding, web UI) | nothing — used as released |
| A model across several machines | **Mesh LLM** (candidate, verified in S0): layer stages, per-peer GGUF fragments, iroh QUIC with relays, invite-token meshes, OpenAI API, console | what it lacks after S0, as upstream contributions |
| Fallback if S0 rejects Mesh LLM | **llama.cpp RPC**, only inside an encrypted tunnel (it has no authentication of its own) | the tunnel and the placement |
| Models | Hugging Face downloads; peers share fragments over the mesh | — |
| Chat | the web UI of Mesh LLM or llama-server | — |
| Phones | the same web UI in the phone's browser on the local network | phones are clients in v4, not compute |
| Desktop app, installer, updates | the installer and packaging of the chosen runtime; Tauri 2 updater if a wrapper is needed | one installer per OS |

What Omnarc owns is what no project above does yet: an owner's budget and hours for each device, a panel a newcomer
understands, organization features and the measured answer to "will it fit, and how fast?". Each of these is first
proposed upstream.

### 3.1 What happens to our own code

Each part stays in git history. Nothing is deleted before S0 decides what replaces it.

| Part | v4 |
|---|---|
| Engine patch and libomnarc ABI 1–7, our ring, live takeover, KV mirroring | frozen; replaced by the runtime chosen in S0 |
| HiGHS MILP scheduler, profiler formulas | frozen; placement is the runtime's, sizes come from `-fit`, speed is measured |
| f8 activations, n-gram drafts, TLA+ models | frozen; llama.cpp's own speculative decoding is used |
| iroh networking, relay on our server, P2P model fetch | kept only where the chosen runtime does not already do it (Mesh LLM uses iroh too) |
| Owner budgets and hours (Q1, Q7) | kept: proposed to the chosen runtime; ours only if it is declined |
| Panel design system, living ring | kept for Omnarc's own surfaces |
| Android app | a client only, or replaced by the browser |

## 4. Stages

A stage is done when its gate is measured on real devices and written to `bench/`. The order is fixed: no stage
starts before the previous gate passes.

| Stage | Work | Gate |
|---|---|---|
| **S0 Verify** | Install Mesh LLM and plain llama.cpp on the devices at hand ([bench/reality](bench/reality/README.md)), from released binaries. Measure the same models on both. | Mesh LLM installs from a release on each OS without building; a private mesh forms across NAT; a model that fits runs whole, within 10 % of plain llama.cpp; a model that does not fit splits and its speed is recorded; gpt-oss-20b and Qwen3-30B-A3B split across two devices are recorded; a device that leaves is recorded. Output: `bench/s0/` and the choice of runtime. |
| **S1 Golden path** | One installer per OS, a QR invite, the panel: devices, what each offers, what runs where, every error in words. After every release, a smoke test on the real cluster asks a question and records time and tokens/s. | A newcomer installs on two computers and gets an answer in ≤ 10 minutes without a terminal, measured with someone who has not seen Omnarc. |
| **S2 Pool** | Every device that holds a model whole runs a copy; requests go to the least busy copy; batching on each device; owner budgets and hours enforced. | N people at once get answers, each at no less than half of single-user speed; the aggregate is measured on 2 and on 4 devices. |
| **S3 Large MoE** | Models that fit on no single device, split by the runtime, with the measured speed shown before they start. | A large MoE model (size chosen from S0) runs across ordinary computers, at the speed S0 measured, end to end with an agent (OpenCode or OpenClaw) on a fixed task set. |
| **S4 Organization** | Accounts and roles, API keys, quotas, schedules (the "night office"), an audit trail. | A team of 5 uses one cluster for a week. |

After S4, each further idea needs its own measured reason before it enters the plan: phones contributing compute,
browsers, training, a public network.

## 5. Who pays, and when

The positioning of v3.2 §14 stands: the compute belongs to the user, so it is not sold. What is sold is what
surrounds it, and revenue comes from organizations. The tiers attach to v4's stages, and none is sold before its
gate:

| Tier | What it adds | Ready at |
|---|---|---|
| Free | your own devices, nothing leaves them, offline | S1 |
| Home | several devices pooled | S2 |
| Organization | accounts, quotas, schedules, audit | S4 |
| Developer | API keys, model catalogue, usage | S4 |

## 6. How we work

- **One session works on the repository at a time.** It ends with a handoff note in memory.
- **The real cluster has a smoke test.** A tiny model is asked after every deploy. A failure stops the work.
- **Measurements first.** An experiment that takes an hour comes before code that takes a day.
- **Release builds** come from CI or from one scripted command, never from hand-made cross-compilation.

## 7. Open decisions for the owner

1. After S0: Mesh LLM as the base, or llama.cpp RPC with our own thin manager.
2. Contribute to the chosen runtime upstream, or keep a fork.
3. The server's role: a relay only, or a compute device with a larger budget.
4. Pushing to a private branch so CI builds releases.
5. Devices for S0. The laptop (5 GB) cannot hold a model that needs splitting, the server is a shared VPS on another
   continent, and the Windows PC has left. A split on one local network needs a second computer there.

[SPEC.md](SPEC.md) still lists v3.2's items by v3.2's stages. It is mapped to S0–S4 once S0 has chosen the runtime.
