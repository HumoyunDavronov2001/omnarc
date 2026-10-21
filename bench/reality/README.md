# What our devices really do (October 2026)

Every number Omnarc states comes from a measurement in this folder: the hardware, the software and its version, the
command, the result. A number that is not here is not stated anywhere else either.

## The devices

| Device | Processor | Memory | Accelerator | Measured memory bandwidth |
|---|---|---|---|---|
| Laptop | AMD Ryzen 5 7520U | 5.1 GB | — | 28.7 GB/s (CPU) |
| Windows PC | Intel Core i5-8300H | 11.8 GB | NVIDIA GTX 1050, 4 GB | 27.8 GB/s (CPU), 87 GB/s (GPU) |
| Server (shared VPS) | 4 vCPU of an AMD EPYC 9354P | 16 GB, about 4 GB free | — | 65 GB/s (2 cores) |
| Phone | armeabi-v7a | — | — | 3.4 GB/s |

Bandwidths are from the Omnarc profiler (`omnarc-node info`). The laptop and the Windows PC were on one home Wi-Fi
network: 4–17 ms round trip, 30–43 Mbit/s. The server is on another continent: 155–378 ms over a phone hotspot.

## One device, plain llama.cpp

llama.cpp b11799 (official Windows Vulkan release, 2026-10-21), `llama-bench -p 512 -n 64 -r 2`, Windows PC,
October 21, 2026.

| Model | Where | Prompt (pp512) | Generation (tg64) |
|---|---|---|---|
| Llama 3.1 8B Instruct Q4_K_M (4.6 GiB) | CPU, 4 threads | 63.3 tokens/s | 4.45 tokens/s |
| Llama 3.1 8B Instruct Q4_K_M | GTX 1050, layers placed by `-fitt 384` | 68.4 tokens/s | 8.22 tokens/s |

The Qwen3-14B run did not finish: the Windows PC left the network during it.

## The same devices, Omnarc's own ring

| Date | Model | Devices | Result |
|---|---|---|---|
| 2026-10-20 | Llama 3.1 8B Q4_K_M | laptop + Windows PC (layers 16–33 on the GTX 1050) | 0.76 tokens/s |
| 2026-10-21 | Qwen3-14B Q4_K_M, KV q8_0, context 2048 | Windows PC (layers 0–36, 5 on the GTX 1050) + laptop (37–39) | first token after 122.6 s, then 2.69 s per token (0.37 tokens/s) |

On 2026-10-21 the plan predicted 0.35 s per token and 11 layers on the GPU. Five fitted. The other layers no
longer fitted in the Windows PC's memory and were read from disk on every token.

## What others measured, recorded October 12, 2026

- Mesh LLM, GLM-4.7-Flash (17 GB) on an M4 Max and a Mac mini over Wi-Fi: 68 tokens/s on one machine, 21 and 12 tokens/s
  split. The model fitted on one machine (wavect.io, 2026-07-18).
- colibri, GLM-5.2 (744B MoE) streamed from disk: 0.05–0.1 tokens/s on a 25 GB laptop, 1.83 tokens/s with 128 GB.

## What follows from it

1. One ordinary PC with plain llama.cpp was 10–20 times faster than our two-device ring. Splitting a model that fits
   on one device only slows it down.
2. Generation reads every active weight once per token. Its speed is about the bytes read per token divided by the
   memory bandwidth, wherever the layers are: splitting adds hops, it never removes bytes. A dense 70B model reads
   about 42 GB per token, an MoE model like gpt-oss-120b about 3 GB.
3. Formulas for memory and speed were wrong by factors of 2 to 7. Fitting is measured (llama.cpp `-fit`), speed is
   measured, and only measured values are shown.
4. Over the internet a split pays its round trip on every token: 155–378 ms per hop, before any compute.
