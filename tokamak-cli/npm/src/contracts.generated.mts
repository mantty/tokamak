// Generated from tok's Rust code; do not edit. Regenerate with
// TOKAMAK_GENERATE=1 cargo test -p tokamak-cli generated_contracts
export interface ConfigReport {
  root: string;
  config: unknown;
}

export interface PluginManifest {
  schemaVersion: number;
  id: string;
  settings?: Record<string, PluginSetting>;
  build?: string | undefined;
  platforms?: Record<string, Record<string, unknown>>;
}

export interface PluginSetting {
  kind: string;
  of?: string | undefined;
  values?: string[] | undefined;
  description: string;
  default?: unknown | undefined;
}

export type ServerFile = { error: string } | ServerReport;

export interface ServerReport {
  url: string;
  certificates?: string;
  workerName?: string;
  socketPort: number;
}
