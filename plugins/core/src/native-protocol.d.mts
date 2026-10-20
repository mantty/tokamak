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
