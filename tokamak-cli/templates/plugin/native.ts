import { Plugin, type TokamakContext } from "@tokamakdev/plugin";

class NativeApi extends Plugin {
  constructor(ctx: TokamakContext) { super(ctx, "{{id}}"); }

  /** Returns a message through the native implementation. */
  echo(message: string): Promise<string> {
    return this.call("echo", message);
  }
}

export function {{factory}}(ctx: TokamakContext) { return new NativeApi(ctx); }
