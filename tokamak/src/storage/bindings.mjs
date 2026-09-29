// The binding constructor of each kind of store, loaded when an app uses it.
const constructors = {
  kv: async () => { const { KVNamespace } = await import("./kv.mjs"); return name => new KVNamespace(name); },
  d1: async () => (await import("./d1.mjs")).createD1Database,
  r2: async () => { const { R2Bucket } = await import("./r2.mjs"); return name => new R2Bucket(name); },
};

// Add each of `bindings`, a `name` and the `type` of its store, to `env`.
export async function install(env, bindings) {
  for (const type of new Set(bindings.map(binding => binding.type))) {
    const create = await constructors[type]();
    for (const { name } of bindings.filter(binding => binding.type === type)) env[name] = create(name);
  }
}
