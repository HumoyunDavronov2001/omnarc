// omnarc-gate — the accuracy gate (PLAN.md §4): a model split into windows must give the same answers as the same
// model run whole on one device. Every chunk of the input runs through the whole model and through the windows in
// turn; the hidden state between windows is rounded to the wire format, as it would travel between devices.
//
//   omnarc-gate --model M.gguf --split 0:40,40:81 [--text FILE | --random-tokens N]
//
// Layers are numbered as in the engine: the output head of an 80-layer model is layer 80.
//
// Reports perplexity of both runs, the KL divergence of the split run's next-token distribution from the whole
// run's, and how often both pick the same token. Exit code 0 means the gate passed.
//
// On a small machine a large model is slow, so the whole model's logits can be computed once and kept in a file
// (--reference-out, with --split none to skip the comparison) and every split compared with them later
// (--reference), as llama.cpp's perplexity tool does for its KL divergence.
#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <random>
#include <sstream>
#include <string>
#include <vector>

#include "omnarc/omnarc.h"
#include "ggml.h"

namespace {

struct window {
    uint32_t begin = 0;
    uint32_t end = 0;
    om_backend backend = OM_BACKEND_AUTO;
};

struct options {
    std::string model;
    std::string split;
    std::string text_file;
    uint32_t random_tokens = 0;
    uint32_t ctx = 512;
    uint32_t chunks = 0;  // 0 = all
    std::string wire = "f16";
    om_backend backend = OM_BACKEND_AUTO;
    bool exact = false;
    double max_kl = 0.01;
    double max_ppl_diff = 0.5;  // percent
    std::string reference;      // read the whole model's logits from this file
    std::string reference_out;  // write them to this file
};

[[noreturn]] void usage(int code) {
    std::fprintf(code == 0 ? stdout : stderr,
                 "usage: omnarc-gate --model M.gguf [--split A:B,B:C[@backend],...|even:K]\n"
                 "                   [--text FILE | --random-tokens N] [--ctx N] [--chunks N] [--wire f32|f16|bf16]\n"
                 "                   [--backend NAME] [--exact] [--reference FILE | --reference-out FILE]\n"
                 "                   [--max-kl X] [--max-ppl-diff PERCENT]\n"
                 "backends: auto cpu cuda hip vulkan metal opencl sycl\n");
    std::exit(code);
}

bool parse_backend(const std::string &name, om_backend &out) {
    static const std::pair<const char *, om_backend> names[] = {
        {"auto", OM_BACKEND_AUTO},     {"cpu", OM_BACKEND_CPU},       {"cuda", OM_BACKEND_CUDA},
        {"hip", OM_BACKEND_HIP},       {"vulkan", OM_BACKEND_VULKAN}, {"metal", OM_BACKEND_METAL},
        {"opencl", OM_BACKEND_OPENCL}, {"sycl", OM_BACKEND_SYCL},
    };
    for (const auto &[n, b] : names) {
        if (name == n) {
            out = b;
            return true;
        }
    }
    return false;
}

options parse_args(int argc, char **argv) {
    options o;
    for (int i = 1; i < argc; ++i) {
        const std::string arg = argv[i];
        auto value = [&]() -> std::string {
            if (i + 1 >= argc) usage(2);
            return argv[++i];
        };
        if (arg == "--model") o.model = value();
        else if (arg == "--split") o.split = value();
        else if (arg == "--text") o.text_file = value();
        else if (arg == "--random-tokens") o.random_tokens = static_cast<uint32_t>(std::stoul(value()));
        else if (arg == "--ctx") o.ctx = static_cast<uint32_t>(std::stoul(value()));
        else if (arg == "--chunks") o.chunks = static_cast<uint32_t>(std::stoul(value()));
        else if (arg == "--wire") o.wire = value();
        else if (arg == "--backend") {
            if (!parse_backend(value(), o.backend)) usage(2);
        } else if (arg == "--exact") o.exact = true;
        else if (arg == "--max-kl") o.max_kl = std::stod(value());
        else if (arg == "--max-ppl-diff") o.max_ppl_diff = std::stod(value());
        else if (arg == "--reference") o.reference = value();
        else if (arg == "--reference-out") o.reference_out = value();
        else if (arg == "-h" || arg == "--help") usage(0);
        else usage(2);
    }
    const bool has_input = !o.text_file.empty() || o.random_tokens > 0;
    if (o.model.empty() || o.ctx < 2 || (o.text_file.empty() == (o.random_tokens == 0) && o.reference.empty()) ||
        (!o.reference.empty() && (has_input || !o.reference_out.empty())) ||
        (o.split == "none" && o.reference_out.empty())) {
        usage(2);
    }
    if (o.wire != "f32" && o.wire != "f16" && o.wire != "bf16") usage(2);
    return o;
}

// "0:40,40:81@cpu" -> windows; "even:K" -> K equal windows of the layers and the output head alone, the way the
// ring keeps the head on the device with the first layers; empty = two halves. Layer n_layer is the output head.
std::vector<window> parse_split(const std::string &spec, uint32_t n_layer, om_backend backend) {
    std::vector<window> windows;
    if (spec == "none") return windows;  // only the reference is computed
    if (spec.empty()) {
        windows.push_back({0, n_layer / 2, backend});
        windows.push_back({n_layer / 2, n_layer + 1, backend});
        return windows;
    }
    if (spec.rfind("even:", 0) == 0) {
        const auto k = static_cast<uint32_t>(std::stoul(spec.substr(5)));
        if (k == 0 || k > n_layer) usage(2);
        for (uint32_t i = 0; i < k; ++i) windows.push_back({n_layer * i / k, n_layer * (i + 1) / k, backend});
        windows.push_back({n_layer, n_layer + 1, backend});
        return windows;
    }
    std::stringstream ss(spec);
    std::string item;
    while (std::getline(ss, item, ',')) {
        window w;
        w.backend = backend;
        const auto at = item.find('@');
        if (at != std::string::npos) {
            if (!parse_backend(item.substr(at + 1), w.backend)) usage(2);
            item = item.substr(0, at);
        }
        const auto colon = item.find(':');
        if (colon == std::string::npos) usage(2);
        w.begin = static_cast<uint32_t>(std::stoul(item.substr(0, colon)));
        w.end = static_cast<uint32_t>(std::stoul(item.substr(colon + 1)));
        windows.push_back(w);
    }
    return windows;
}

const char *backend_name(om_backend backend) {
    switch (backend) {
        case OM_BACKEND_AUTO: return "auto";
        case OM_BACKEND_CPU: return "cpu";
        case OM_BACKEND_CUDA: return "cuda";
        case OM_BACKEND_HIP: return "hip";
        case OM_BACKEND_VULKAN: return "vulkan";
        case OM_BACKEND_METAL: return "metal";
        case OM_BACKEND_OPENCL: return "opencl";
        case OM_BACKEND_HEXAGON: return "hexagon";
        case OM_BACKEND_SYCL: return "sycl";
        case OM_BACKEND_CANN: return "cann";
        case OM_BACKEND_WEBGPU: return "webgpu";
    }
    return "unknown";
}

om_shard *open_shard(const std::string &model, uint32_t begin, uint32_t end, om_backend backend, uint32_t ctx) {
    const om_shard_params params = {model.c_str(), begin, end, backend, ctx, OM_KV_F16};
    om_shard *shard = nullptr;
    const om_status status = om_shard_open(&params, &shard);
    if (status != OM_OK) {
        std::fprintf(stderr, "omnarc-gate: cannot open layers [%u, %u): %s: %s\n", begin, end,
                     om_status_string(status), om_last_error());
        std::exit(1);
    }
    return shard;
}

om_backend shard_backend(const om_shard *shard) {
    om_shard_info info{};
    om_shard_get_info(shard, &info);
    return info.backend;
}

void check(om_status status, const char *what) {
    if (status != OM_OK) {
        std::fprintf(stderr, "omnarc-gate: %s: %s: %s\n", what, om_status_string(status), om_last_error());
        std::exit(1);
    }
}

// The hidden state as it arrives after travelling in the wire format.
void round_to_wire(std::vector<float> &v, const std::string &wire) {
    if (wire == "f16") {
        for (float &x : v) x = ggml_fp16_to_fp32(ggml_fp32_to_fp16(x));
    } else if (wire == "bf16") {
        for (float &x : v) x = ggml_bf16_to_fp32(ggml_fp32_to_bf16(x));
    }
}

void log_softmax(const float *logits, size_t n, std::vector<double> &out) {
    out.resize(n);
    const double max = *std::max_element(logits, logits + n);
    double sum = 0.0;
    for (size_t i = 0; i < n; ++i) sum += std::exp(static_cast<double>(logits[i]) - max);
    const double log_sum = std::log(sum) + max;
    for (size_t i = 0; i < n; ++i) out[i] = static_cast<double>(logits[i]) - log_sum;
}

}  // namespace

int main(int argc, char **argv) {
    const options opt = parse_args(argc, argv);

    om_model_info info{};
    check(om_model_info_read(opt.model.c_str(), &info), "reading the model");
    const std::vector<window> windows = parse_split(opt.split, info.layer_count, opt.backend);
    // layer layer_count is the output head, so the split covers [0, layer_count + 1)
    const uint32_t end = info.layer_count + 1;
    if (!windows.empty() && (windows.size() < 2 || windows.front().begin != 0 || windows.back().end != end)) {
        std::fprintf(stderr, "omnarc-gate: the split must cover layers [0, %u) with at least two windows\n", end);
        return 2;
    }
    for (size_t i = 1; i < windows.size(); ++i) {
        if (windows[i].begin != windows[i - 1].end) {
            std::fprintf(stderr, "omnarc-gate: window %zu does not start where window %zu ends\n", i, i - 1);
            return 2;
        }
    }

    std::printf("model   %s: %s, %u layers, hidden %u, vocab %u\n", opt.model.c_str(), info.architecture,
                info.layer_count, info.hidden_dim, info.vocab_size);
    if (!windows.empty()) {
        std::printf("split  ");
        for (const auto &w : windows) std::printf(" [%u, %u)", w.begin, w.end);
        std::printf(", hidden state on the wire as %s\n", opt.wire.c_str());
    }

    // the reference: the whole model on one device, run now or read from a file that --reference-out wrote
    const size_t vocab = info.vocab_size;
    std::vector<int32_t> tokens;
    uint32_t file_ctx = 0;
    uint32_t file_chunks = 0;
    std::ifstream ref_in;
    if (!opt.reference.empty()) {
        ref_in.open(opt.reference, std::ios::binary);
        if (!ref_in) {
            std::fprintf(stderr, "omnarc-gate: cannot read %s\n", opt.reference.c_str());
            return 1;
        }
        char magic[4] = {};
        uint32_t head[4] = {};  // vocab, ctx, chunks, tokens
        ref_in.read(magic, 4);
        ref_in.read(reinterpret_cast<char *>(head), sizeof(head));
        if (!ref_in || std::memcmp(magic, "OMR1", 4) != 0 || head[0] != vocab) {
            std::fprintf(stderr, "omnarc-gate: %s is not a reference for this model\n", opt.reference.c_str());
            return 1;
        }
        file_ctx = head[1];
        file_chunks = head[2];
        tokens.resize(head[3]);
        ref_in.read(reinterpret_cast<char *>(tokens.data()), static_cast<std::streamsize>(tokens.size() * 4));
    }

    om_shard *whole = opt.reference.empty() ? open_shard(opt.model, 0, end, opt.backend, opt.ctx) : nullptr;
    std::vector<om_shard *> shards;
    for (const auto &w : windows) shards.push_back(open_shard(opt.model, w.begin, w.end, w.backend, opt.ctx));
    std::printf("backend whole: %s, windows:", whole != nullptr ? backend_name(shard_backend(whole)) : "from file");
    for (const om_shard *shard : shards) std::printf(" %s", backend_name(shard_backend(shard)));
    std::printf("\n");

    if (!opt.reference.empty()) {
        // the tokens came with the reference
    } else if (!opt.text_file.empty()) {
        std::ifstream file(opt.text_file, std::ios::binary);
        if (!file) {
            std::fprintf(stderr, "omnarc-gate: cannot read %s\n", opt.text_file.c_str());
            return 1;
        }
        const std::string text((std::istreambuf_iterator<char>(file)), std::istreambuf_iterator<char>());
        size_t count = 0;
        om_shard_tokenize(whole, text.data(), text.size(), 1, nullptr, 0, &count);
        tokens.resize(count);
        check(om_shard_tokenize(whole, text.data(), text.size(), 1, tokens.data(), tokens.size(), &count),
              "tokenizing");
    } else {
        std::mt19937 rng(42);
        std::uniform_int_distribution<int32_t> dist(0, static_cast<int32_t>(info.vocab_size) - 1);
        tokens.resize(opt.random_tokens);
        for (auto &t : tokens) t = dist(rng);
    }

    const uint32_t ctx =
        file_ctx > 0 ? file_ctx : std::min<uint32_t>(opt.ctx, static_cast<uint32_t>(tokens.size()));
    uint32_t n_chunks = file_ctx > 0 ? file_chunks : static_cast<uint32_t>(tokens.size()) / ctx;
    if (opt.chunks > 0) n_chunks = std::min(n_chunks, opt.chunks);
    if (n_chunks == 0 || ctx > opt.ctx) {
        std::fprintf(stderr, "omnarc-gate: not enough tokens, or --ctx is smaller than the reference's %u\n", ctx);
        return 1;
    }

    std::ofstream ref_out;
    if (!opt.reference_out.empty()) {
        ref_out.open(opt.reference_out, std::ios::binary);
        if (!ref_out) {
            std::fprintf(stderr, "omnarc-gate: cannot write %s\n", opt.reference_out.c_str());
            return 1;
        }
        const uint32_t head[4] = {static_cast<uint32_t>(vocab), ctx, n_chunks, static_cast<uint32_t>(tokens.size())};
        ref_out.write("OMR1", 4);
        ref_out.write(reinterpret_cast<const char *>(head), sizeof(head));
        ref_out.write(reinterpret_cast<const char *>(tokens.data()), static_cast<std::streamsize>(tokens.size() * 4));
    }

    const size_t hidden = info.hidden_dim;
    std::vector<float> ref_logits(ctx * vocab);
    std::vector<float> split_logits(ctx * vocab);
    std::vector<float> state(ctx * hidden);
    std::vector<double> ref_lp;
    std::vector<double> split_lp;

    double nll_ref = 0.0, nll_split = 0.0, kl_sum = 0.0, kl_max = 0.0, logit_diff_max = 0.0;
    uint64_t evaluated = 0, same_top = 0;

    for (uint32_t c = 0; c < n_chunks; ++c) {
        const int32_t *chunk = tokens.data() + static_cast<size_t>(c) * ctx;
        const uint64_t request = c + 1;

        const auto logits_bytes = static_cast<std::streamsize>(ref_logits.size() * sizeof(float));
        if (whole != nullptr) {
            const om_batch whole_batch = {request, 0, ctx, chunk, nullptr, 1};
            check(om_shard_run(whole, &whole_batch, ref_logits.data(), ref_logits.size()), "running the whole model");
            check(om_shard_drop(whole, request), "dropping a request");
        } else if (!ref_in.read(reinterpret_cast<char *>(ref_logits.data()), logits_bytes)) {
            std::fprintf(stderr, "omnarc-gate: %s ends early\n", opt.reference.c_str());
            return 1;
        }
        if (ref_out.is_open() && !ref_out.write(reinterpret_cast<const char *>(ref_logits.data()), logits_bytes)) {
            std::fprintf(stderr, "omnarc-gate: cannot write %s\n", opt.reference_out.c_str());
            return 1;
        }
        if (shards.empty()) {
            std::fprintf(stderr, "\rchunk %u/%u", c + 1, n_chunks);
            continue;
        }

        for (size_t s = 0; s < shards.size(); ++s) {
            const bool first = s == 0;
            const bool last = s + 1 == shards.size();
            const om_batch batch = {request, 0, ctx, first ? chunk : nullptr, first ? nullptr : state.data(), 1};
            if (last) {
                check(om_shard_run(shards[s], &batch, split_logits.data(), split_logits.size()), "running a window");
            } else {
                std::vector<float> next(ctx * hidden);
                check(om_shard_run(shards[s], &batch, next.data(), next.size()), "running a window");
                round_to_wire(next, opt.wire);
                state.swap(next);
            }
        }

        // every position predicts the next token of the chunk
        for (uint32_t t = 0; t + 1 < ctx; ++t) {
            const float *a = ref_logits.data() + t * vocab;
            const float *b = split_logits.data() + t * vocab;
            log_softmax(a, vocab, ref_lp);
            log_softmax(b, vocab, split_lp);

            double kl = 0.0;
            size_t top_a = 0, top_b = 0;
            for (size_t v = 0; v < vocab; ++v) {
                kl += std::exp(ref_lp[v]) * (ref_lp[v] - split_lp[v]);
                logit_diff_max = std::max(logit_diff_max, std::fabs(static_cast<double>(a[v]) - b[v]));
                if (a[v] > a[top_a]) top_a = v;
                if (b[v] > b[top_b]) top_b = v;
            }
            kl = std::max(kl, 0.0);
            kl_sum += kl;
            kl_max = std::max(kl_max, kl);
            same_top += top_a == top_b ? 1 : 0;

            const auto next = static_cast<size_t>(chunk[t + 1]);
            nll_ref -= ref_lp[next];
            nll_split -= split_lp[next];
            evaluated++;
        }

        for (om_shard *shard : shards) check(om_shard_drop(shard, request), "dropping a request");
        std::fprintf(stderr, "\rchunk %u/%u", c + 1, n_chunks);
    }
    std::fprintf(stderr, "\n");
    if (shards.empty()) {
        std::printf("reference %s: %u chunks of %u tokens\n", opt.reference_out.c_str(), n_chunks, ctx);
        om_shard_close(whole);
        return 0;
    }

    const double ppl_ref = std::exp(nll_ref / static_cast<double>(evaluated));
    const double ppl_split = std::exp(nll_split / static_cast<double>(evaluated));
    const double ppl_diff = (ppl_split / ppl_ref - 1.0) * 100.0;
    const double kl_mean = kl_sum / static_cast<double>(evaluated);
    const double top = 100.0 * static_cast<double>(same_top) / static_cast<double>(evaluated);

    std::printf("tokens  %llu in %u chunks of %u\n", static_cast<unsigned long long>(evaluated), n_chunks, ctx);
    std::printf("ppl     whole %.4f, split %.4f, difference %+.4f%%\n", ppl_ref, ppl_split, ppl_diff);
    std::printf("kl      mean %.6f, max %.6f\n", kl_mean, kl_max);
    std::printf("top-1   %.2f%% the same, max |logit difference| %.6f\n", top, logit_diff_max);

    bool pass = kl_mean <= opt.max_kl && std::fabs(ppl_diff) <= opt.max_ppl_diff;
    if (opt.exact) pass = pass && same_top == evaluated && kl_max <= 1e-6;
    std::printf("gate    %s (mean KL <= %g, |ppl difference| <= %g%%%s)\n", pass ? "PASS" : "FAIL", opt.max_kl,
                opt.max_ppl_diff, opt.exact ? ", exact: same top-1 everywhere and max KL <= 1e-6" : "");

    for (om_shard *shard : shards) om_shard_close(shard);
    om_shard_close(whole);
    return pass ? 0 : 1;
}
