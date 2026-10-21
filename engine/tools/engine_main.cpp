// omnarc-engine — libomnarc in its own process, driven by omnarc-node over stdin/stdout with the bulk data in shared
// memory. The protocol is specified in engine/HOST.md.
#include <algorithm>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <memory>
#include <string>
#include <unordered_map>
#include <vector>

#include "omnarc/omnarc.h"
#include "ggml.h"

#if defined(_WIN32)
#  ifndef NOMINMAX
#    define NOMINMAX
#  endif
#  ifndef WIN32_LEAN_AND_MEAN
#    define WIN32_LEAN_AND_MEAN
#  endif
#  include <fcntl.h>
#  include <io.h>
#  include <windows.h>
#else
#  include <fcntl.h>
#  include <sys/mman.h>
#  include <unistd.h>
#endif

namespace {

enum opcode : uint16_t {
    OP_HELLO = 1,
    OP_MODEL_INFO = 2,
    OP_OPEN = 3,
    OP_CLOSE = 4,
    OP_RUN = 5,
    OP_DROP = 6,
    OP_TRUNCATE = 7,
    OP_PREFETCH = 8,
    OP_THREADS = 9,
    OP_TOKENIZE = 10,
    OP_TOKEN_TEXT = 11,
    OP_SAMPLER_OPEN = 12,
    OP_SAMPLE = 13,
    OP_SAMPLER_CLOSE = 14,
    OP_QUIT = 15,
    OP_DEVICES = 16,
    OP_KV_SIZE = 17,
    OP_KV_READ = 18,
    OP_KV_WRITE = 19,
    OP_CHAT_PROMPT = 20,
    OP_BENCH = 21,
};

enum element : uint8_t { EL_TOKENS = 0, EL_F32 = 1, EL_F16 = 2, EL_BF16 = 3 };

// The shared-memory region created by the host: input area, then output area.
class region {
public:
    bool open(const std::string &name, size_t size) {
        size_ = size;
#if defined(_WIN32)
        const std::wstring wide(name.begin(), name.end());
        mapping_ = OpenFileMappingW(FILE_MAP_ALL_ACCESS, FALSE, wide.c_str());
        if (mapping_ == nullptr) return false;
        base_ = static_cast<uint8_t *>(MapViewOfFile(mapping_, FILE_MAP_ALL_ACCESS, 0, 0, size));
        return base_ != nullptr;
#else
#if defined(__ANDROID__)
        // Android has no POSIX shared memory; there the host's region is an ordinary file, mapped shared
        const int fd = ::open(name.c_str(), O_RDWR);
#else
        const int fd = shm_open(name.c_str(), O_RDWR, 0600);
#endif
        if (fd < 0) return false;
        void *addr = mmap(nullptr, size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
        close(fd);
        if (addr == MAP_FAILED) return false;
        base_ = static_cast<uint8_t *>(addr);
        return true;
#endif
    }

    ~region() {
#if defined(_WIN32)
        if (base_ != nullptr) UnmapViewOfFile(base_);
        if (mapping_ != nullptr) CloseHandle(mapping_);
#else
        if (base_ != nullptr) munmap(base_, size_);
#endif
    }

    size_t size() const { return size_; }
    size_t half() const { return size_ / 2; }
    uint8_t *input() const { return base_; }
    uint8_t *output() const { return base_ + half(); }

private:
    uint8_t *base_ = nullptr;
    size_t size_ = 0;
#if defined(_WIN32)
    HANDLE mapping_ = nullptr;
#endif
};

// Little-endian reader over a request payload.
class reader {
public:
    explicit reader(const std::vector<uint8_t> &data) : data_(data) {}

    template <typename T>
    T get() {
        T value{};
        if (pos_ + sizeof(T) > data_.size()) {
            ok_ = false;
            return value;
        }
        std::memcpy(&value, data_.data() + pos_, sizeof(T));
        pos_ += sizeof(T);
        return value;
    }

    std::string str() {
        const auto len = get<uint32_t>();
        if (!ok_ || pos_ + len > data_.size()) {
            ok_ = false;
            return {};
        }
        std::string s(reinterpret_cast<const char *>(data_.data() + pos_), len);
        pos_ += len;
        return s;
    }

    bool ok() const { return ok_; }
    bool more() const { return pos_ < data_.size(); }

private:
    const std::vector<uint8_t> &data_;
    size_t pos_ = 0;
    bool ok_ = true;
};

// Little-endian writer of a response payload.
class writer {
public:
    template <typename T>
    void put(T value) {
        const auto *p = reinterpret_cast<const uint8_t *>(&value);
        data.insert(data.end(), p, p + sizeof(T));
    }

    void str(const std::string &s) {
        put(static_cast<uint32_t>(s.size()));
        data.insert(data.end(), s.begin(), s.end());
    }

    std::vector<uint8_t> data;
};

bool read_exact(void *buf, size_t n) { return std::fread(buf, 1, n, stdin) == n; }

bool respond(uint16_t status, const std::vector<uint8_t> &payload) {
    const auto length = static_cast<uint32_t>(4 + payload.size());
    const uint16_t zero = 0;
    const bool ok = std::fwrite(&length, 4, 1, stdout) == 1 && std::fwrite(&status, 2, 1, stdout) == 1 &&
                    std::fwrite(&zero, 2, 1, stdout) == 1 &&
                    (payload.empty() || std::fwrite(payload.data(), 1, payload.size(), stdout) == payload.size());
    return ok && std::fflush(stdout) == 0;
}

void put_model_info(writer &w, const om_model_info &info) {
    w.str(info.architecture);
    w.put(info.layer_count);
    w.put(info.hidden_dim);
    w.put(info.vocab_size);
    w.put(info.context_length);
    w.put(info.file_size);
    w.put(static_cast<uint8_t>(info.splittable));
}

struct engine {
    region shm;
    std::unordered_map<uint32_t, om_shard *> shards;
    std::unordered_map<uint32_t, om_sampler *> samplers;
    uint32_t next_id = 1;
    std::vector<float> scratch_in;
    std::vector<float> scratch_out;

    ~engine() {
        for (auto &[id, shard] : shards) om_shard_close(shard);
        for (auto &[id, sampler] : samplers) om_sampler_close(sampler);
    }

    om_shard *shard(uint32_t id) const {
        const auto it = shards.find(id);
        return it == shards.end() ? nullptr : it->second;
    }

    // Handles one request; returns the status and fills the response payload.
    om_status handle(uint16_t op, const std::vector<uint8_t> &payload, writer &out, std::string &error) {
        reader in(payload);
        switch (op) {
            case OP_HELLO: {
                out.put(om_abi_version());
                out.str(om_version_string());
                out.put(static_cast<uint64_t>(shm.size()));
                return OM_OK;
            }
            case OP_DEVICES: {
                std::vector<om_device> devices(om_devices(nullptr, 0));
                devices.resize(std::min(devices.size(), om_devices(devices.data(), devices.size())));
                out.put(static_cast<uint32_t>(devices.size()));
                for (const om_device &device : devices) {
                    out.put(static_cast<uint32_t>(device.backend));
                    out.put(device.memory_bytes);
                    out.str(device.name);
                    // ABI 7
                    out.put(device.integrated);
                    out.put(device.free_bytes);
                }
                return OM_OK;
            }
            case OP_BENCH: {
                const auto index = in.get<uint32_t>();
                const auto millis = in.get<uint32_t>();
                if (!in.ok()) break;
                double bandwidth = 0;
                double flops = 0;
                const om_status status = om_device_bench(index, millis, &bandwidth, &flops);
                if (status == OM_OK) {
                    out.put(bandwidth);
                    out.put(flops);
                }
                return status;
            }
            case OP_MODEL_INFO: {
                const std::string path = in.str();
                if (!in.ok()) break;
                om_model_info info{};
                const om_status status = om_model_info_read(path.c_str(), &info);
                if (status == OM_OK) put_model_info(out, info);
                return status;
            }
            case OP_OPEN: {
                const std::string path = in.str();
                om_shard_params params{};
                params.model_path = path.c_str();
                params.layer_begin = in.get<uint32_t>();
                params.layer_end = in.get<uint32_t>();
                params.backend = static_cast<om_backend>(in.get<uint32_t>());
                params.max_tokens = in.get<uint32_t>();
                // ABI 5: the KV cache type follows; an older host sends none and gets f16
                if (in.more()) params.kv_type = static_cast<om_kv_type>(in.get<uint32_t>());
                // ABI 7: the GPU and how many layers run on it
                if (in.more()) {
                    params.gpu = in.get<uint32_t>();
                    params.gpu_layers = in.get<uint32_t>();
                }
                if (!in.ok()) break;
                om_shard *s = nullptr;
                const om_status status = om_shard_open(&params, &s);
                if (status != OM_OK) return status;
                om_shard_info info{};
                om_shard_get_info(s, &info);
                const uint32_t id = next_id++;
                shards[id] = s;
                out.put(id);
                put_model_info(out, info.model);
                out.put(info.layer_begin);
                out.put(info.layer_end);
                out.put(static_cast<uint32_t>(info.backend));
                return OM_OK;
            }
            case OP_CLOSE: {
                const auto id = in.get<uint32_t>();
                if (!in.ok()) break;
                const auto it = shards.find(id);
                if (it != shards.end()) {
                    om_shard_close(it->second);
                    shards.erase(it);
                }
                return OM_OK;
            }
            case OP_RUN: return run(in, out, error);
            case OP_DROP: {
                om_shard *s = shard(in.get<uint32_t>());
                const auto request = in.get<uint64_t>();
                if (!in.ok() || s == nullptr) break;
                return om_shard_drop(s, request);
            }
            case OP_TRUNCATE: {
                om_shard *s = shard(in.get<uint32_t>());
                const auto request = in.get<uint64_t>();
                const auto position = in.get<uint32_t>();
                if (!in.ok() || s == nullptr) break;
                return om_shard_truncate(s, request, position);
            }
            case OP_KV_SIZE: {
                om_shard *s = shard(in.get<uint32_t>());
                const auto request = in.get<uint64_t>();
                if (!in.ok() || s == nullptr) break;
                size_t size = 0;
                const om_status status = om_shard_kv_size(s, request, &size);
                if (status == OM_OK) out.put(static_cast<uint64_t>(size));
                return status;
            }
            case OP_KV_READ: {
                om_shard *s = shard(in.get<uint32_t>());
                const auto request = in.get<uint64_t>();
                if (!in.ok() || s == nullptr) break;
                size_t written = 0;
                const om_status status = om_shard_kv_read(s, request, shm.output(), shm.half(), &written);
                if (status == OM_OK) out.put(static_cast<uint64_t>(written));
                return status;
            }
            case OP_KV_WRITE: {
                om_shard *s = shard(in.get<uint32_t>());
                const auto request = in.get<uint64_t>();
                const auto size = in.get<uint64_t>();
                if (!in.ok() || s == nullptr || size > shm.half()) break;
                return om_shard_kv_write(s, request, shm.input(), static_cast<size_t>(size));
            }
            case OP_PREFETCH: {
                om_shard *s = shard(in.get<uint32_t>());
                if (!in.ok() || s == nullptr) break;
                return om_shard_prefetch(s);
            }
            case OP_THREADS: {
                om_shard *s = shard(in.get<uint32_t>());
                const auto threads = in.get<uint32_t>();
                if (!in.ok() || s == nullptr) break;
                return om_shard_set_threads(s, threads);
            }
            case OP_TOKENIZE: {
                om_shard *s = shard(in.get<uint32_t>());
                const auto add_special = in.get<uint8_t>();
                const std::string text = in.str();
                if (!in.ok() || s == nullptr) break;
                size_t count = 0;
                auto *tokens = reinterpret_cast<int32_t *>(shm.output());
                const om_status status = om_shard_tokenize(s, text.data(), text.size(), add_special, tokens,
                                                           shm.half() / sizeof(int32_t), &count);
                if (status == OM_OK) out.put(static_cast<uint32_t>(count));
                return status;
            }
            case OP_CHAT_PROMPT: {
                om_shard *s = shard(in.get<uint32_t>());
                const auto count = in.get<uint32_t>();
                std::vector<std::string> texts;
                for (uint32_t i = 0; i < count && in.ok(); ++i) {
                    texts.push_back(in.str());
                    texts.push_back(in.str());
                }
                if (!in.ok() || s == nullptr) break;
                std::vector<om_chat_message> messages(count);
                for (uint32_t i = 0; i < count; ++i) messages[i] = {texts[2 * i].c_str(), texts[2 * i + 1].c_str()};
                std::string prompt(4096, '\0');
                size_t len = 0;
                om_status status = om_shard_chat_prompt(s, messages.data(), count, prompt.data(), prompt.size(), &len);
                if (status == OM_ERR_CAPACITY) {
                    prompt.resize(len);
                    status = om_shard_chat_prompt(s, messages.data(), count, prompt.data(), prompt.size(), &len);
                }
                if (status != OM_OK) return status;
                prompt.resize(len);
                out.str(prompt);
                return status;
            }
            case OP_TOKEN_TEXT: {
                om_shard *s = shard(in.get<uint32_t>());
                const auto token = in.get<int32_t>();
                if (!in.ok() || s == nullptr) break;
                char buf[256];
                size_t len = 0;
                om_status status = om_shard_token_text(s, token, buf, sizeof(buf), &len);
                std::string text(buf, status == OM_OK ? len : 0);
                if (status == OM_ERR_CAPACITY) {
                    text.resize(len);
                    status = om_shard_token_text(s, token, text.data(), text.size(), &len);
                }
                if (status != OM_OK) return status;
                out.str(text);
                out.put(static_cast<uint8_t>(om_shard_token_is_end(s, token)));
                return OM_OK;
            }
            case OP_SAMPLER_OPEN: {
                om_sampler_params params{};
                params.temperature = in.get<float>();
                params.top_k = in.get<uint32_t>();
                params.top_p = in.get<float>();
                params.min_p = in.get<float>();
                params.seed = in.get<uint64_t>();
                if (!in.ok()) break;
                om_sampler *sampler = nullptr;
                const om_status status = om_sampler_open(&params, &sampler);
                if (status != OM_OK) return status;
                const uint32_t id = next_id++;
                samplers[id] = sampler;
                out.put(id);
                return OM_OK;
            }
            case OP_SAMPLE: {
                const auto id = in.get<uint32_t>();
                const auto vocab = in.get<uint32_t>();
                const auto it = samplers.find(id);
                if (!in.ok() || it == samplers.end() || static_cast<size_t>(vocab) * sizeof(float) > shm.half()) break;
                int32_t token = -1;
                const om_status status =
                    om_sampler_sample(it->second, reinterpret_cast<const float *>(shm.output()), vocab, &token);
                if (status == OM_OK) out.put(token);
                return status;
            }
            case OP_SAMPLER_CLOSE: {
                const auto id = in.get<uint32_t>();
                if (!in.ok()) break;
                const auto it = samplers.find(id);
                if (it != samplers.end()) {
                    om_sampler_close(it->second);
                    samplers.erase(it);
                }
                return OM_OK;
            }
            default: error = "unknown request " + std::to_string(op); return OM_ERR_INVALID_ARGUMENT;
        }
        error = "malformed request, or an unknown shard or sampler";
        return OM_ERR_INVALID_ARGUMENT;
    }

    om_status run(reader &in, writer &out, std::string &error) {
        om_shard *s = shard(in.get<uint32_t>());
        om_batch batch{};
        batch.request_id = in.get<uint64_t>();
        batch.position_start = in.get<uint32_t>();
        batch.token_count = in.get<uint32_t>();
        const auto input = in.get<uint8_t>();
        batch.all_logits = in.get<uint8_t>();
        const auto output = in.get<uint8_t>();
        if (!in.ok() || s == nullptr || batch.token_count == 0) {
            error = "malformed run request or unknown shard";
            return OM_ERR_INVALID_ARGUMENT;
        }

        om_shard_info info{};
        om_shard_get_info(s, &info);
        const size_t n = batch.token_count;
        const size_t hidden = info.model.hidden_dim;
        const bool head = info.layer_end > info.model.layer_count;

        // input
        const size_t in_elems = input == EL_TOKENS ? n : n * hidden;
        const size_t in_size = input == EL_F32 || input == EL_TOKENS ? 4 : 2;
        if (in_elems * in_size > shm.half()) {
            error = "the input does not fit in the shared memory";
            return OM_ERR_CAPACITY;
        }
        const void *src = shm.input();
        switch (input) {
            case EL_TOKENS: batch.tokens = static_cast<const int32_t *>(src); break;
            case EL_F32: batch.hidden = static_cast<const float *>(src); break;
            case EL_F16:
                scratch_in.resize(in_elems);
                ggml_fp16_to_fp32_row(static_cast<const ggml_fp16_t *>(src), scratch_in.data(),
                                      static_cast<int64_t>(in_elems));
                batch.hidden = scratch_in.data();
                break;
            case EL_BF16:
                scratch_in.resize(in_elems);
                ggml_bf16_to_fp32_row(static_cast<const ggml_bf16_t *>(src), scratch_in.data(),
                                      static_cast<int64_t>(in_elems));
                batch.hidden = scratch_in.data();
                break;
            default: error = "unknown input type"; return OM_ERR_INVALID_ARGUMENT;
        }

        // output: logits are always f32, hidden states in the requested type
        const uint32_t rows = head && batch.all_logits == 0 ? 1 : batch.token_count;
        const uint32_t row = head ? info.model.vocab_size : info.model.hidden_dim;
        const size_t out_elems = static_cast<size_t>(rows) * row;
        const bool direct = head || output == EL_F32;
        if (out_elems * (direct ? 4 : 2) > shm.half()) {
            error = "the output does not fit in the shared memory";
            return OM_ERR_CAPACITY;
        }
        float *dst = reinterpret_cast<float *>(shm.output());
        if (!direct) {
            scratch_out.resize(out_elems);
            dst = scratch_out.data();
        }
        const om_status status = om_shard_run(s, &batch, dst, out_elems);
        if (status != OM_OK) return status;
        if (!direct) {
            const auto count = static_cast<int64_t>(out_elems);
            if (output == EL_F16) {
                ggml_fp32_to_fp16_row(dst, reinterpret_cast<ggml_fp16_t *>(shm.output()), count);
            } else if (output == EL_BF16) {
                ggml_fp32_to_bf16_row(dst, reinterpret_cast<ggml_bf16_t *>(shm.output()), count);
            } else {
                error = "unknown output type";
                return OM_ERR_INVALID_ARGUMENT;
            }
        }
        out.put(rows);
        out.put(row);
        return OM_OK;
    }
};

}  // namespace

int main(int argc, char **argv) {
    std::string shm_name;
    size_t shm_size = 0;
    for (int i = 1; i + 1 < argc; i += 2) {
        if (std::strcmp(argv[i], "--shm") == 0) shm_name = argv[i + 1];
        if (std::strcmp(argv[i], "--shm-size") == 0) {
            shm_size = static_cast<size_t>(std::strtoull(argv[i + 1], nullptr, 10));
        }
    }
    if (shm_name.empty() || shm_size < 4096) {
        std::fprintf(stderr, "usage: omnarc-engine --shm NAME --shm-size BYTES  (started by omnarc-node)\n");
        return 2;
    }
#if defined(_WIN32)
    _setmode(_fileno(stdin), _O_BINARY);
    _setmode(_fileno(stdout), _O_BINARY);
#endif

    engine e;
    if (!e.shm.open(shm_name, shm_size)) {
        std::fprintf(stderr, "omnarc-engine: cannot open shared memory %s\n", shm_name.c_str());
        return 1;
    }

    std::vector<uint8_t> payload;
    for (;;) {
        uint32_t length = 0;
        uint16_t head[2] = {0, 0};
        if (!read_exact(&length, 4) || length < 4 || !read_exact(head, 4)) break;  // the host closed the pipe
        payload.resize(length - 4);
        if (!payload.empty() && !read_exact(payload.data(), payload.size())) break;
        if (head[0] == OP_QUIT) {
            respond(OM_OK, {});
            break;
        }

        writer out;
        std::string error;
        const om_status status = e.handle(head[0], payload, out, error);
        if (status != OM_OK) {
            writer err;
            err.str(error.empty() ? om_last_error() : error);
            if (!respond(static_cast<uint16_t>(status), err.data)) break;
        } else if (!respond(OM_OK, out.data)) {
            break;
        }
    }
    return 0;
}
