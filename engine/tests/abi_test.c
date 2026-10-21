/* Calls the engine from plain C, the way every foreign-function interface will. */
#include <stdio.h>
#include <string.h>

#include "omnarc/omnarc.h"

static int failures = 0;
#define CHECK(cond)                                                       \
    do {                                                                  \
        if (!(cond)) {                                                    \
            fprintf(stderr, "%s:%d: check failed: %s\n", __FILE__, __LINE__, #cond); \
            failures++;                                                   \
        }                                                                 \
    } while (0)

int main(void) {
    CHECK(om_abi_version() == OMNARC_ABI_VERSION);
    CHECK(OMNARC_ABI_VERSION == 5u);
    CHECK(strcmp(om_version_string(), "0.1.0") == 0);
    CHECK(strcmp(om_status_string(OM_OK), "ok") == 0);
    CHECK(strcmp(om_status_string(OM_ERR_NOT_IMPLEMENTED), "not implemented") == 0);
    CHECK(strcmp(om_status_string(OM_ERR_CAPACITY), "capacity exhausted") == 0);

    /* the GPUs of this build: the count alone, then the devices; a build for the processor has none */
    size_t gpus = om_devices(NULL, 0);
    om_device devices[8];
    size_t listed = om_devices(devices, 8);
    CHECK(listed == gpus);
    for (size_t i = 0; i < listed && i < 8; i++) {
        CHECK(devices[i].backend != OM_BACKEND_CPU && devices[i].backend != OM_BACKEND_AUTO);
        CHECK(strlen(devices[i].name) > 0);
    }

    om_shard *shard = (om_shard *)1;
    CHECK(om_shard_open(NULL, &shard) == OM_ERR_INVALID_ARGUMENT);
    CHECK(shard == NULL);
    CHECK(om_shard_open(NULL, NULL) == OM_ERR_INVALID_ARGUMENT);
    CHECK(strlen(om_last_error()) > 0);

    om_shard_params params = {"no-such-model.gguf", 10, 10, OM_BACKEND_AUTO, 4096, OM_KV_F16};
    CHECK(om_shard_open(&params, &shard) == OM_ERR_INVALID_ARGUMENT); /* empty layer range */
    params.layer_end = 24;
    params.max_tokens = 0;
    CHECK(om_shard_open(&params, &shard) == OM_ERR_INVALID_ARGUMENT); /* no KV capacity */
    params.max_tokens = 4096;
    params.backend = (om_backend)42;
    CHECK(om_shard_open(&params, &shard) == OM_ERR_INVALID_ARGUMENT); /* unknown backend */
    params.backend = OM_BACKEND_AUTO;
    params.kv_type = (om_kv_type)7;
    CHECK(om_shard_open(&params, &shard) == OM_ERR_INVALID_ARGUMENT); /* unknown KV cache type */
    params.kv_type = OM_KV_Q8_0;
    CHECK(om_shard_open(&params, &shard) == OM_ERR_IO); /* valid arguments, no such file */
    CHECK(strstr(om_last_error(), "no-such-model.gguf") != NULL);

    om_model_info info;
    CHECK(om_model_info_read(NULL, &info) == OM_ERR_INVALID_ARGUMENT);
    CHECK(om_model_info_read("no-such-model.gguf", &info) == OM_ERR_IO);

    float buffer[4] = {0};
    CHECK(om_shard_forward(NULL, 1, 0, 1, buffer, buffer) == OM_ERR_INVALID_ARGUMENT);
    CHECK(om_shard_run(NULL, NULL, buffer, 4) == OM_ERR_INVALID_ARGUMENT);
    CHECK(om_shard_drop(NULL, 1) == OM_ERR_INVALID_ARGUMENT);
    size_t kv_size = 0;
    CHECK(om_shard_kv_size(NULL, 1, &kv_size) == OM_ERR_INVALID_ARGUMENT);
    CHECK(om_shard_kv_read(NULL, 1, buffer, sizeof buffer, &kv_size) == OM_ERR_INVALID_ARGUMENT);
    CHECK(om_shard_kv_write(NULL, 1, buffer, sizeof buffer) == OM_ERR_INVALID_ARGUMENT);
    om_shard_close(NULL);

    /* greedy sampling picks the largest logit */
    om_sampler_params sp = {0.0f, 0, 1.0f, 0.0f, 7};
    om_sampler *sampler = NULL;
    CHECK(om_sampler_open(&sp, &sampler) == OM_OK);
    const float logits[5] = {0.1f, 2.5f, -1.0f, 2.4f, 0.0f};
    int32_t token = -1;
    CHECK(om_sampler_sample(sampler, logits, 5, &token) == OM_OK);
    CHECK(token == 1);
    om_sampler_close(sampler);

    /* with a temperature, a seeded sampler is reproducible */
    sp.temperature = 0.8f;
    sp.top_k = 3;
    int32_t first = -1, second = -1;
    CHECK(om_sampler_open(&sp, &sampler) == OM_OK);
    CHECK(om_sampler_sample(sampler, logits, 5, &first) == OM_OK);
    om_sampler_close(sampler);
    CHECK(om_sampler_open(&sp, &sampler) == OM_OK);
    CHECK(om_sampler_sample(sampler, logits, 5, &second) == OM_OK);
    om_sampler_close(sampler);
    CHECK(first == second);
    CHECK(first == 0 || first == 1 || first == 3); /* top-3 */

    if (failures) {
        fprintf(stderr, "%d check(s) failed\n", failures);
        return 1;
    }
    puts("libomnarc C ABI: all checks passed");
    return 0;
}
