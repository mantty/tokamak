#include "wasm_c_api_internal.h"
#include "../interpreter/wasm_runtime.h"
#include <stdlib.h>
#include <string.h>

_Static_assert(sizeof(wasm_val_t) == 16, "Rust value ABI size");
_Static_assert(offsetof(wasm_val_t, of) == 8, "Rust value ABI alignment");

/* The public bridge uses only fixed-width values and opaque handles. */
typedef bool (*TkCallback)(void *, uint32_t, const wasm_val_t *, size_t,
                           wasm_val_t *, size_t);
typedef bool (*TkInterrupted)(void *);
typedef struct TkImport TkImport;
typedef struct {
    void *user;
    TkCallback callback;
    TkInterrupted interrupted;
} TkCall;
#ifdef _MSC_VER
__declspec(thread) static TkCall *active_call;
#else
static _Thread_local TkCall *active_call;
#endif

bool tokamak_wasm_interrupted(void) {
    return active_call && active_call->interrupted(active_call->user);
}

typedef struct {
    wasm_store_t *store;
    wasm_module_t *module;
    wasm_importtype_vec_t imports;
    wasm_exporttype_vec_t exports;
    wasm_extern_vec_t supplied;
    wasm_extern_vec_t values;
    wasm_instance_t *instance;
    TkImport *environments;
    char error[512];
} TkModule;

struct TkImport {
    TkModule *module;
    uint32_t index;
};

typedef struct {
    const char *name;
    size_t name_len;
    const char *module;
    size_t module_len;
    uint32_t kind;
    uint32_t params;
    uint32_t results;
    uint8_t value_kind;
    uint32_t mutable_value;
} TkType;

void *tk_wasm_engine(void) { return wasm_engine_new(); }

void tk_wasm_delete(TkModule *module) {
    if (!module) return;
    wasm_extern_vec_delete(&module->values);
    wasm_extern_vec_delete(&module->supplied);
    wasm_importtype_vec_delete(&module->imports);
    wasm_exporttype_vec_delete(&module->exports);
    wasm_store_delete(module->store);
    free(module->environments);
    free(module);
}

TkModule *tk_wasm_module(void *engine, const uint8_t *bytes, size_t length) {
    TkModule *module = calloc(1, sizeof(TkModule));
    if (!module) return NULL;
    module->store = wasm_store_new(engine);
    wasm_byte_vec_t binary = { length, (char *)bytes, length, 1, NULL };
    if (module->store) module->module = wasm_module_new(module->store, &binary);
    if (!module->module) {
        tk_wasm_delete(module);
        return NULL;
    }
    wasm_module_imports(module->module, &module->imports);
    wasm_module_exports(module->module, &module->exports);
    return module;
}

size_t tk_wasm_count(TkModule *module, bool imports) {
    return imports ? module->imports.num_elems : module->exports.num_elems;
}

static const wasm_externtype_t *type_at(TkModule *module, bool imports, size_t index) {
    return imports ? wasm_importtype_type(module->imports.data[index])
                   : wasm_exporttype_type(module->exports.data[index]);
}

TkType tk_wasm_type(TkModule *module, bool imports, size_t index) {
    TkType out = {0};
    const wasm_name_t *name = imports
        ? wasm_importtype_name(module->imports.data[index])
        : wasm_exporttype_name(module->exports.data[index]);
    out.name = name->data;
    out.name_len = name->num_elems;
    if (imports) {
        const wasm_name_t *namespace = wasm_importtype_module(module->imports.data[index]);
        out.module = namespace->data;
        out.module_len = namespace->num_elems;
    }
    const wasm_externtype_t *type = type_at(module, imports, index);
    out.kind = wasm_externtype_kind(type);
    if (out.kind == WASM_EXTERN_FUNC) {
        const wasm_functype_t *function = wasm_externtype_as_functype_const(type);
        out.params = wasm_functype_params(function)->num_elems;
        out.results = wasm_functype_results(function)->num_elems;
    }
    if (out.kind == WASM_EXTERN_GLOBAL) {
        const wasm_globaltype_t *global = wasm_externtype_as_globaltype_const(type);
        out.value_kind = wasm_valtype_kind(wasm_globaltype_content(global));
        out.mutable_value = wasm_globaltype_mutability(global);
    }
    return out;
}

uint8_t tk_wasm_value_kind(TkModule *module, bool imports, size_t index,
                           bool results, size_t argument) {
    const wasm_functype_t *type = wasm_externtype_as_functype_const(type_at(module, imports, index));
    const wasm_valtype_vec_t *types = results ? wasm_functype_results(type) : wasm_functype_params(type);
    return wasm_valtype_kind(types->data[argument]);
}

static bool take_trap(TkModule *module, wasm_trap_t *trap) {
    if (!trap) return true;
    wasm_message_t message = {0};
    wasm_trap_message(trap, &message);
    snprintf(module->error, sizeof(module->error), "%.*s", (int)message.num_elems, message.data);
    wasm_byte_vec_delete(&message);
    wasm_trap_delete(trap);
    return false;
}

static wasm_trap_t *invoke_import(void *environment, const wasm_val_vec_t *args, wasm_val_vec_t *results) {
    TkImport *import = environment;
    if (active_call && active_call->callback(active_call->user, import->index,
        args->data, args->num_elems, results->data, results->size)) {
        results->num_elems = results->size;
        return NULL;
    }
    wasm_message_t message;
    wasm_name_new_from_string(&message, "JavaScript import threw an exception");
    wasm_trap_t *trap = wasm_trap_new(import->module->store, &message);
    wasm_byte_vec_delete(&message);
    return trap;
}

bool tk_wasm_instantiate(TkModule *module, const wasm_val_t *globals,
                         void *user, TkCallback callback, TkInterrupted interrupted) {
    wasm_extern_vec_new_uninitialized(&module->supplied, module->imports.num_elems);
    if (module->imports.num_elems && !module->supplied.data) return false;
    module->environments = calloc(module->imports.num_elems, sizeof(TkImport));
    if (module->imports.num_elems && !module->environments) return false;
    for (size_t i = 0; i < module->imports.num_elems; ++i) {
        const wasm_externtype_t *type = type_at(module, true, i);
        if (wasm_externtype_kind(type) == WASM_EXTERN_FUNC) {
            TkImport *environment = &module->environments[i];
            *environment = (TkImport){module, (uint32_t)i};
            wasm_func_t *function = wasm_func_new_with_env(module->store,
                wasm_externtype_as_functype_const(type), invoke_import, environment, NULL);
            if (!function) return false;
            module->supplied.data[i] = wasm_func_as_extern(function);
        } else if (wasm_externtype_kind(type) == WASM_EXTERN_GLOBAL) {
            module->supplied.data[i] = wasm_global_as_extern(wasm_global_new(module->store,
                wasm_externtype_as_globaltype_const(type), &globals[i]));
        } else {
            snprintf(module->error, sizeof(module->error), "WAMR does not support imported memories or tables");
            return false;
        }
        if (!module->supplied.data[i]) return false;
        module->supplied.num_elems = i + 1;
    }
    TkCall call = {user, callback, interrupted}, *previous = active_call;
    active_call = &call;
    wasm_trap_t *trap = NULL;
    module->instance = wasm_instance_new_with_args(module->store, module->module,
        &module->supplied, &trap, 256 * 1024, 0);
    active_call = previous;
    if (!take_trap(module, trap) || !module->instance) return false;
    wasm_instance_exports(module->instance, &module->values);
    return true;
}

const char *tk_wasm_error(TkModule *module) { return module->error; }

bool tk_wasm_call(TkModule *module, size_t index, const wasm_val_t *arguments,
                  size_t argc, wasm_val_t *results, size_t resultc,
                  void *user, TkCallback callback, TkInterrupted interrupted) {
    wasm_val_vec_t args = {argc, (wasm_val_t *)arguments, argc, sizeof(wasm_val_t), NULL};
    wasm_val_vec_t out = {resultc, results, resultc, sizeof(wasm_val_t), NULL};
    TkCall call = {user, callback, interrupted}, *previous = active_call;
    active_call = &call;
    wasm_trap_t *trap = wasm_func_call(wasm_extern_as_func(module->values.data[index]), &args, &out);
    active_call = previous;
    return take_trap(module, trap);
}

uint8_t *tk_wasm_memory(TkModule *module, size_t index, size_t *length, uint64_t *generation) {
    wasm_memory_t *memory = wasm_extern_as_memory(module->values.data[index]);
    *length = wasm_memory_data_size(memory);
    *generation = ((WASMModuleInstance *)memory->inst_comm_rt)->memories[memory->memory_idx_rt]->growth_generation;
    return (uint8_t *)wasm_memory_data(memory);
}

int64_t tk_wasm_memory_grow(TkModule *module, size_t index, uint32_t delta) {
    wasm_memory_t *memory = wasm_extern_as_memory(module->values.data[index]);
    uint32_t previous = wasm_memory_size(memory);
    return wasm_enlarge_memory((WASMModuleInstance *)memory->inst_comm_rt, delta) ? (int64_t)previous : -1;
}

wasm_val_t tk_wasm_global_get(TkModule *module, size_t index) {
    wasm_val_t value = {0};
    wasm_global_get(wasm_extern_as_global(module->values.data[index]), &value);
    return value;
}
void tk_wasm_global_set(TkModule *module, size_t index, const wasm_val_t *value) {
    wasm_global_set(wasm_extern_as_global(module->values.data[index]), value);
}
