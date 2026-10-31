import type { InstalledBinding, StoreKind } from "tokamak:storage";

// The binding constructor of each kind of store, loaded when an app uses it.
const constructors: Record<StoreKind, () => Promise<(name: string) => unknown>> = {
  kv: async () => { const { KVNamespace } = await import("./kv.ts"); return name => new KVNamespace(name); },
  d1: async () => (await import("./d1.ts")).createD1Database,
  r2: async () => { const { R2Bucket } = await import("./r2.ts"); return name => new R2Bucket(name); },
};

// Add each of `bindings`, a `name` and the `type` of its store, to `env`.
export async function install(env: Record<PropertyKey, unknown>, bindings: InstalledBinding[]): Promise<void> {
  for (const type of new Set(bindings.map(binding => binding.type))) {
    const create = await constructors[type]();
    for (const { name } of bindings.filter(binding => binding.type === type)) env[name] = create(name);
  }
}
