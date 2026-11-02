// The globals the runtime's modules define for each other and for the Rust
// host, which reads them by name.
import type { NativePlatform, RequestDescriptor } from "tokamak:host";
import type { InstalledBinding } from "tokamak:storage";
import type { ExecutionContext } from "./builtins/runtime.ts";

declare global {
  var __tokamak_context: ExecutionContext;
  var __tokamak_env: Record<PropertyKey, unknown>;
  var __tokamak_exports: Record<PropertyKey, unknown> | undefined;
  var __tokamak_drain_wait_until: () => Promise<void>;
  // The native platform, which `@tokamakdev/plugin` reads while modules load.
  var __tokamak_platform: NativePlatform | undefined;
  // The request the dispatcher is handling.
  var __tokamak_request: RequestDescriptor | undefined;
  // The app's storage bindings, when the storage part is linked.
  var __tokamak_storage: InstalledBinding[] | undefined;
}

export {};
