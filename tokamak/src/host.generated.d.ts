// Generated from tokamak's Rust code; do not edit. Regenerate with
// TOKAMAK_GENERATE=1 cargo test -p tokamak --features native generated_declarations

declare module "tokamak:host" {
  const classBrand: unique symbol;

  export function arrayBufferView(value: unknown): ViewParts;

  export function asyncContextGet(): AsyncContextFrame | undefined;

  export function asyncContextSet(frame: AsyncContextFrame | undefined): void;

  export function builtinNamespace(name: string): Record<string, unknown> | undefined;

  export function cacheDelete(name: string, key: string): boolean;

  export function cacheMatch(name: string, key: string): object | undefined;

  export function cachePut(name: string, key: string, metadata: object, body: Uint8Array): void;

  export function cloneArrayBuffer(value: unknown, shared: boolean): ArrayBufferLike;

  export function createCompression(format: string, decode: boolean): CompressionCodec;

  export function createDecoder(label: string, fatal: boolean, ignoreBom: boolean): TextDecoderState;

  export function createNodeCompression(mode: number, options: string, dictionary: Uint8Array): object;

  export function cryptoAesGcm(encrypt: boolean, key: Uint8Array, iv: Uint8Array, input: Uint8Array, options: object): ArrayBuffer;

  export function cryptoCheckPrime(candidate: Uint8Array, options: object): boolean;

  export function cryptoCreateCipher(mode: string, encrypt: boolean, key: Uint8Array, iv: Uint8Array, tagLength: number | undefined): CipherState;

  export function cryptoCreateDigest(algorithm: string, key?: Uint8Array): DigestState;

  export function cryptoDecrypt(input: Uint8Array, options: object): ArrayBuffer;

  export function cryptoDerive(options: object): ArrayBuffer;

  export function cryptoDhCompute(options: object): ArrayBuffer;

  export function cryptoDhGenerate(options: object): object;

  export function cryptoDhParams(options: object): object;

  export function cryptoEcdhCompute(options: object): ArrayBuffer;

  export function cryptoEcdhConvert(input: Uint8Array, options: object): ArrayBuffer;

  export function cryptoEcdhPublic(options: object): ArrayBuffer;

  export function cryptoEncrypt(input: Uint8Array, options: object): ArrayBuffer;

  export function cryptoExportKey(options: object): ArrayBuffer;

  export function cryptoGenerateKey(options: object): object;

  export function cryptoGeneratePrime(options: object): ArrayBuffer;

  export function cryptoHkdf(algorithm: string, key: Uint8Array, salt: Uint8Array, info: Uint8Array, length: number): ArrayBuffer;

  export function cryptoImportKey(options: object): object;

  export function cryptoPbkdf2(algorithm: string, password: Uint8Array, salt: Uint8Array, iterations: number, length: number): ArrayBuffer;

  export function cryptoRsaLegacyPrivateEncrypt(input: Uint8Array, options: object): ArrayBuffer;

  export function cryptoRsaLegacyPublicDecrypt(input: Uint8Array, options: object): ArrayBuffer;

  export function cryptoScrypt(password: Uint8Array, options: object): ArrayBuffer;

  export function cryptoSign(input: Uint8Array, options: object): ArrayBuffer;

  export function cryptoTimingSafeEqual(left: Uint8Array, right: Uint8Array): boolean;

  export function cryptoVerify(signature: Uint8Array, options: object): boolean;

  export function detachArrayBuffer(buffer: ArrayBuffer): void;

  export function digest(algorithm: string, input: Uint8Array): ArrayBuffer;

  export function htmlRewrite(handlers: string, contentType: string): object;

  export function htmlValidateSelector(selector: string): void;

  export function httpStatusText(status: number): string;

  export function intlCanonicalLocales(input: string): string;

  export function intlCollatorCompare(left: string, right: string, locales: string, options: string): number;

  export function intlDateTime(milliseconds: number, locales: string, optionsJson: string, parts: boolean): string;

  export function intlDisplayName(code: string, locales: string, options: string): string;

  export function intlFractionDigits(options: string): number[];

  export function intlHourCycle(locales: string): string;

  export function intlListParts(values: string, locales: string, options: string): string;

  export function intlLocaleInfo(tag: string): string;

  export function intlNumber(value: number, locales: string, options: string): string;

  export function intlNumberParts(value: number, locales: string, options: string): string;

  export function intlPlural(value: number, locales: string, kind: string): string;

  export function intlPluralCategories(locales: string, kind: string): string;

  export function intlPluralRange(start: number, end: number, locales: string, kind: string): string;

  export function intlRelativeParts(value: number, unit: string, locales: string, options: string): string;

  export function intlSegment(text: string, granularity: string): string;

  export function ipVersion(input: string): number;

  export function isNodeBuiltin(name: string): boolean;

  export function nodeBuiltinNames(): string[];

  export function objectClass(value: unknown): string | undefined;

  export function processChdir(directory: unknown): void;

  export function processCwd(): string;

  export function randomBytes(length: number): Uint8Array;

  export function registerDomException(constructor: abstract new (...args: never[]) => unknown): void;

  export function scheduleTimer(callback: () => void, milliseconds: number): TimerHandle;

  export function socketConnect(host: string, port: number, secure: boolean): SocketConnection;

  export function urlDecodeParams(input: string): string;

  export function urlEncodeParams(entries: string): string;

  export function urlParse(input: string, base: string | undefined): object;

  export function urlPatternCompile(input: string, base: string | undefined, ignoreCase: boolean, runner: (...args: never[]) => unknown): object;

  export function urlPatternMatchInput(input: string, base: string | undefined): string[] | undefined;

  export function urlSetComponent(href: string, component: string, value: string): object;

  export function writeStderr(text: string): void;

  export function writeStdout(text: string): void;

  export interface CipherState {
    readonly [classBrand]: "CipherState";
    update(input: Uint8Array): ArrayBuffer;
    finish(): ArrayBuffer;
    setAutoPadding(padding: boolean): void;
    setAAD(data: Uint8Array): void;
    setAuthTag(tag: Uint8Array): void;
    getAuthTag(): ArrayBuffer;
  }

  export interface CompressionCodec {
    readonly [classBrand]: "CompressionCodec";
    write(input: Uint8Array, finish: boolean, emit: (arg0: Uint8Array) => void): void;
  }

  export interface DigestState {
    readonly [classBrand]: "DigestState";
    update(input: Uint8Array): void;
    finish(): ArrayBuffer;
    copy(): DigestState;
  }

  export interface SocketConnection {
    readonly [classBrand]: "SocketConnection";
    opened(): Promise<void>;
    close(): void;
    read(): Promise<Uint8Array | undefined>;
    write(bytes: Uint8Array): Promise<void>;
    shutdown(): Promise<void>;
    startTls(host: string): SocketConnection;
  }

  export interface TextDecoderState {
    readonly [classBrand]: "TextDecoderState";
    readonly encoding: string;
    decode(input: Uint8Array, stream: boolean): string;
  }

  export interface TimerHandle {
    readonly [classBrand]: "TimerHandle";
    cancel(): void;
  }

  export interface ViewParts {
    buffer: ArrayBufferLike;
    offset: number;
    length: number;
    tracking: boolean;
  }
}

declare module "tokamak:storage" {
  const classBrand: unique symbol;

  export function d1Query(binding: string, format: string, body: string): Promise<object>;

  export function kvDelete(binding: string, key: string): Promise<void>;

  export function kvGet(binding: string, key: string, cacheTtl: number | undefined): Promise<object | undefined>;

  export function kvGetMany(binding: string, keys: string[], cacheTtl: number | undefined): Promise<Array<object | undefined>>;

  export function kvList(binding: string, prefix: string, cursor: string, limit: number): Promise<string>;

  export function kvPut(binding: string, key: string, writer: ValueWriter, options: string): Promise<void>;

  export function kvValue(): ValueWriter;

  export function kvWrite(writer: ValueWriter, chunk: Uint8Array): void;

  export function r2AbortUpload(binding: string, key: string, upload: string): Promise<void>;

  export function r2CloseBody(reader: Reader): void;

  export function r2CompleteUpload(binding: string, key: string, upload: string, parts: string): Promise<string>;

  export function r2CreateUpload(binding: string, key: string, metadata: string): Promise<string>;

  export function r2Delete(binding: string, keys: string[]): Promise<void>;

  export function r2Get(binding: string, key: string, request: string): Promise<object | undefined>;

  export function r2Head(binding: string, key: string): Promise<string | undefined>;

  export function r2List(binding: string, request: string): Promise<string>;

  export function r2ObjectWriter(binding: string, length: number, checksum: string | undefined): Promise<Writer>;

  export function r2PartWriter(binding: string, length: number): Promise<Writer>;

  export function r2Put(binding: string, key: string, writer: Writer, request: string): Promise<string | undefined>;

  export function r2Read(reader: Reader): Promise<Uint8Array | undefined>;

  export function r2UploadPart(binding: string, key: string, upload: string, number: number, writer: Writer): Promise<string>;

  export function r2Write(writer: Writer, chunk: Uint8Array): Promise<void>;

  export interface Reader {
    readonly [classBrand]: "Reader";
  }

  export interface ValueWriter {
    readonly [classBrand]: "ValueWriter";
  }

  export interface Writer {
    readonly [classBrand]: "Writer";
  }
}
