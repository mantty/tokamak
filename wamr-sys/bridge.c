#include "tokamak.h"
#include "wasm_c_api_internal.h"
#include <stdlib.h>
#include <string.h>

_Static_assert(sizeof(wasm_val_t) == 16, "Rust value ABI size");
_Static_assert(offsetof(wasm_val_t, of) == 8, "Rust value ABI alignment");
#define NONE UINT32_MAX

typedef struct TkModule TkModule;
typedef struct TkRuntime TkRuntime;
typedef bool (*TkCallback)(void *, TkModule *, uint32_t, const wasm_val_t *,
                           size_t, wasm_val_t *, size_t);
typedef bool (*TkInterrupted)(void *);
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
bool
tokamak_wasm_interrupted(void)
{
    return active_call && active_call->interrupted(active_call->user);
}

typedef struct {
    uint32_t kind;
    uint8_t value_kind;
    bool mutable_value;
    uint32_t maximum;
    TkModule *owner;
    uint32_t index;
    union {
        WASMMemoryInstance *memory;
        WASMTableInstance *table;
        uint8_t *global;
        WASMFuncType *function;
    } of;
    uint64_t storage;
} TkObject;
typedef struct {
    TkModule *module;
    uint32_t index, target;
} TkImport;
struct TkModule {
    TkRuntime *runtime;
    TkModule *next;
    wasm_store_t *store;
    wasm_module_t *module;
    wasm_importtype_vec_t imports;
    wasm_exporttype_vec_t exports;
    WASMModuleInstance *native;
    TkImport *environments;
    uint32_t *functions, *memories, *tables, *globals;
    bool link_error;
};
struct TkRuntime {
    void *engine;
    TkModule *modules;
    TkObject **objects;
    size_t count, capacity;
    char error[512];
};
typedef struct {
    const char *name;
    size_t name_len;
    const char *module;
    size_t module_len;
    uint32_t kind, params, results;
    uint8_t value_kind;
    uint32_t mutable_value;
    uint32_t function_index;
} TkType;

static WASMModule *
native_module(TkModule *module)
{
    return (WASMModule *)*module->module;
}
static uint8_t
value_kind(uint8_t type)
{
    switch (type) {
        case VALUE_TYPE_I32:
            return WASM_I32;
        case VALUE_TYPE_I64:
            return WASM_I64;
        case VALUE_TYPE_F32:
            return WASM_F32;
        case VALUE_TYPE_F64:
            return WASM_F64;
        case VALUE_TYPE_V128:
            return WASM_V128;
        case VALUE_TYPE_EXTERNREF:
            return WASM_EXTERNREF;
        default:
            return WASM_FUNCREF;
    }
}
static size_t
value_size(uint8_t kind)
{
    return kind == WASM_I64 || kind == WASM_F64 ? 8 : 4;
}
static TkObject *
object_at(TkRuntime *runtime, uint32_t id)
{
    return id < runtime->count ? runtime->objects[id] : NULL;
}
static uint32_t
add_object(TkRuntime *runtime, TkObject *object)
{
    if (!object)
        return NONE;
    if (runtime->count == runtime->capacity) {
        size_t capacity = runtime->capacity ? runtime->capacity * 2 : 16;
        if (capacity > UINT32_MAX) {
            free(object);
            return NONE;
        }
        void *objects =
            realloc(runtime->objects, capacity * sizeof(TkObject *));
        if (!objects) {
            free(object);
            return NONE;
        }
        runtime->objects = objects;
        runtime->capacity = capacity;
    }
    runtime->objects[runtime->count] = object;
    return (uint32_t)runtime->count++;
}
void *
tk_wasm_engine(void)
{
    return wasm_engine_new();
}
TkRuntime *
tk_wasm_runtime(void *engine)
{
    TkRuntime *runtime = calloc(1, sizeof(TkRuntime));
    if (runtime)
        runtime->engine = engine;
    return runtime;
}
static void
delete_module(TkModule *module)
{
    if (module->native)
        wasm_runtime_deinstantiate((WASMModuleInstanceCommon *)module->native);
    wasm_importtype_vec_delete(&module->imports);
    wasm_exporttype_vec_delete(&module->exports);
    wasm_store_delete(module->store);
    free(module->environments);
    free(module->functions);
    free(module->memories);
    free(module->tables);
    free(module->globals);
    free(module);
}
void
tk_wasm_runtime_delete(TkRuntime *runtime)
{
    if (!runtime)
        return;
    for (TkModule *module = runtime->modules, *next; module; module = next) {
        next = module->next;
        delete_module(module);
    }
    for (size_t i = 0; i < runtime->count; i++) {
        TkObject *object = runtime->objects[i];
        if (object->kind == WASM_EXTERN_TABLE) {
            free(object->of.table->host_elems);
            free(object->of.table);
        }
        free(object);
    }
    free(runtime->objects);
    free(runtime);
}
TkModule *
tk_wasm_module(TkRuntime *runtime, const uint8_t *bytes, size_t length)
{
    TkModule *module = calloc(1, sizeof(TkModule));
    if (!module)
        return NULL;
    module->runtime = runtime;
    module->store = wasm_store_new(runtime->engine);
    wasm_byte_vec_t binary = { length, (char *)bytes, length, 1, NULL };
    if (module->store)
        module->module = wasm_module_new(module->store, &binary);
    if (!module->module) {
        delete_module(module);
        return NULL;
    }
    native_module(module)->tokamak_context = module;
    wasm_module_imports(module->module, &module->imports);
    wasm_module_exports(module->module, &module->exports);
    WASMModule *native = native_module(module);
    wasm_importtype_t **ordered =
        calloc(native->import_count + 1, sizeof(wasm_importtype_t *));
    if (!ordered) {
        delete_module(module);
        return NULL;
    }
    uint32_t offsets[] = {
        0,
        native->import_function_count + native->import_global_count
            + native->import_memory_count,
        native->import_function_count + native->import_global_count,
        native->import_function_count
    };
    for (uint32_t i = 0; i < native->import_count; i++) {
        WASMImport *import = native->imports + i;
        ordered[import->source_index] =
            module->imports.data[offsets[import->kind]++];
    }
    if (native->import_count)
        memcpy(module->imports.data, ordered,
               native->import_count * sizeof(*ordered));
    free(ordered);
    module->next = runtime->modules;
    runtime->modules = module;
    return module;
}
size_t
tk_wasm_count(TkModule *module, bool imports)
{
    return imports ? module->imports.num_elems : module->exports.num_elems;
}

static const wasm_externtype_t *
type_at(TkModule *module, bool imports, size_t index)
{
    return imports ? wasm_importtype_type(module->imports.data[index])
                   : wasm_exporttype_type(module->exports.data[index]);
}

TkType
tk_wasm_type(TkModule *module, bool imports, size_t index)
{
    TkType out = { 0 };
    const wasm_name_t *name =
        imports ? wasm_importtype_name(module->imports.data[index])
                : wasm_exporttype_name(module->exports.data[index]);
    out.name = name->data;
    out.name_len = name->num_elems;
    if (imports) {
        const wasm_name_t *namespace =
            wasm_importtype_module(module->imports.data[index]);
        out.module = namespace->data;
        out.module_len = namespace->num_elems;
    }
    const wasm_externtype_t *type = type_at(module, imports, index);
    out.kind = wasm_externtype_kind(type);
    if (out.kind == WASM_EXTERN_FUNC) {
        const wasm_functype_t *function =
            wasm_externtype_as_functype_const(type);
        out.params = wasm_functype_params(function)->num_elems;
        out.results = wasm_functype_results(function)->num_elems;
    }
    if (out.kind == WASM_EXTERN_GLOBAL) {
        const wasm_globaltype_t *global =
            wasm_externtype_as_globaltype_const(type);
        out.value_kind = wasm_valtype_kind(wasm_globaltype_content(global));
        out.mutable_value = wasm_globaltype_mutability(global);
    }
    return out;
}

uint8_t
tk_wasm_value_kind(TkModule *module, bool imports, size_t index, bool results,
                   size_t argument)
{
    const wasm_functype_t *type =
        wasm_externtype_as_functype_const(type_at(module, imports, index));
    const wasm_valtype_vec_t *types =
        results ? wasm_functype_results(type) : wasm_functype_params(type);
    return wasm_valtype_kind(types->data[argument]);
}

static WASMFuncType *
function_type(TkModule *module, uint32_t index)
{
    WASMModule *native = native_module(module);
    return index < native->import_function_count
               ? native->import_functions[index].u.function.func_type
               : native->functions[index - native->import_function_count]
                     ->func_type;
}
bool
tokamak_same_function_type(const WASMFuncType *a, const WASMFuncType *b)
{
    return a->param_count == b->param_count
           && a->result_count == b->result_count
           && !memcmp(a->types, b->types, a->param_count + a->result_count);
}
void
tokamak_bind_instance(WASMModuleInstance *instance)
{
    TkModule *module = instance->module->tokamak_context;
    if (module)
        module->native = instance;
}
uint32_t
tokamak_function_ref(WASMModuleInstance *instance, uint32_t index)
{
    TkModule *module = instance->module->tokamak_context;
    return module && index != NONE ? module->functions[index] : index;
}
WASMFunctionInstance *
tokamak_resolve_function(WASMModuleInstance *instance, uint32_t reference)
{
    TkModule *module = instance->module->tokamak_context;
    if (!module)
        return reference < instance->e->function_count
                   ? instance->e->functions + reference
                   : NULL;
    TkObject *object = object_at(module->runtime, reference);
    if (!object || object->kind != WASM_EXTERN_FUNC || !object->owner->native
        || !object->owner->native->e->functions)
        return NULL;
    WASMFunctionInstance *function =
        object->owner->native->e->functions + object->index;
    function->host_owner = object->owner->native;
    return function;
}
WASMMemoryInstance *
tokamak_import_memory(const WASMModule *native, uint32_t index)
{
    TkModule *module = native->tokamak_context;
    return module->runtime->objects[module->memories[index]]->of.memory;
}
uint8_t *
tokamak_import_global(const WASMModule *native, uint32_t index)
{
    TkModule *module = native->tokamak_context;
    return module ? module->runtime->objects[module->globals[index]]->of.global
                  : NULL;
}
uint32_t
tk_wasm_new_global(TkRuntime *runtime, uint8_t kind, bool mutable_value,
                   const wasm_val_t *value)
{
    TkObject *object = calloc(1, sizeof(TkObject));
    if (!object)
        return NONE;
    object->kind = WASM_EXTERN_GLOBAL;
    object->value_kind = kind;
    object->mutable_value = mutable_value;
    object->of.global = (uint8_t *)&object->storage;
    memcpy(object->of.global, &value->of, value_size(kind));
    return add_object(runtime, object);
}
uint32_t
tk_wasm_new_table(TkRuntime *runtime, uint8_t kind, uint32_t initial,
                  uint32_t maximum, uint32_t value)
{
    TkObject *object = calloc(1, sizeof(TkObject));
    WASMTableInstance *table = calloc(1, sizeof(WASMTableInstance));
    table_elem_type_t *elements =
        malloc((size_t)(initial ? initial : 1) * sizeof(table_elem_type_t));
    if (!object || !table || !elements) {
        free(object);
        free(table);
        free(elements);
        return NONE;
    }
    for (uint32_t i = 0; i < initial; i++)
        elements[i] = value;
    table->elem_type =
        kind == WASM_FUNCREF ? VALUE_TYPE_FUNCREF : VALUE_TYPE_EXTERNREF;
    table->cur_size = initial;
    table->max_size = maximum;
    table->host_elems = elements;
    object->kind = WASM_EXTERN_TABLE;
    object->value_kind = kind;
    object->of.table = table;
    object->maximum = maximum;
    uint32_t id = add_object(runtime, object);
    if (id == NONE) {
        free(elements);
        free(table);
    }
    return id;
}
bool
tokamak_grow_table(WASMTableInstance *table, uint32_t delta,
                   table_elem_type_t value)
{
    if (delta > table->max_size - table->cur_size
        || delta > UINT32_MAX - table->cur_size)
        return false;
    if (!delta)
        return true;
    uint32_t size = table->cur_size + delta;
    void *elements =
        realloc(table->host_elems, (size_t)size * sizeof(table_elem_type_t));
    if (!elements)
        return false;
    table->host_elems = elements;
    for (uint32_t i = table->cur_size; i < size; i++)
        table->host_elems[i] = value;
    table->cur_size = size;
    return true;
}
WASMTableInstance **
tokamak_instantiate_tables(WASMModuleInstance *instance, char *error,
                           uint32_t length)
{
    TkModule *module = instance->module->tokamak_context;
    WASMModule *native = instance->module;
    uint32_t count = native->import_table_count + native->table_count;
    WASMTableInstance **tables =
        wasm_runtime_malloc((uint64_t)count * sizeof(WASMTableInstance *));
    if (!tables)
        return NULL;
    for (uint32_t i = 0; i < count; i++) {
        if (i >= native->import_table_count) {
            WASMTableType *type =
                &native->tables[i - native->import_table_count].table_type;
            uint32_t maximum = type->flags & 1 ? type->max_size : NONE;
            module->tables[i] =
                tk_wasm_new_table(module->runtime, value_kind(type->elem_type),
                                  type->init_size, maximum, NONE);
        }
        TkObject *object = object_at(module->runtime, module->tables[i]);
        if (!object) {
            snprintf(error, length, "allocate table failed");
            wasm_runtime_free(tables);
            return NULL;
        }
        tables[i] = object->of.table;
    }
    return tables;
}
static uint32_t *
indices(uint32_t count)
{
    uint32_t *values = malloc((size_t)(count ? count : 1) * sizeof(uint32_t));
    if (values)
        memset(values, 0xff, (size_t)count * sizeof(uint32_t));
    return values;
}
static bool
prepare_objects(TkModule *module, const uint32_t *imports)
{
    WASMModule *native = native_module(module);
    uint32_t counts[4] = { native->import_function_count
                               + native->function_count,
                           native->import_global_count + native->global_count,
                           native->import_table_count + native->table_count,
                           native->import_memory_count + native->memory_count };
    uint32_t **arrays[4] = { &module->functions, &module->globals,
                             &module->tables, &module->memories };
    uint32_t offsets[4] = { 0 };
    for (uint32_t kind = 0; kind < 4; kind++) {
        *arrays[kind] = indices(counts[kind]);
        if (!*arrays[kind])
            return false;
    }
    for (size_t i = 0; i < module->imports.num_elems; i++) {
        const wasm_externtype_t *type = type_at(module, true, i);
        uint32_t kind = wasm_externtype_kind(type), index = offsets[kind]++;
        TkObject *object = object_at(module->runtime, imports[i]);
        if (kind == WASM_EXTERN_FUNC && imports[i] == NONE)
            continue;
        bool matches = object && object->kind == kind;
        if (matches && kind == WASM_EXTERN_FUNC)
            matches = tokamak_same_function_type(object->of.function,
                                                 function_type(module, index));
        if (matches && kind == WASM_EXTERN_GLOBAL) {
            const wasm_globaltype_t *global =
                wasm_externtype_as_globaltype_const(type);
            matches = object->value_kind
                          == wasm_valtype_kind(wasm_globaltype_content(global))
                      && object->mutable_value
                             == (wasm_globaltype_mutability(global) != 0);
        }
        if (matches
            && (kind == WASM_EXTERN_TABLE || kind == WASM_EXTERN_MEMORY)) {
            const wasm_limits_t *limits =
                kind == WASM_EXTERN_TABLE
                    ? wasm_tabletype_limits(
                          wasm_externtype_as_tabletype_const(type))
                    : wasm_memorytype_limits(
                          wasm_externtype_as_memorytype_const(type));
            uint32_t size = kind == WASM_EXTERN_TABLE
                                ? object->of.table->cur_size
                                : object->of.memory->cur_page_count;
            bool has_maximum =
                kind == WASM_EXTERN_TABLE
                    ? (native->import_tables[index].u.table.table_type.flags
                       & 1)
                          != 0
                    : (native->import_memories[index].u.memory.mem_type.flags
                       & 1)
                          != 0;
            matches = size >= limits->min
                      && (!has_maximum || object->maximum <= limits->max);
            if (matches && kind == WASM_EXTERN_MEMORY)
                matches =
                    object->of.memory->is_shared_memory
                    == ((native->import_memories[index].u.memory.mem_type.flags
                         & 2)
                        != 0);
            if (matches && kind == WASM_EXTERN_TABLE)
                matches = object->value_kind
                          == wasm_valtype_kind(wasm_tabletype_element(
                              wasm_externtype_as_tabletype_const(type)));
        }
        if (!matches) {
            module->link_error = true;
            snprintf(module->runtime->error, sizeof(module->runtime->error),
                     "WebAssembly import type or limits mismatch");
            return false;
        }
        (*arrays[kind])[index] = imports[i];
    }
    for (uint32_t i = 0; i < counts[0]; i++) {
        if (module->functions[i] != NONE)
            continue;
        TkObject *object = calloc(1, sizeof(TkObject));
        if (!object)
            return false;
        object->kind = WASM_EXTERN_FUNC;
        object->owner = module;
        object->index = i;
        object->of.function = function_type(module, i);
        module->functions[i] = add_object(module->runtime, object);
        if (module->functions[i] == NONE)
            return false;
    }
    return true;
}
static bool
call_cells(TkObject *object, uint32_t *cells);
static void
native_marker(void)
{
}
bool
tokamak_prepare_instance(WASMModuleInstance *instance)
{
    TkModule *module = instance->module->tokamak_context;
    if (!module)
        return true;
    uint32_t function_index = instance->module->import_function_count;
    if (!wasm_runtime_create_exec_env_singleton(
            (WASMModuleInstanceCommon *)instance))
        return false;
    instance->c_api_func_imports = wasm_runtime_malloc(
        (uint64_t)(function_index + 1) * sizeof(CApiFuncImport));
    if (!instance->c_api_func_imports)
        return false;
    for (uint32_t i = 0; i < function_index; i++) {
        instance->c_api_func_imports[i] = (CApiFuncImport){ 0 };
        instance->c_api_func_imports[i].func_ptr_linked = native_marker;
        instance->c_api_func_imports[i].with_env_arg = true;
        instance->c_api_func_imports[i].env_arg = module->environments + i;
    }
    return true;
}
bool
tk_wasm_instantiate(TkModule *module, const uint32_t *imports, void *user,
                    TkCallback callback, TkInterrupted interrupted)
{
    snprintf(module->runtime->error, sizeof(module->runtime->error),
             "WebAssembly allocation failed");
    if (!prepare_objects(module, imports))
        return false;
    WASMModule *native = native_module(module);
    module->environments =
        calloc(module->imports.num_elems + 1, sizeof(TkImport));
    if (!module->environments)
        return false;
    uint32_t function_index = 0;
    for (size_t i = 0; i < module->imports.num_elems; i++) {
        if (wasm_externtype_kind(type_at(module, true, i)) != WASM_EXTERN_FUNC)
            continue;
        module->environments[function_index] =
            (TkImport){ module, (uint32_t)i, imports[i] };
        WASMFunctionImport *import =
            &native->import_functions[function_index++].u.function;
        import->func_ptr_linked = native_marker;
        import->call_conv_wasm_c_api = true;
    }
    for (uint32_t i = 0; i < native->import_global_count; i++)
        native->import_globals[i].u.global.is_linked = true;
    WASMModuleInstance *instance =
        (WASMModuleInstance *)wasm_runtime_instantiate(
            *module->module, 256 * 1024, 0, module->runtime->error,
            sizeof(module->runtime->error));
    if (!instance)
        return false;
    TkCall call = { user, callback, interrupted }, *previous = active_call;
    active_call = &call;
    bool ok = true;
    if (native->start_function != NONE) {
        uint32_t cells[1] = { 0 };
        ok = call_cells(
            module->runtime->objects[module->functions[native->start_function]],
            cells);
        if (!ok)
            snprintf(module->runtime->error, sizeof(module->runtime->error),
                     "%s", wasm_get_exception(instance));
    }
    active_call = previous;
    return ok;
}
bool
tk_wasm_link_error(TkModule *module)
{
    return module->link_error;
}
const char *
tk_wasm_error(TkRuntime *runtime)
{
    return runtime->error;
}
uint32_t
tk_wasm_export(TkModule *module, size_t index)
{
    WASMModule *native = native_module(module);
    WASMExport *export = native->exports + index;
    const uint32_t kinds[] = { WASM_EXTERN_FUNC, WASM_EXTERN_TABLE,
                               WASM_EXTERN_MEMORY, WASM_EXTERN_GLOBAL };
    uint32_t kind = kinds[export->kind], local = export->index;
    uint32_t *ids[] = { module->functions, module->globals, module->tables,
                        module->memories };
    if (ids[kind][local] != NONE)
        return ids[kind][local];
    TkObject *object = calloc(1, sizeof(TkObject));
    if (!object)
        return NONE;
    object->kind = kind;
    object->owner = module;
    object->index = local;
    if (kind == WASM_EXTERN_MEMORY) {
        object->of.memory = module->native->memories[local];
        WASMMemory *memory =
            native->memories + local - native->import_memory_count;
        object->maximum = memory->flags & 1 ? memory->max_page_count : NONE;
    }
    else if (kind == WASM_EXTERN_GLOBAL) {
        WASMGlobalInstance *global = module->native->e->globals + local;
        object->value_kind = value_kind(global->type);
        object->mutable_value = global->is_mutable;
        object->of.global = module->native->global_data + global->data_offset;
    }
    return ids[kind][local] = add_object(module->runtime, object);
}
TkType
tk_wasm_object_type(TkRuntime *runtime, uint32_t id)
{
    TkType type = { 0 };
    TkObject *object = object_at(runtime, id);
    if (!object) {
        type.kind = NONE;
        return type;
    }
    type.kind = object->kind;
    type.value_kind = object->value_kind;
    type.mutable_value = object->mutable_value;
    type.function_index = object->index;
    if (object->kind == WASM_EXTERN_FUNC) {
        type.params = object->of.function->param_count;
        type.results = object->of.function->result_count;
    }
    return type;
}
uint8_t
tk_wasm_object_value_kind(TkRuntime *runtime, uint32_t id, bool results,
                          size_t argument)
{
    WASMFuncType *type = runtime->objects[id]->of.function;
    return value_kind(
        type->types[(results ? type->param_count : 0) + argument]);
}
static void
cells_to_values(const uint32_t *cells, const uint8_t *types, uint32_t count,
                wasm_val_t *values)
{
    for (uint32_t i = 0; i < count; i++) {
        values[i].kind = value_kind(types[i]);
        size_t size = value_size(values[i].kind);
        memcpy(&values[i].of, cells, size);
        cells += size / 4;
    }
}
static void
values_to_cells(const wasm_val_t *values, uint32_t count, uint32_t *cells)
{
    for (uint32_t i = 0; i < count; i++) {
        size_t size = value_size(values[i].kind);
        memcpy(cells, &values[i].of, size);
        cells += size / 4;
    }
}
static bool
call_cells(TkObject *object, uint32_t *cells)
{
    WASMModuleInstance *instance = object->owner->native;
    if (!instance || !instance->e->functions || !instance->exec_env_singleton)
        return false;
    WASMFunctionInstance *function = instance->e->functions + object->index;
    WASMExecEnv *env = wasm_runtime_get_exec_env_singleton(
        (WASMModuleInstanceCommon *)instance);
    wasm_set_exception(instance, NULL);
    return env
           && wasm_call_function(env, function, function->param_cell_num,
                                 cells);
}
bool
tokamak_invoke_import(WASMModuleInstance *instance, WASMFuncType *type,
                      uint32_t *cells, void *environment)
{
    TkImport *import = environment;
    TkRuntime *runtime = import->module->runtime;
    if (import->target != NONE) {
        TkObject *object = runtime->objects[import->target];
        bool ok = call_cells(object, cells);
        if (!ok)
            wasm_set_exception(
                instance, (object->owner->native
                                   && wasm_get_exception(object->owner->native)
                               ? wasm_get_exception(object->owner->native)
                               : "WebAssembly instance is not executable"));
        return ok;
    }
    wasm_val_t *args =
        calloc(type->param_count + type->result_count + 1, sizeof(wasm_val_t));
    if (!args) {
        wasm_set_exception(instance, "allocate callback values failed");
        return false;
    }
    wasm_val_t *results = args + type->param_count;
    cells_to_values(cells, type->types, type->param_count, args);
    bool ok = active_call
              && active_call->callback(active_call->user, import->module,
                                       import->index, args, type->param_count,
                                       results, type->result_count);
    if (ok)
        values_to_cells(results, type->result_count, cells);
    else
        wasm_set_exception(instance, "JavaScript import threw an exception");
    free(args);
    return ok;
}
bool
tk_wasm_call(TkRuntime *runtime, uint32_t id, const wasm_val_t *args,
             wasm_val_t *results, void *user, TkCallback callback,
             TkInterrupted interrupted)
{
    TkObject *object = runtime->objects[id];
    WASMFuncType *type = object->of.function;
    uint32_t size = type->param_cell_num > type->ret_cell_num
                        ? type->param_cell_num
                        : type->ret_cell_num;
    snprintf(runtime->error, sizeof(runtime->error),
             "WebAssembly allocation failed");
    uint32_t *cells = calloc(size ? size : 1, sizeof(uint32_t));
    if (!cells)
        return false;
    values_to_cells(args, type->param_count, cells);
    TkCall call = { user, callback, interrupted }, *previous = active_call;
    active_call = &call;
    bool ok = call_cells(object, cells);
    active_call = previous;
    if (ok)
        cells_to_values(cells, type->types + type->param_count,
                        type->result_count, results);
    if (!ok)
        snprintf(
            runtime->error, sizeof(runtime->error), "%s",
            (object->owner->native && wasm_get_exception(object->owner->native)
                 ? wasm_get_exception(object->owner->native)
                 : "WebAssembly instance is not executable"));
    free(cells);
    return ok;
}
uint8_t *
tk_wasm_memory(TkRuntime *runtime, uint32_t id, size_t *length,
               uint64_t *generation)
{
    WASMMemoryInstance *memory = runtime->objects[id]->of.memory;
    *length = memory->memory_data_size;
    *generation = memory->growth_generation;
    return memory->memory_data;
}
uint64_t
tk_wasm_memory_maximum(TkRuntime *runtime, uint32_t id)
{
    uint32_t maximum = runtime->objects[id]->maximum;
    return maximum == NONE ? UINT64_MAX : (uint64_t)maximum * 65536;
}
bool
tk_wasm_memory_shared(TkRuntime *runtime, uint32_t id)
{
    return runtime->objects[id]->of.memory->is_shared_memory;
}
int64_t
tk_wasm_memory_grow(TkRuntime *runtime, uint32_t id, uint32_t delta)
{
    TkObject *object = runtime->objects[id];
    uint32_t previous = object->of.memory->cur_page_count;
    return wasm_enlarge_memory(object->owner->native, delta) ? (int64_t)previous
                                                             : -1;
}
wasm_val_t
tk_wasm_global_get(TkRuntime *runtime, uint32_t id)
{
    TkObject *object = runtime->objects[id];
    wasm_val_t value = { 0 };
    value.kind = object->value_kind;
    memcpy(&value.of, object->of.global, value_size(value.kind));
    return value;
}
void
tk_wasm_global_set(TkRuntime *runtime, uint32_t id, const wasm_val_t *value)
{
    TkObject *object = runtime->objects[id];
    memcpy(object->of.global, &value->of, value_size(value->kind));
}
uint32_t
tk_wasm_table_size(TkRuntime *runtime, uint32_t id)
{
    return runtime->objects[id]->of.table->cur_size;
}
uint32_t
tk_wasm_table_get(TkRuntime *runtime, uint32_t id, uint32_t index)
{
    return runtime->objects[id]->of.table->host_elems[index];
}
void
tk_wasm_table_set(TkRuntime *runtime, uint32_t id, uint32_t index,
                  uint32_t value)
{
    runtime->objects[id]->of.table->host_elems[index] = value;
}
int64_t
tk_wasm_table_grow(TkRuntime *runtime, uint32_t id, uint32_t delta,
                   uint32_t value)
{
    WASMTableInstance *table = runtime->objects[id]->of.table;
    uint32_t previous = table->cur_size;
    return tokamak_grow_table(table, delta, value) ? (int64_t)previous : -1;
}
