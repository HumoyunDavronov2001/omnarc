// libomnarc — shards on llama.cpp.
#include "omnarc/omnarc.h"

#include <algorithm>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <filesystem>
#include <memory>
#include <mutex>
#include <string>
#include <unordered_map>
#include <vector>

#include "ggml-backend.h"
#include "gguf.h"
#include "llama.h"
#include "system.h"

namespace {

constexpr const char *kVersion = "0.1.0";

// Requests a shard serves at once. The KV cache is unified, so requests share max_tokens cells and an idle slot
// costs nothing.
constexpr uint32_t kMaxRequests = 64;

// Tokens computed in one graph; longer batches run in several steps.
constexpr uint32_t kMaxUbatch = 512;

// Architectures whose layers can be split into windows; llama.cpp checks the same list
// (llm_arch_supports_layer_window in engine/patches).
constexpr const char *kSplittable[] = {"llama", "qwen3"};

thread_local std::string g_last_error;
thread_local ggml_log_level g_last_level = GGML_LOG_LEVEL_INFO;

om_status fail(om_status status, std::string message) {
    g_last_error = std::move(message);
    return status;
}

bool verbose() {
    static const bool value = [] {
        const char *env = std::getenv("OMNARC_VERBOSE");
        return env != nullptr && env[0] != '\0' && std::strcmp(env, "0") != 0;
    }();
    return value;
}

// llama.cpp errors become the thread's last error; warnings and errors also go to stderr, and everything does with
// OMNARC_VERBOSE=1.
void log_callback(ggml_log_level level, const char *text, void * /*user_data*/) {
    if (level == GGML_LOG_LEVEL_CONT) {
        level = g_last_level;
        if (level == GGML_LOG_LEVEL_ERROR) g_last_error += text;
    } else {
        g_last_level = level;
        if (level == GGML_LOG_LEVEL_ERROR) g_last_error = text;
    }
    while (!g_last_error.empty() && (g_last_error.back() == '\n' || g_last_error.back() == ' ')) {
        g_last_error.pop_back();
    }
    if (verbose() || level >= GGML_LOG_LEVEL_WARN) std::fputs(text, stderr);
}

void init_once() {
    static std::once_flag flag;
    std::call_once(flag, [] {
        llama_log_set(log_callback, nullptr);
        llama_backend_init();
    });
}

bool splittable(const std::string &arch) {
    return std::any_of(std::begin(kSplittable), std::end(kSplittable), [&](const char *a) { return arch == a; });
}

// From the names in the ggml backend registry.
om_backend backend_from_name(const std::string &name) {
    if (name == "CPU") return OM_BACKEND_CPU;
    if (name == "CUDA") return OM_BACKEND_CUDA;
    if (name == "ROCm" || name == "HIP") return OM_BACKEND_HIP;
    if (name == "Vulkan") return OM_BACKEND_VULKAN;
    if (name == "Metal" || name == "MTL") return OM_BACKEND_METAL;
    if (name == "OpenCL") return OM_BACKEND_OPENCL;
    if (name == "HTP" || name == "Hexagon") return OM_BACKEND_HEXAGON;
    if (name == "SYCL") return OM_BACKEND_SYCL;
    if (name == "CANN") return OM_BACKEND_CANN;
    if (name == "WebGPU") return OM_BACKEND_WEBGPU;
    return OM_BACKEND_AUTO;
}

om_backend device_backend(ggml_backend_dev_t dev) {
    return backend_from_name(ggml_backend_reg_name(ggml_backend_dev_backend_reg(dev)));
}

bool is_gpu(ggml_backend_dev_t dev) {
    const auto type = ggml_backend_dev_type(dev);
    return type == GGML_BACKEND_DEVICE_TYPE_GPU || type == GGML_BACKEND_DEVICE_TYPE_IGPU;
}

struct gguf_deleter {
    void operator()(gguf_context *ctx) const { gguf_free(ctx); }
};
using gguf_ptr = std::unique_ptr<gguf_context, gguf_deleter>;

gguf_ptr gguf_open(const std::string &path) {
    gguf_init_params params = {/*no_alloc =*/true, /*ctx =*/nullptr};
    return gguf_ptr(gguf_init_from_file(path.c_str(), params));
}

bool gguf_u32(const gguf_context *ctx, const std::string &key, uint32_t &out) {
    const int64_t id = gguf_find_key(ctx, key.c_str());
    if (id < 0) return false;
    switch (gguf_get_kv_type(ctx, id)) {
        case GGUF_TYPE_UINT32: out = gguf_get_val_u32(ctx, id); return true;
        case GGUF_TYPE_INT32: out = static_cast<uint32_t>(gguf_get_val_i32(ctx, id)); return true;
        case GGUF_TYPE_UINT64: out = static_cast<uint32_t>(gguf_get_val_u64(ctx, id)); return true;
        case GGUF_TYPE_INT64: out = static_cast<uint32_t>(gguf_get_val_i64(ctx, id)); return true;
        case GGUF_TYPE_UINT16: out = gguf_get_val_u16(ctx, id); return true;
        default: return false;
    }
}

// The files of a model: one, or every part of a split GGUF (model-00001-of-00003.gguf, ...).
std::vector<std::string> model_files(const std::string &path, const gguf_context *first) {
    uint32_t count = 1;
    if (!gguf_u32(first, "split.count", count) || count <= 1) return {path};
    char prefix[4096];
    if (llama_split_prefix(prefix, sizeof(prefix), path.c_str(), 0, static_cast<int>(count)) <= 0) return {path};
    std::vector<std::string> files;
    for (uint32_t i = 0; i < count; ++i) {
        char split[4096];
        llama_split_path(split, sizeof(split), prefix, static_cast<int>(i), static_cast<int>(count));
        files.emplace_back(split);
    }
    return files;
}

om_status read_info(const char *model_path, om_model_info &info) {
    std::error_code ec;
    if (!std::filesystem::is_regular_file(model_path, ec)) {
        return fail(OM_ERR_IO, std::string("model file not found: ") + model_path);
    }
    const gguf_ptr ctx = gguf_open(model_path);
    if (!ctx) return fail(OM_ERR_INVALID_ARGUMENT, std::string("not a GGUF model: ") + model_path);

    info = om_model_info{};
    const int64_t arch_id = gguf_find_key(ctx.get(), "general.architecture");
    if (arch_id < 0 || gguf_get_kv_type(ctx.get(), arch_id) != GGUF_TYPE_STRING) {
        return fail(OM_ERR_INVALID_ARGUMENT, "the model has no general.architecture");
    }
    const std::string arch = gguf_get_val_str(ctx.get(), arch_id);
    std::snprintf(info.architecture, sizeof(info.architecture), "%s", arch.c_str());

    if (!gguf_u32(ctx.get(), arch + ".block_count", info.layer_count) ||
        !gguf_u32(ctx.get(), arch + ".embedding_length", info.hidden_dim)) {
        return fail(OM_ERR_INVALID_ARGUMENT, "the model has no block_count or embedding_length");
    }
    gguf_u32(ctx.get(), arch + ".context_length", info.context_length);
    if (!gguf_u32(ctx.get(), arch + ".vocab_size", info.vocab_size)) {
        const int64_t tokens = gguf_find_key(ctx.get(), "tokenizer.ggml.tokens");
        if (tokens >= 0) info.vocab_size = static_cast<uint32_t>(gguf_get_arr_n(ctx.get(), tokens));
    }
    for (const auto &file : model_files(model_path, ctx.get())) {
        info.file_size += std::filesystem::file_size(file, ec);
    }
    info.splittable = splittable(arch) ? 1 : 0;
    return OM_OK;
}

// Whether a tensor belongs to the window [begin, end) of a model with n_layer layers: layer tensors by their number,
// the token embeddings with the window that starts at layer 0, the rest with the output head (layer n_layer).
// Models without an output matrix use the token embeddings as the output head too (tied embeddings).
bool tensor_in_window(const char *name, uint32_t begin, uint32_t end, uint32_t n_layer, bool tied_embeddings) {
    if (std::strncmp(name, "blk.", 4) == 0) {
        const auto layer = static_cast<uint32_t>(std::strtoul(name + 4, nullptr, 10));
        return layer >= begin && layer < end;
    }
    if (std::strncmp(name, "token_embd.", 11) == 0) return begin == 0 || (tied_embeddings && end > n_layer);
    return end > n_layer;  // output, output_norm and the rest of the head
}

std::vector<std::unique_ptr<omnarc::file_prefetcher>> window_prefetchers(const std::string &path, uint32_t begin,
                                                                         uint32_t end, uint32_t n_layer) {
    std::vector<std::unique_ptr<omnarc::file_prefetcher>> result;
    const gguf_ptr first = gguf_open(path);
    if (!first) return result;
    const std::vector<std::string> files = model_files(path, first.get());
    bool tied_embeddings = true;
    for (const auto &file : files) {
        const gguf_ptr ctx = gguf_open(file);
        if (ctx && gguf_find_tensor(ctx.get(), "output.weight") >= 0) tied_embeddings = false;
    }
    for (const auto &file : files) {
        const gguf_ptr ctx = gguf_open(file);
        if (!ctx) continue;
        std::vector<omnarc::file_prefetcher::range> ranges;
        const uint64_t data = gguf_get_data_offset(ctx.get());
        for (int64_t i = 0; i < gguf_get_n_tensors(ctx.get()); ++i) {
            if (tensor_in_window(gguf_get_tensor_name(ctx.get(), i), begin, end, n_layer, tied_embeddings)) {
                const uint64_t offset = data + gguf_get_tensor_offset(ctx.get(), i);
                ranges.emplace_back(offset, offset + gguf_get_tensor_size(ctx.get(), i));
            }
        }
        if (auto prefetcher = omnarc::file_prefetcher::open(file, std::move(ranges))) {
            result.push_back(std::move(prefetcher));
        }
    }
    return result;
}

}  // namespace

struct om_shard {
    llama_model *model = nullptr;
    llama_context *ctx = nullptr;
    const llama_vocab *vocab = nullptr;

    om_model_info info{};
    uint32_t layer_begin = 0;
    uint32_t layer_end = 0;
    uint32_t max_tokens = 0;
    om_backend backend = OM_BACKEND_CPU;
    bool weights_on_host = true;  // the window's weights are read from the mapped file on every pass
    bool fits_in_memory = true;   // the window's weights stay in the page cache once read

    std::unordered_map<uint64_t, llama_seq_id> requests;
    std::vector<llama_seq_id> free_seqs;

    // batch scratch, sized for max_tokens
    std::vector<llama_pos> pos;
    std::vector<int32_t> n_seq_id;
    std::vector<llama_seq_id> seq;
    std::vector<llama_seq_id *> seq_ptr;
    std::vector<int8_t> output;

    std::vector<std::unique_ptr<omnarc::file_prefetcher>> prefetchers;

    bool first() const { return layer_begin == 0; }
    bool last() const { return layer_end > info.layer_count; }           // has the output head
    bool head_only() const { return layer_begin == info.layer_count; }  // the output head alone

    ~om_shard() {
        if (ctx != nullptr) llama_free(ctx);
        if (model != nullptr) llama_model_free(model);
    }
};

struct om_sampler {
    llama_sampler *chain = nullptr;
    std::vector<llama_token_data> candidates;

    ~om_sampler() {
        if (chain != nullptr) llama_sampler_free(chain);
    }
};

extern "C" {

uint32_t om_abi_version(void) { return OMNARC_ABI_VERSION; }

const char *om_version_string(void) { return kVersion; }

size_t om_devices(om_device *out, size_t capacity) {
    init_once();
    size_t count = 0;
    for (size_t i = 0; i < ggml_backend_dev_count(); ++i) {
        ggml_backend_dev_t dev = ggml_backend_dev_get(i);
        if (!is_gpu(dev)) continue;
        if (out != nullptr && count < capacity) {
            om_device &device = out[count];
            device.backend = device_backend(dev);
            size_t free_mem = 0;
            size_t total_mem = 0;
            ggml_backend_dev_memory(dev, &free_mem, &total_mem);
            device.memory_bytes = total_mem;
            std::snprintf(device.name, sizeof device.name, "%s", ggml_backend_dev_description(dev));
        }
        ++count;
    }
    return count;
}

const char *om_status_string(om_status status) {
    switch (status) {
        case OM_OK: return "ok";
        case OM_ERR_INVALID_ARGUMENT: return "invalid argument";
        case OM_ERR_NOT_IMPLEMENTED: return "not implemented";
        case OM_ERR_OUT_OF_MEMORY: return "out of memory";
        case OM_ERR_BACKEND: return "backend error";
        case OM_ERR_IO: return "i/o error";
        case OM_ERR_CAPACITY: return "capacity exhausted";
        case OM_ERR_UNSUPPORTED: return "unsupported";
    }
    return "unknown status";
}

const char *om_last_error(void) { return g_last_error.c_str(); }

om_status om_model_info_read(const char *model_path, om_model_info *out) {
    if (model_path == nullptr || model_path[0] == '\0' || out == nullptr) {
        return fail(OM_ERR_INVALID_ARGUMENT, "model_path and out are required");
    }
    g_last_error.clear();
    return read_info(model_path, *out);
}

om_status om_shard_open(const om_shard_params *params, om_shard **out) {
    if (out == nullptr) return fail(OM_ERR_INVALID_ARGUMENT, "out is required");
    *out = nullptr;
    if (params == nullptr || params->model_path == nullptr || params->model_path[0] == '\0') {
        return fail(OM_ERR_INVALID_ARGUMENT, "a model path is required");
    }
    if (params->layer_end <= params->layer_begin) return fail(OM_ERR_INVALID_ARGUMENT, "the layer window is empty");
    if (params->max_tokens == 0) return fail(OM_ERR_INVALID_ARGUMENT, "max_tokens must be positive");
    const int backend = static_cast<int>(params->backend);  // enums may be unsigned: compare as int
    if (backend < OM_BACKEND_AUTO || backend > OM_BACKEND_WEBGPU) {
        return fail(OM_ERR_INVALID_ARGUMENT, "unknown backend");
    }
    g_last_error.clear();

    auto shard = std::make_unique<om_shard>();
    if (const om_status status = read_info(params->model_path, shard->info); status != OM_OK) return status;
    const uint32_t n_layer = shard->info.layer_count;
    if (params->layer_end > n_layer + 1) {
        return fail(OM_ERR_INVALID_ARGUMENT, "the layer window ends after the output head (layer " +
                                                 std::to_string(n_layer) + ")");
    }
    const bool whole = params->layer_begin == 0 && params->layer_end == n_layer + 1;
    if (!whole && shard->info.splittable == 0) {
        return fail(OM_ERR_UNSUPPORTED,
                    std::string("the ") + shard->info.architecture + " architecture cannot be split into windows yet");
    }
    shard->layer_begin = params->layer_begin;
    shard->layer_end = params->layer_end;
    shard->max_tokens = params->max_tokens;

    init_once();

    // devices: the GPUs of the requested backend, every GPU for AUTO, none for CPU
    std::vector<ggml_backend_dev_t> devices;
    for (size_t i = 0; i < ggml_backend_dev_count(); ++i) {
        ggml_backend_dev_t dev = ggml_backend_dev_get(i);
        if (is_gpu(dev) && (params->backend == OM_BACKEND_AUTO || device_backend(dev) == params->backend)) {
            devices.push_back(dev);
        }
    }
    if (params->backend != OM_BACKEND_AUTO && params->backend != OM_BACKEND_CPU && devices.empty()) {
        return fail(OM_ERR_UNSUPPORTED, "the requested backend is not available on this device or in this build");
    }
    if (params->backend == OM_BACKEND_CPU) devices.clear();
    shard->backend = devices.empty() ? OM_BACKEND_CPU : device_backend(devices.front());
    shard->weights_on_host = devices.empty();
    devices.push_back(nullptr);  // the list is NULL-terminated

    llama_model_params mparams = llama_model_default_params();
    mparams.devices = devices.data();
    mparams.n_gpu_layers = shard->weights_on_host ? 0 : -1;  // all of the window's layers on the GPUs
    mparams.layer_begin = static_cast<int32_t>(params->layer_begin);
    mparams.layer_end = static_cast<int32_t>(params->layer_end);
    if (shard->weights_on_host) {
        // The window is read from the mapped file. Weights repacked for faster CPU kernels would be copied into
        // RAM instead, so repacking is used only when the window fits in free memory with room to spare.
        size_t free_mem = 0;
        size_t total_mem = 0;
        if (ggml_backend_dev_t cpu = ggml_backend_dev_by_type(GGML_BACKEND_DEVICE_TYPE_CPU)) {
            ggml_backend_dev_memory(cpu, &free_mem, &total_mem);
        }
        const uint64_t window_bytes = shard->info.file_size * (params->layer_end - params->layer_begin) / (n_layer + 1);
        shard->fits_in_memory = free_mem > 0 && window_bytes < free_mem / 10 * 7;
        mparams.use_extra_bufts = shard->fits_in_memory;
    }

    shard->model = llama_model_load_from_file(params->model_path, mparams);
    if (shard->model == nullptr) {
        return fail(OM_ERR_BACKEND, g_last_error.empty() ? "failed to load the model" : g_last_error);
    }
    shard->vocab = llama_model_get_vocab(shard->model);

    llama_context_params cparams = llama_context_default_params();
    cparams.n_ctx = params->max_tokens;
    cparams.n_batch = params->max_tokens;
    cparams.n_ubatch = std::min(params->max_tokens, kMaxUbatch);
    cparams.n_seq_max = kMaxRequests;
    cparams.kv_unified = true;
    cparams.n_threads = static_cast<int32_t>(omnarc::physical_cores());
    cparams.n_threads_batch = cparams.n_threads;
    cparams.layer_begin = static_cast<int32_t>(params->layer_begin);
    cparams.layer_end = static_cast<int32_t>(params->layer_end);
    cparams.no_perf = true;

    shard->ctx = llama_init_from_model(shard->model, cparams);
    if (shard->ctx == nullptr) {
        return fail(OM_ERR_OUT_OF_MEMORY, g_last_error.empty() ? "failed to create the context" : g_last_error);
    }

    for (uint32_t s = kMaxRequests; s > 0; --s) shard->free_seqs.push_back(static_cast<llama_seq_id>(s - 1));
    const size_t n = params->max_tokens;
    shard->pos.resize(n);
    shard->n_seq_id.assign(n, 1);
    shard->seq.resize(n);
    shard->seq_ptr.resize(n);
    for (size_t i = 0; i < n; ++i) shard->seq_ptr[i] = &shard->seq[i];
    shard->output.resize(n);

    // a window that stays in memory needs no prefetching; one that does not is read again on every pass
    if (shard->weights_on_host && !shard->fits_in_memory) {
        shard->prefetchers = window_prefetchers(params->model_path, params->layer_begin, params->layer_end, n_layer);
    }

    *out = shard.release();
    return OM_OK;
}

om_status om_shard_get_info(const om_shard *shard, om_shard_info *out) {
    if (shard == nullptr || out == nullptr) return fail(OM_ERR_INVALID_ARGUMENT, "shard and out are required");
    out->model = shard->info;
    out->layer_begin = shard->layer_begin;
    out->layer_end = shard->layer_end;
    out->backend = shard->backend;
    return OM_OK;
}

om_status om_shard_run(om_shard *shard, const om_batch *batch, float *out, size_t out_count) {
    if (shard == nullptr || batch == nullptr || out == nullptr) {
        return fail(OM_ERR_INVALID_ARGUMENT, "shard, batch and out are required");
    }
    const uint32_t n = batch->token_count;
    if (n == 0 || n > shard->max_tokens) {
        return fail(OM_ERR_INVALID_ARGUMENT, "token_count must be between 1 and max_tokens");
    }
    if (shard->first() ? (batch->tokens == nullptr || batch->hidden != nullptr)
                       : (batch->hidden == nullptr || batch->tokens != nullptr)) {
        return fail(OM_ERR_INVALID_ARGUMENT,
                    shard->first() ? "the first shard takes token ids" : "this shard takes hidden states");
    }
    const size_t row = shard->last() ? shard->info.vocab_size : shard->info.hidden_dim;
    const size_t rows = shard->last() && batch->all_logits == 0 ? 1 : n;
    if (out_count < rows * row) {
        return fail(OM_ERR_INVALID_ARGUMENT, "out holds " + std::to_string(out_count) + " floats, " +
                                                 std::to_string(rows * row) + " are needed");
    }
    g_last_error.clear();

    auto it = shard->requests.find(batch->request_id);
    if (it == shard->requests.end()) {
        if (shard->free_seqs.empty()) return fail(OM_ERR_CAPACITY, "all request slots of the shard are in use");
        it = shard->requests.emplace(batch->request_id, shard->free_seqs.back()).first;
        shard->free_seqs.pop_back();
    }
    const llama_seq_id seq = it->second;

    // The output head alone has no attention and so no use for the KV cache: it gets only the rows that need
    // logits, and its cache entries are cleared before every step.
    uint32_t first_row = 0;
    uint32_t count = n;
    if (shard->head_only()) {
        llama_memory_seq_rm(llama_get_memory(shard->ctx), seq, -1, -1);
        if (batch->all_logits == 0) {
            first_row = n - 1;
            count = 1;
        }
    }
    for (uint32_t i = 0; i < count; ++i) {
        shard->pos[i] = static_cast<llama_pos>(batch->position_start + first_row + i);
        shard->seq[i] = seq;
        shard->output[i] = !shard->last() || shard->head_only() || batch->all_logits != 0 || i == count - 1;
    }
    llama_batch lb{};
    lb.n_tokens = static_cast<int32_t>(count);
    lb.token = const_cast<llama_token *>(batch->tokens);  // llama.cpp only reads the inputs
    lb.embd = const_cast<float *>(batch->hidden);
    if (lb.embd != nullptr) lb.embd += static_cast<size_t>(first_row) * shard->info.hidden_dim;
    lb.pos = shard->pos.data();
    lb.n_seq_id = shard->n_seq_id.data();
    lb.seq_id = shard->seq_ptr.data();
    lb.logits = shard->output.data();

    const int32_t rc = llama_decode(shard->ctx, lb);
    if (rc == 1) return fail(OM_ERR_CAPACITY, "the KV cache of the shard is full");
    if (rc != 0) {
        return fail(rc == -1 ? OM_ERR_INVALID_ARGUMENT : OM_ERR_BACKEND,
                    g_last_error.empty() ? "llama_decode failed (" + std::to_string(rc) + ")" : g_last_error);
    }

    for (size_t r = 0; r < rows; ++r) {
        const auto index = static_cast<int32_t>(rows == 1 ? count - 1 : r);
        const float *src = shard->last() ? llama_get_logits_ith(shard->ctx, index)
                                         : llama_get_embeddings_ith(shard->ctx, index);
        if (src == nullptr) {
            return fail(OM_ERR_BACKEND, "the shard produced no output for token " + std::to_string(index));
        }
        std::memcpy(out + r * row, src, row * sizeof(float));
    }
    return OM_OK;
}

om_status om_shard_forward(om_shard *shard, uint64_t request_id, uint32_t position_start, uint32_t token_count,
                           const float *hidden_in, float *hidden_out) {
    if (shard == nullptr || token_count == 0 || hidden_in == nullptr || hidden_out == nullptr) {
        return fail(OM_ERR_INVALID_ARGUMENT, "shard, token_count, hidden_in and hidden_out are required");
    }
    if (shard->first() || shard->last()) {
        return fail(OM_ERR_INVALID_ARGUMENT, "om_shard_forward runs middle shards only; use om_shard_run");
    }
    const om_batch batch = {request_id, position_start, token_count, nullptr, hidden_in, 0};
    return om_shard_run(shard, &batch, hidden_out, static_cast<size_t>(token_count) * shard->info.hidden_dim);
}

om_status om_shard_drop(om_shard *shard, uint64_t request_id) {
    if (shard == nullptr) return fail(OM_ERR_INVALID_ARGUMENT, "shard is required");
    const auto it = shard->requests.find(request_id);
    if (it == shard->requests.end()) return OM_OK;
    llama_memory_seq_rm(llama_get_memory(shard->ctx), it->second, -1, -1);
    shard->free_seqs.push_back(it->second);
    shard->requests.erase(it);
    return OM_OK;
}

om_status om_shard_truncate(om_shard *shard, uint64_t request_id, uint32_t position) {
    if (shard == nullptr) return fail(OM_ERR_INVALID_ARGUMENT, "shard is required");
    const auto it = shard->requests.find(request_id);
    if (it == shard->requests.end()) return fail(OM_ERR_INVALID_ARGUMENT, "unknown request");
    if (!llama_memory_seq_rm(llama_get_memory(shard->ctx), it->second, static_cast<llama_pos>(position), -1)) {
        return fail(OM_ERR_BACKEND, "the KV cache cannot be truncated at this position");
    }
    return OM_OK;
}

om_status om_shard_kv_size(om_shard *shard, uint64_t request_id, size_t *out_size) {
    if (shard == nullptr || out_size == nullptr) return fail(OM_ERR_INVALID_ARGUMENT, "shard and out_size are required");
    const auto it = shard->requests.find(request_id);
    if (it == shard->requests.end()) return fail(OM_ERR_INVALID_ARGUMENT, "unknown request");
    *out_size = llama_state_seq_get_size(shard->ctx, it->second);
    return OM_OK;
}

om_status om_shard_kv_read(om_shard *shard, uint64_t request_id, void *out, size_t out_size, size_t *out_written) {
    if (shard == nullptr || out == nullptr) return fail(OM_ERR_INVALID_ARGUMENT, "shard and out are required");
    const auto it = shard->requests.find(request_id);
    if (it == shard->requests.end()) return fail(OM_ERR_INVALID_ARGUMENT, "unknown request");
    const size_t needed = llama_state_seq_get_size(shard->ctx, it->second);
    if (out_size < needed) return fail(OM_ERR_CAPACITY, "the buffer is smaller than the KV cache");
    const size_t written = llama_state_seq_get_data(shard->ctx, static_cast<uint8_t *>(out), out_size, it->second);
    if (written == 0) return fail(OM_ERR_BACKEND, "the KV cache cannot be read");
    if (out_written != nullptr) *out_written = written;
    return OM_OK;
}

om_status om_shard_kv_write(om_shard *shard, uint64_t request_id, const void *data, size_t size) {
    if (shard == nullptr || data == nullptr) return fail(OM_ERR_INVALID_ARGUMENT, "shard and data are required");
    if (size == 0) return fail(OM_ERR_INVALID_ARGUMENT, "the KV cache is empty");
    auto it = shard->requests.find(request_id);
    if (it == shard->requests.end()) {
        if (shard->free_seqs.empty()) return fail(OM_ERR_CAPACITY, "all request slots of the shard are in use");
        it = shard->requests.emplace(request_id, shard->free_seqs.back()).first;
        shard->free_seqs.pop_back();
    } else {
        // whatever the request held is replaced, so the written cache is the whole of it
        llama_memory_seq_rm(llama_get_memory(shard->ctx), it->second, -1, -1);
    }
    if (llama_state_seq_set_data(shard->ctx, static_cast<const uint8_t *>(data), size, it->second) == 0) {
        return fail(OM_ERR_BACKEND, "the KV cache cannot be written: it came from a different layer window");
    }
    return OM_OK;
}

om_status om_shard_prefetch(om_shard *shard) {
    if (shard == nullptr) return fail(OM_ERR_INVALID_ARGUMENT, "shard is required");
    for (auto &prefetcher : shard->prefetchers) prefetcher->prefetch();
    return OM_OK;
}

om_status om_shard_set_threads(om_shard *shard, uint32_t threads) {
    if (shard == nullptr) return fail(OM_ERR_INVALID_ARGUMENT, "shard is required");
    const auto n = static_cast<int32_t>(threads == 0 ? omnarc::physical_cores() : threads);
    llama_set_n_threads(shard->ctx, n, n);
    return OM_OK;
}

om_status om_shard_tokenize(const om_shard *shard, const char *text, size_t text_len, uint32_t add_special,
                            int32_t *tokens, size_t capacity, size_t *count) {
    if (shard == nullptr || (text == nullptr && text_len > 0) || count == nullptr || text_len > INT32_MAX) {
        return fail(OM_ERR_INVALID_ARGUMENT, "shard, text and count are required");
    }
    const auto cap = static_cast<int32_t>(std::min<size_t>(capacity, INT32_MAX));
    const int32_t n = llama_tokenize(shard->vocab, text, static_cast<int32_t>(text_len), tokens,
                                     tokens != nullptr ? cap : 0, add_special != 0, /*parse_special =*/true);
    if (n == INT32_MIN) return fail(OM_ERR_CAPACITY, "the text is too long");
    *count = static_cast<size_t>(n < 0 ? -n : n);
    return n < 0 ? fail(OM_ERR_CAPACITY, "the token buffer is too small") : OM_OK;
}

om_status om_shard_token_text(const om_shard *shard, int32_t token, char *out, size_t capacity, size_t *len) {
    if (shard == nullptr || len == nullptr || token < 0 || token >= llama_vocab_n_tokens(shard->vocab)) {
        return fail(OM_ERR_INVALID_ARGUMENT, "shard, a valid token and len are required");
    }
    const auto cap = static_cast<int32_t>(std::min<size_t>(capacity, INT32_MAX));
    const int32_t n = llama_token_to_piece(shard->vocab, token, out, out != nullptr ? cap : 0, 0, /*special =*/false);
    *len = static_cast<size_t>(n < 0 ? -n : n);
    return n < 0 ? fail(OM_ERR_CAPACITY, "the text buffer is too small") : OM_OK;
}

uint32_t om_shard_token_is_end(const om_shard *shard, int32_t token) {
    return shard != nullptr && llama_vocab_is_eog(shard->vocab, token) ? 1 : 0;
}

void om_shard_close(om_shard *shard) { delete shard; }

om_status om_sampler_open(const om_sampler_params *params, om_sampler **out) {
    if (params == nullptr || out == nullptr) return fail(OM_ERR_INVALID_ARGUMENT, "params and out are required");
    *out = nullptr;
    init_once();
    auto sampler = std::make_unique<om_sampler>();
    sampler->chain = llama_sampler_chain_init(llama_sampler_chain_default_params());
    if (params->temperature <= 0.0f) {
        llama_sampler_chain_add(sampler->chain, llama_sampler_init_greedy());
    } else {
        if (params->top_k > 0) {
            llama_sampler_chain_add(sampler->chain, llama_sampler_init_top_k(static_cast<int32_t>(params->top_k)));
        }
        if (params->top_p < 1.0f) llama_sampler_chain_add(sampler->chain, llama_sampler_init_top_p(params->top_p, 1));
        if (params->min_p > 0.0f) llama_sampler_chain_add(sampler->chain, llama_sampler_init_min_p(params->min_p, 1));
        llama_sampler_chain_add(sampler->chain, llama_sampler_init_temp(params->temperature));
        llama_sampler_chain_add(sampler->chain, llama_sampler_init_dist(static_cast<uint32_t>(params->seed)));
    }
    *out = sampler.release();
    return OM_OK;
}

om_status om_sampler_sample(om_sampler *sampler, const float *logits, uint32_t vocab_size, int32_t *token) {
    if (sampler == nullptr || logits == nullptr || vocab_size == 0 || token == nullptr) {
        return fail(OM_ERR_INVALID_ARGUMENT, "sampler, logits, vocab_size and token are required");
    }
    sampler->candidates.resize(vocab_size);
    for (uint32_t i = 0; i < vocab_size; ++i) {
        sampler->candidates[i] = llama_token_data{static_cast<llama_token>(i), logits[i], 0.0f};
    }
    llama_token_data_array array = {sampler->candidates.data(), vocab_size, -1, false};
    llama_sampler_apply(sampler->chain, &array);
    if (array.selected < 0 || static_cast<size_t>(array.selected) >= array.size) {
        return fail(OM_ERR_BACKEND, "the sampler selected no token");
    }
    *token = array.data[array.selected].id;
    llama_sampler_accept(sampler->chain, *token);
    return OM_OK;
}

void om_sampler_close(om_sampler *sampler) { delete sampler; }

}  // extern "C"
