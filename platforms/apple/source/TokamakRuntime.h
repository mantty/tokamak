#ifndef TOKAMAK_RUNTIME_H
#define TOKAMAK_RUNTIME_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

enum {
  TOKAMAK_DECISION_DEFAULT = 0,
  TOKAMAK_DECISION_CANCEL = 1,
  TOKAMAK_DECISION_USE = 2,
};

typedef struct {
  uint8_t *data;
  size_t len;
} TokamakBytes;

typedef struct {
  TokamakBytes certificate;
  TokamakBytes private_key;
} TokamakIdentity;

typedef void (*TokamakPluginRequest)(void *context, uint64_t id,
                                     const char *plugin, const char *method,
                                     const char *arguments);

typedef struct {
  void *context;
  TokamakPluginRequest call;
  TokamakPluginRequest subscribe;
  void (*unsubscribe)(void *context, uint64_t id);
} TokamakPluginHandler;

void *tokamak_runtime_start(const char *packaged_dir, const char *state_dir,
                            const char *storage_dir, const char *host,
                            bool foreground, TokamakPluginHandler plugins,
                            char *error, size_t error_len);
void *tokamak_runtime_start_development(const char *state_dir, const char *host,
                                     const char *endpoint,
                                     const char *session_token,
                                     bool foreground,
                                     TokamakPluginHandler plugins,
                                     char *error, size_t error_len);
uint16_t tokamak_runtime_port(const void *runtime);
uint16_t tokamak_runtime_restore_gateway(const void *runtime, char *error,
                                      size_t error_len);
void tokamak_runtime_stop(void *runtime);
bool tokamak_runtime_emit(const void *runtime, const char *name,
                          const char *event, uint64_t timeout_ms,
                          TokamakBytes *reply, char *error, size_t error_len);
bool tokamak_runtime_fetch(const void *runtime, const char *path,
                           uint64_t timeout_ms, TokamakBytes *body, char *error,
                           size_t error_len);
bool tokamak_runtime_set_foreground(const void *runtime, bool foreground);
bool tokamak_runtime_is_foreground(const void *runtime);
void tokamak_runtime_reply(const void *runtime, uint64_t id, const char *result);

int32_t tokamak_runtime_server_authority(const void *runtime, const char *host,
                                      TokamakBytes *authority);
int32_t tokamak_runtime_client_identity(const void *runtime, const char *host,
                                     size_t previous_failures,
                                     TokamakIdentity *identity);
void tokamak_bytes_free(TokamakBytes bytes);
void tokamak_identity_free(TokamakIdentity identity);

#endif
