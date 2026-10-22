# S0 — first measurements (October 22, 2026)

Partial results of stage S0 ([PLAN §4](../../PLAN.md)), measured on the devices at hand. Every number on the landing
page that is credited to S0 comes from this file. S0's gate is not passed yet: Mesh LLM is not measured, and
gpt-oss-20b and Qwen3-30B-A3B split across two devices are not recorded.

## Devices and software

| Device | Details |
|---|---|
| Laptop | Ryzen 5 7520U, 5.1 GB RAM (about 2.5 GB free), on the phone's Wi-Fi hotspot |
| Server | 4 vCPU EPYC VPS, 16 GB RAM (about 4 GB free), Docker with 2–3 GB limits, on another continent |
| Phone | armeabi-v7a (32-bit), 2.9 GB RAM (1.4 GB free); it provides the hotspot |
| Network | laptop ↔ phone ping 2.95 ms; laptop ↔ server 140–145 ms, 300–960 ms under load (bufferbloat) |

llama.cpp b11799, the official x86_64 release; on the phone, llama.cpp's `rpc-server` built for armeabi-v7a with
NDK r27c. Text: wikitext.

## 1. One device, the whole model (generation speed)

| Model | Device | Tokens/s |
|---|---|---|
| Granite 3B-A800M Q4_K_M (1.9 GB, 40 experts, 8 per token) | server, 2 cores | 32.8 |
| — | laptop, 4 cores | 42.0 |
| — | phone, 4 cores | 0.70 (the model is larger than its free memory) |
| GLM-4.7-Flash UD-TQ1_0 (8.3 GB, 31B, 64 experts, 4 per token) | server, 3 cores, 3 GB memory limit, the rest read from disk | 0.35 |

## 2. The cost of one remote expert step (llama.cpp RPC)

- **One machine** (loopback, 2 cores): 47.7 ms per token with no remote layer, 75.3 ms with the experts of 31 layers
  on the RPC server, so about **0.9 ms** per remote expert layer (a noisy measurement).
- **A phone over Wi-Fi**: about **28 ms** per remote expert layer.
- **The internet** (the server): about **500 ms** per step. One remote layer on the server added 369 ms per token over
  a link with a 220 ms round trip, and the round trip grows to 300–960 ms under load.
- llama.cpp RPC did not work with the 32-bit phone at first: lengths were sent as `size_t`, and the client reported a
  malformed response. Sending them as `uint64_t` fixed it.

## 3. Routing (our `moestat` tool and `analyze.py`)

| | Granite 3B (2048 tokens, 31 layers) | GLM-4.7-Flash (1024 tokens, 45 layers) |
|---|---|---|
| Share of calls taken by the hottest 10 % of experts | 27.8 % | 41.0 % |
| … by the hottest 25 % | 52.7 % | 66.7 % |
| Steps with no network wait when the core keeps the hottest 25 % | 1.0 % | 16.8 % |
| … the hottest 50 % | 19.8 % | 53.7 % |

With methods that change no output — hot experts learned from the prompt, and prefetching the next layer's eight
likeliest experts — GLM-4.7-Flash's sequential network waits fell from 45 to about 16 per token (14 to 18.5,
depending on the text).

## 4. Two and three devices, one answer

Granite 3B-A800M Q4_K_M, temperature 0, 24 tokens. Every tensor stays on the laptop except the experts of the given
layers.

| Placement | ms per token | Tokens/s |
|---|---|---|
| laptop alone | 67 | 14.9 |
| phone holds the experts of layers 0–7 | 249 | 4.0 |
| server holds the experts of 1 layer (over the internet) | 436 | 2.3 |
| phone layers 0–7, server layer 8 | 2046–2115 | 0.47–0.49 |

With the phone, and with the server, the text is byte for byte the server's own text (md5 prefix `2d1af767`); the
laptop alone differs because it uses its own repacked kernels. While its owner watched video, the phone ran 4 to 6
times slower.

The laptop's command (`--rpc` trimmed to the phone's endpoint; the phone runs `rpc-server` on port 50052):

```
llama-completion -m granite-3b-a800m-Q4_K_M.gguf -t 4 -ngl 0 --rpc 192.168.43.145:50052 \
  -ot "blk\.(0|1|2|3|4|5|6|7)\.ffn_.*_exps\.weight=RPC0[192.168.43.145:50052]" \
  -n 24 -no-cnv --temp 0 -p "The history of the city begins in the early medieval period, when"
```

```
common_perf_print: prompt eval time =    4575.98 ms /    15 tokens (  305.07 ms per token,     3.28 tokens per second)
common_perf_print:        eval time =    5734.72 ms /    23 runs   (  249.34 ms per token,     4.01 tokens per second)
```
