import type { TokamakContext } from "@tokamakdev/plugin";

/** Native application operations composed from existing capabilities. */
export function {{factory}}(ctx: TokamakContext) {
  return {
    /** The current native page's URL. */
    getCurrentPage(): Promise<string | null> {
      return ctx.webview.getCurrentPage();
    },
  };
}
