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

/** The words a plugin id must not be or start with, since tok's options use them. */
export const reservedPluginWords: readonly string[] = [
  "tokamak",
  "android",
  "ios",
  "macos",
  "windows",
  "name",
  "identifier",
  "icon",
  "version",
  "module",
  "build",
  "project",
  "platform",
  "skip",
  "update",
  "host",
  "help",
  "vite",
  "session"
];
