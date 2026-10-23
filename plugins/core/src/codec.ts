export type JsonValue = null | boolean | number | string | readonly JsonValue[] | { readonly [key: string]: JsonValue };

const encoder = new TextEncoder();
const decoder = new TextDecoder("utf-8", { fatal: true, ignoreBOM: false });

/** Encodes the standard structured-value endpoint payload. */
export function encodeJson(value: unknown): Uint8Array {
  return encoder.encode(JSON.stringify(value ?? null));
}

/** Decodes the standard structured-value endpoint payload. */
// eslint-disable-next-line @typescript-eslint/no-unnecessary-type-parameters -- the endpoint signature supplies the decoded type.
export function decodeJson<T>(payload: Uint8Array): T {
  return JSON.parse(decoder.decode(payload)) as T;
}
