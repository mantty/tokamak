import { capabilitiesOf, platformApi, type TokamakContext } from "@tokamakdev/plugin";

{{imports}}

/** The {{id}} plugin's native operations. */
export function {{factory}}(ctx: TokamakContext) {
  const native = platformApi(ctx, { {{families}} });
  return {
    /** Whether each operation is implemented where the app runs, without calling it. Permission is not reported. */
    get capabilities(): Readonly<Record<string, boolean>> {
      return capabilitiesOf(ctx, "{{id}}");
    },

    /** Returns a message through the native implementation. */
    echo: (message: string): Promise<string> => native.echo(message),
  };
}
