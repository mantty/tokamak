import type { MessageHeader } from "./contracts.generated.ts";

export type { MessageHeader };
/** The global through which the native host names its platform before the Worker's modules load. */
export const platformGlobal: "__tokamak_platform";
/**
 * A message's header: a `MessageHeader` on the device's link, or one without an `id` on the
 * development Worker's link to the dev socket, where each request is one call.
 */
export type RoutingHeader = Omit<MessageHeader, "id"> & Partial<Pick<MessageHeader, "id">>;
export interface NativeMessage {
  header: RoutingHeader;
  payload: Uint8Array;
}
export function encodeMessage(header: RoutingHeader, payload?: Uint8Array): Uint8Array<ArrayBuffer>;
export function decodeMessage(bytes: Uint8Array): NativeMessage;
export function messages(body: AsyncIterable<Uint8Array> | Iterable<Uint8Array> | null): AsyncGenerator<NativeMessage>;
/** A failed endpoint payload. */
export interface NativeFailure {
  name: string;
  message: string;
  cause?: unknown;
}
export function nativeError(failure: NativeFailure): DOMException;
