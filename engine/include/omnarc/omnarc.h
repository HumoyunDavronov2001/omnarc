/* libomnarc — the Omnarc engine C ABI.
 *
 * Every platform calls the engine through this header: the Rust agent (FFI), the Android app (JNI through the
 * Rust core) and the iOS app (Swift through the Rust core). The ABI only grows: functions are never removed and
 * struct fields are only appended, so om_abi_version() tells callers what they can use.
 *
 * A model is split into windows of consecutive layers. The repeating layers are numbered 0 .. layer_count - 1 and
 * the output head (output norm and the matrix that gives logits) is layer layer_count, as in llama.cpp's
 * n_gpu_layers; the whole model is the window [0, layer_count + 1). A shard is one window loaded on one device; the
 * windows of a model form a pipeline (the ring). The shard that starts at layer 0 takes token ids, the others take
 * the hidden state of the previous layer; the shard with the output head gives logits, the others hidden states.
 * Shards are built on llama.cpp with the patches in engine/patches. A shard is not thread-safe: call it from one
 * thread at a time. */
#ifndef OMNARC_OMNARC_H
#define OMNARC_OMNARC_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* 1: stage B0 interface. 2: shards on llama.cpp — model info, om_shard_run, KV management, prefetching,
 * tokenizer and sampler. 3: om_devices. */
#define OMNARC_ABI_VERSION 5u

typedef enum om_status {
    OM_OK = 0,
    OM_ERR_INVALID_ARGUMENT = 1,
    OM_ERR_NOT_IMPLEMENTED = 2,
    OM_ERR_OUT_OF_MEMORY = 3,
    OM_ERR_BACKEND = 4,
    OM_ERR_IO = 5,
    OM_ERR_CAPACITY = 6,   /* the KV cache or the request slots of a shard are full */
    OM_ERR_UNSUPPORTED = 7 /* the model or the requested backend cannot be used this way */
} om_status;

/* Values match omnarc.v1.Backend in protocol/omnarc/v1/device.proto. */
typedef enum om_backend {
    OM_BACKEND_AUTO = 0,
    OM_BACKEND_CPU = 1,
    OM_BACKEND_CUDA = 2,
    OM_BACKEND_HIP = 3,
    OM_BACKEND_VULKAN = 4,
    OM_BACKEND_METAL = 5,
    OM_BACKEND_OPENCL = 6,
    OM_BACKEND_HEXAGON = 7,
    OM_BACKEND_SYCL = 8,
    OM_BACKEND_CANN = 9,
    OM_BACKEND_WEBGPU = 10
} om_backend;

/* A loaded, contiguous range of model layers on one device. Opaque. */
typedef struct om_shard om_shard;

/* The element type of a shard's KV cache (SPEC EN-16): 8- and 4-bit caches hold two and almost four times as
 * many tokens in the same memory. */
typedef enum om_kv_type {
    OM_KV_F16 = 0,
    OM_KV_Q8_0 = 1,
    OM_KV_Q4_0 = 2,
} om_kv_type;

typedef struct om_shard_params {
    const char *model_path; /* GGUF file */
    uint32_t layer_begin;   /* first layer, inclusive */
    uint32_t layer_end;     /* last layer, exclusive; layer_count + 1 includes the output head */
    om_backend backend;     /* OM_BACKEND_AUTO picks the fastest available backend */
    uint32_t max_tokens;    /* KV cache capacity in tokens, shared by all requests of the shard */
    om_kv_type kv_type;     /* KV cache element type; OM_KV_F16 when zero */
} om_shard_params;

/* The ABI version this library implements (OMNARC_ABI_VERSION at build time). */
uint32_t om_abi_version(void);

/* Library version, e.g. "0.1.0". The string is static. */
const char *om_version_string(void);

/* A short English description of a status code. The string is static. */
const char *om_status_string(om_status status);

/* A GPU this build of the engine can run layers on. */
typedef struct om_device {
    om_backend backend;    /* CUDA, Vulkan, Metal, ... */
    uint64_t memory_bytes; /* its memory as its backend reports it; 0 if unknown */
    char name[128];        /* e.g. "NVIDIA GeForce GTX 1050", NUL-terminated */
} om_device;

/* Lists the GPUs this build of the engine can run layers on: writes up to capacity of them to out and returns how
 * many there are. None means the engine computes on the processor. */
size_t om_devices(om_device *out, size_t capacity);

/* Loads layers [layer_begin, layer_end) of a model. Only the weights of those layers are read; the shard that
 * starts at layer 0 also loads the token embeddings. [layer_count, layer_count + 1) is the output head alone: it
 * turns the last hidden state into logits, so the device that holds the first layers can keep the head too, as
 * prima.cpp's ring does. On success *out receives a shard that must be released with om_shard_close(). */
om_status om_shard_open(const om_shard_params *params, om_shard **out);

/* Runs a middle shard (neither layer 0 nor the output head) on token_count tokens starting at position_start.
 * hidden_in and hidden_out hold token_count × hidden_dim f32 values; they may point to the same buffer.
 * The same as om_shard_run() with hidden input; other shards return OM_ERR_INVALID_ARGUMENT. */
om_status om_shard_forward(om_shard *shard, uint64_t request_id, uint32_t position_start, uint32_t token_count,
                           const float *hidden_in, float *hidden_out);

/* Releases a shard. Passing NULL is allowed. */
void om_shard_close(om_shard *shard);

/* --- ABI version 2 --------------------------------------------------------------------------------------- */

/* Model metadata, read from the GGUF header without loading weights. */
typedef struct om_model_info {
    char architecture[32];   /* e.g. "llama", "qwen3" */
    uint32_t layer_count;    /* repeating layers; the output head is layer layer_count */
    uint32_t hidden_dim;     /* width of the hidden state passed between shards */
    uint32_t vocab_size;     /* logits per output token */
    uint32_t context_length; /* trained context length */
    uint64_t file_size;      /* bytes */
    uint32_t splittable;     /* 1 when the architecture can be split into windows */
} om_model_info;

om_status om_model_info_read(const char *model_path, om_model_info *out);

typedef struct om_shard_info {
    om_model_info model;
    uint32_t layer_begin;
    uint32_t layer_end;
    om_backend backend; /* the backend the shard's layers run on */
} om_shard_info;

om_status om_shard_get_info(const om_shard *shard, om_shard_info *out);

/* The last error message of the calling thread, from the engine or from llama.cpp; "" if there was none.
 * The string stays valid until the next engine call on the same thread. */
const char *om_last_error(void);

/* One step of one request through a shard. */
typedef struct om_batch {
    uint64_t request_id;     /* a request keeps its KV cache in the shard until om_shard_drop() */
    uint32_t position_start; /* position of the first token */
    uint32_t token_count;
    const int32_t *tokens;   /* a shard that starts at layer 0: token_count token ids; NULL otherwise */
    const float *hidden;     /* other shards: token_count × hidden_dim f32 values; NULL otherwise */
    uint32_t all_logits;     /* a shard with the output head: 0 = logits of the last token only, 1 = of every token */
} om_batch;

/* Runs a shard. A shard without the output head writes token_count × hidden_dim hidden-state values to out; a
 * shard with it writes vocab_size logits per output token. out_count is the capacity of out in floats.
 * Returns OM_ERR_CAPACITY when the KV cache or the request slots are full. */
om_status om_shard_run(om_shard *shard, const om_batch *batch, float *out, size_t out_count);

/* Forgets a request and frees its KV cache. Unknown requests are ignored. */
om_status om_shard_drop(om_shard *shard, uint64_t request_id);

/* Removes a request's KV cache from position onwards, e.g. after rejected speculative tokens. */
om_status om_shard_truncate(om_shard *shard, uint64_t request_id, uint32_t position);

/* The bytes om_shard_kv_read needs for this request's KV cache as it stands. The size grows with the answer, so
 * it is asked for again before every read. */
om_status om_shard_kv_size(om_shard *shard, uint64_t request_id, size_t *out_size);

/* Copies a request's KV cache out of the shard. A replica of these layers on another device writes it back with
 * om_shard_kv_write and carries the answer on from the same token, instead of reading the prompt again -- the
 * promise that a device leaving does not interrupt the answer. Writes how many bytes were used. */
om_status om_shard_kv_read(om_shard *shard, uint64_t request_id, void *out, size_t out_size, size_t *out_written);

/* Writes a KV cache taken by om_shard_kv_read into this shard as request_id, replacing what that request held.
 * The shard must hold the same layer window the cache came from, or the engine refuses it. */
om_status om_shard_kv_write(om_shard *shard, uint64_t request_id, const void *data, size_t size);

/* Starts reading the shard's weights from disk in the background and returns at once (prima.cpp's prefetching):
 * a shard whose weights do not stay in memory is read while the other shards of the ring compute, so it is ready
 * when its turn comes. It does nothing when the weights need no prefetching: a GPU holds them, or they fit in free
 * memory when the shard opens, so the operating system keeps them after the first pass. */
om_status om_shard_prefetch(om_shard *shard);

/* Threads for computing on the CPU; 0 = one per physical core (the default). */
om_status om_shard_set_threads(om_shard *shard, uint32_t threads);

/* Text to token ids with the model's tokenizer. *count receives the number of tokens; if it exceeds capacity,
 * nothing is written and OM_ERR_CAPACITY is returned. add_special adds the model's BOS token, if any. */
om_status om_shard_tokenize(const om_shard *shard, const char *text, size_t text_len, uint32_t add_special,
                            int32_t *tokens, size_t capacity, size_t *count);

/* The text of one token, written to out without a terminating zero; *len receives its length in bytes. */
om_status om_shard_token_text(const om_shard *shard, int32_t token, char *out, size_t capacity, size_t *len);

/* 1 if the token ends generation (end of sequence, end of turn), else 0. */
uint32_t om_shard_token_is_end(const om_shard *shard, int32_t token);

/* Picks the next token from logits. Opaque; one per request. */
typedef struct om_sampler om_sampler;

typedef struct om_sampler_params {
    float temperature; /* 0 = greedy: always the most likely token */
    uint32_t top_k;    /* 0 = off */
    float top_p;       /* 1 = off */
    float min_p;       /* 0 = off */
    uint64_t seed;
} om_sampler_params;

om_status om_sampler_open(const om_sampler_params *params, om_sampler **out);

/* Samples from vocab_size logits and returns the token id in *token. */
om_status om_sampler_sample(om_sampler *sampler, const float *logits, uint32_t vocab_size, int32_t *token);

void om_sampler_close(om_sampler *sampler);

#ifdef __cplusplus
}
#endif

#endif /* OMNARC_OMNARC_H */
