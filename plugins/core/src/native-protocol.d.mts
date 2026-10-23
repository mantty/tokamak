/** The global through which the native host names its platform before the Worker's modules load. */
export const platformGlobal: "__tokamak_platform";
export interface MessageHeader {
  type: string;
  id?: number;
  plugin?: string;
  method?: string;
  name?: string;
  overflow?: string;
  failed?: boolean;
  done?: boolean;
}
export interface NativeMessage {
  header: MessageHeader;
  payload: Uint8Array;
}
export function encodeMessage(header: MessageHeader, payload?: Uint8Array): Uint8Array<ArrayBuffer>;
export function decodeMessage(bytes: Uint8Array): NativeMessage;
export function messages(body: AsyncIterable<Uint8Array> | Iterable<Uint8Array> | null): AsyncGenerator<NativeMessage>;
/** A failed endpoint payload. */
export interface NativeFailure {
  name: string;
  message: string;
  cause?: unknown;
}
export function nativeError(failure: NativeFailure): DOMException;
