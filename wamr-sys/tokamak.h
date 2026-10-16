#ifndef TOKAMAK_WAMR_H
#define TOKAMAK_WAMR_H
#include "wasm_runtime.h"
/* TOKAMAK: embedding hooks for JavaScript object identity and host references.
 */
void
tokamak_bind_instance(WASMModuleInstance *instance);
bool
tokamak_prepare_instance(WASMModuleInstance *instance);
WASMMemoryInstance *
tokamak_import_memory(const WASMModule *module, uint32_t index);
uint8_t *
tokamak_import_global(const WASMModule *module, uint32_t index);
WASMTableInstance **
tokamak_instantiate_tables(WASMModuleInstance *instance, char *error,
                           uint32_t length);
uint32_t
tokamak_function_ref(WASMModuleInstance *instance, uint32_t index);
WASMFunctionInstance *
tokamak_resolve_function(WASMModuleInstance *instance, uint32_t reference);
bool
tokamak_same_function_type(const WASMFuncType *a, const WASMFuncType *b);
bool
tokamak_invoke_import(WASMModuleInstance *instance, WASMFuncType *type,
                      uint32_t *cells, void *environment);
bool
tokamak_grow_table(WASMTableInstance *table, uint32_t delta,
                   table_elem_type_t value);
static inline table_elem_type_t *
tokamak_table_elements(WASMTableInstance *table)
{
    return table->host_elems ? table->host_elems : table->elems;
}
#endif
