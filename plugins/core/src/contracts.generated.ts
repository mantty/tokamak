// Generated from tok's Rust code; do not edit. Regenerate with
// TOKAMAK_GENERATE=1 cargo test -p tokamak-cli generated_contracts
export interface BuildScriptInput {
  platform: string;
  settings: Record<string, unknown>;
  output: string;
}

export interface MessageHeader {
  type: string;
  id: number;
  plugin?: string | undefined;
  method?: string | undefined;
  name?: string | undefined;
  overflow?: string | undefined;
  failed?: boolean;
  done?: boolean;
}
