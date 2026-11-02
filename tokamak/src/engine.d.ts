// QuickJS features that TypeScript 5.9's libraries do not declare.
declare global {
  interface Uint8ArrayConstructor {
    fromBase64(text: string, options?: { alphabet?: "base64" | "base64url"; lastChunkHandling?: "loose" | "strict" | "stop-before-partial" }): Uint8Array<ArrayBuffer>;
    fromHex(text: string): Uint8Array<ArrayBuffer>;
  }

  interface Uint8Array {
    toBase64(options?: { alphabet?: "base64" | "base64url"; omitPadding?: boolean }): string;
    toHex(): string;
  }
}

export {};
