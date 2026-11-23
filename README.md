<div align="center">

<img src="design/assets/living-ring.webp" width="320" alt="The Omnarc ring: every arc is a device, gold comets are words travelling through them">

# Omnarc

**A private AI cluster from the devices you already own.**

Omnarc joins your phones, laptops and computers into one AI that runs an open model across all of them —
on your own network, with nothing sent out, and free at home.

**[Download](https://omnarc.uz/download/)** · [How it works](https://omnarc.uz/how-it-works/) · [Benchmarks](https://omnarc.uz/benchmarks/) · [Docs](https://omnarc.uz/docs/getting-started/what-omnarc-is/) · [FAQ](https://omnarc.uz/docs/getting-started/faq/) · [O‘zbekcha](https://omnarc.uz/uz/)

</div>

> This repository holds Omnarc's **early open versions (to October 2026)**. The apps on
> [omnarc.uz](https://omnarc.uz/download/) are free, and their source is not public. Issues and questions are welcome here.
> Omnarc (omnarc.uz) is not affiliated with Omnara.

## What it does

- **One model across many devices.** Each device holds a block of the model's layers; for every word a small state
  travels around them — a ring. Together they run a model none of them could hold alone.
- **Mixture-of-experts models** (Qwen3 30B-A3B, Qwen3 Coder 30B-A3B, GLM-4.7 Flash, Qwen3 235B-A22B): large models
  of which only a small part works for each word, which is what lets laptops and phones keep up.
- **Windows, Linux and Android.** A graphics card is used through Vulkan (NVIDIA, AMD, Intel); without one, the processor.
  A phone helps only while it charges, on Wi-Fi, cool and unused.
- **Private by construction.** Questions and answers stay on your devices; it works with the internet off; a device
  joins only after you confirm the same four symbols on both screens; every update is signed.
- **The API your code already speaks.** Every device serves an OpenAI-, Anthropic- and Ollama-shaped API on your own
  network — no key from anyone, no charge.

## Measured, not promised

On deliberately modest hardware — a budget laptop (Ryzen 5 7520U, 5.1 GB), an old 32-bit phone (MT6739, 2.9 GB) and a
server on another continent — running GLM-4.7-Flash (8.3 GB, 31B parameters), which none of them holds alone:

| | tokens/s, 8 prompts at once |
|---|---|
| The server alone (the only device that can) | 0.92 |
| Omnarc ring: laptop + phone + server | 4.17 (4.5×) |

- 13× faster than llama.cpp RPC on the same layers and devices
- −0.0015 % perplexity change when the model is split (16-bit between devices)
- 6 % slowdown of the owner's own work while the ring runs

More devices add room for larger models; they do not make a small model faster than one good GPU.
Every run, with its devices, version and command: [omnarc.uz/benchmarks](https://omnarc.uz/benchmarks/).

## Editions

| | |
|---|---|
| **Omnarc** | your own devices — free, no account |
| **Omnarc for organizations** | on the organization's own computers and network, one signed license file, no account; data never leaves the building |
| **Developer API** | OpenAI-, Anthropic- and Ollama-compatible, on your own cluster |

## Links

[Website](https://omnarc.uz) · [Docs](https://omnarc.uz/docs/getting-started/what-omnarc-is/) · [Trust center](https://omnarc.uz/trust/) · [Changelog](https://omnarc.uz/changelog/) · [Security](https://omnarc.uz/docs/security/report-a-vulnerability/)

Made in Uzbekistan.
