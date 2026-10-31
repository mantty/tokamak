// Generated from tokamak's Rust code; do not edit. Regenerate with
// TOKAMAK_GENERATE=1 cargo test -p tokamak --features native generated_declarations

declare module "tokamak:host" {
  const classBrand: unique symbol;

  export function arrayBufferView(value: unknown): ViewParts;

  export function asyncContextGet(): AsyncContextFrame | undefined;

  export function asyncContextSet(frame: AsyncContextFrame | undefined): void;

  export function builtinNamespace(name: string): Record<string, unknown> | undefined;

  export function cacheDelete(name: string, key: string): boolean;

  export function cacheMatch(name: string, key: string): CachedResponse | undefined;

  export function cachePut(name: string, key: string, metadata: CacheMetadata, body: Uint8Array): void;

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

  export function httpFetch(request: FetchRequest): FetchTask;

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

  export function scheduleTimer(callback: () => void, milliseconds: number): Cancellation;

  export function socketConnect(host: string, port: number, secure: boolean): SocketConnection;

  export function urlDecodeParams(input: string): Array<[string, string]>;

  export function urlEncodeParams(entries: Array<[string, string]>): string;

  export function urlParse(input: string, base: string | undefined): UrlParts;

  export function urlPatternCompile(input: string, base: string | undefined, ignoreCase: boolean, runner: (arg0: string, arg1: string, arg2: string) => Array<[number, number] | undefined> | undefined): CompiledPattern;

  export function urlPatternMatchInput(input: string, base: string | undefined): PatternInput | undefined;

  export function urlSetComponent(href: string, component: string, value: string): UrlParts;

  export function writeStderr(text: string): void;

  export function writeStdout(text: string): void;

  export interface AssetFetcher {
    readonly [classBrand]: "AssetFetcher";
    fetch(method: string, path: string): AssetReply;
  }

  export interface AssetReply {
    status: number;
    statusText?: string | undefined;
    contentType?: string | undefined;
    body?: Uint8Array | undefined;
  }

  export interface BodyChunk {
    payload: Uint8Array;
    failed: boolean;
    done: boolean;
  }

  export interface BodyReader {
    read: () => Promise<Uint8Array | undefined>;
    cancel: () => Promise<void>;
  }

  export interface Bootstrap {
    hostRequest: (arg0: RequestDescriptor, arg1: Uint8Array | undefined) => object;
    hostResponse: (arg0: unknown) => ResponseParts;
    dispatchEvent: (arg0: object, arg1: string, arg2: Uint8Array) => Promise<unknown>;
    installAssets: (arg0: string, arg1: AssetFetcher) => void;
    installPlugins: (arg0: NativeHost, arg1: NativePlatform) => void;
    installServices: (arg0: string[], arg1: ServiceHost) => void;
    invokeService: (arg0: object, arg1: string, arg2: string, arg3: JsMessage) => Promise<JsReply>;
    serviceStreams: StreamCodec;
    trackWaitUntil: (arg0: WaitUntilObserver) => void;
  }

  export interface CacheMetadata {
    status: number;
    statusText: string;
    headers: Array<[string, string]>;
    url: string;
    redirected: boolean;
    type: string;
  }

  export interface CachedResponse {
    metadata: CacheMetadata;
    body: Uint8Array;
  }

  export interface Cancellation {
    readonly [classBrand]: "Cancellation";
    cancel(): void;
  }

  export interface CipherState {
    readonly [classBrand]: "CipherState";
    update(input: Uint8Array): ArrayBuffer;
    finish(): ArrayBuffer;
    setAutoPadding(padding: boolean): void;
    setAAD(data: Uint8Array): void;
    setAuthTag(tag: Uint8Array): void;
    getAuthTag(): ArrayBuffer;
  }

  export interface CompiledPattern {
    protocol: PatternComponent;
    username: PatternComponent;
    password: PatternComponent;
    hostname: PatternComponent;
    port: PatternComponent;
    pathname: PatternComponent;
    search: PatternComponent;
    hash: PatternComponent;
    hasRegExpGroups: boolean;
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

  export interface FetchRequest {
    url: string;
    method: string;
    headers: Array<[string, string]>;
    body?: Uint8Array | ReadableStream | undefined;
    redirect: string;
  }

  export interface FetchTask {
    fetch: HttpFetch;
    upload: Promise<void>;
    response: Promise<FetchedResponse>;
  }

  export interface FetchedResponse {
    status: number;
    statusText: string;
    headers: Array<[string, string]>;
    url: string;
    redirected: boolean;
    bodyless: boolean;
    length?: number | undefined;
  }

  export interface HttpFetch {
    readonly [classBrand]: "HttpFetch";
    abort(): void;
    cancel(): void;
    read(): Promise<Uint8Array | undefined>;
  }

  export interface JsMessage {
    data: string;
    buffers: ArrayBuffer[];
    streams: ReadableStream[];
  }

  export interface JsReply {
    failed: boolean;
    value: JsMessage;
  }

  export interface NativeCall {
    reply: Promise<PluginReply>;
    cancellation: Cancellation;
  }

  export interface NativeHost {
    readonly [classBrand]: "NativeHost";
    call(plugin: string, method: string, payload: Uint8Array, pull: (() => Promise<BodyChunk>) | undefined): NativeCall;
    subscribe(plugin: string, method: string, payload: Uint8Array, subscriber: Subscriber): Cancellation;
  }

  export type NativePlatform = "ios" | "macos" | "android" | "windows" | "linux";

  export interface NativeSocket {
    receive: (arg0: string | ArrayBuffer) => void;
    close: (arg0: number, arg1: string) => void;
    disconnect: () => void;
    take: () => OutboxFrame[];
    listen: (arg0: SocketNotifier) => void;
  }

  export type OutboxFrame = { type: "message"; binary: true; data: ArrayBuffer } | { type: "message"; binary: false; data: string } | { type: "close"; code: number; reason: string };

  export type Overflow = "error" | "dropOldest";

  export interface PatternComponent {
    pattern: string;
    regexp: string;
    flags: string;
    groups: string[];
  }

  export interface PatternInput {
    protocol: string;
    username: string;
    password: string;
    hostname: string;
    port: string;
    pathname: string;
    search: string;
    hash: string;
  }

  export interface PluginReply {
    payload: ArrayBuffer;
    failed: boolean;
    done: boolean;
  }

  export interface RequestDescriptor {
    method: string;
    url: string;
    headers: Array<[string, string]>;
  }

  export interface ResponseParts {
    status: number;
    statusText: string;
    headers: Array<[string, string]>;
    encodeBody: string;
    body: Uint8Array | BodyReader;
    webSocket?: NativeSocket | undefined;
  }

  export interface ServiceCall {
    reply: Promise<JsReply>;
    cancellation: Cancellation;
  }

  export interface ServiceHost {
    readonly [classBrand]: "ServiceHost";
    call(name: string, method: string, message: JsMessage, codec: StreamCodec): ServiceCall;
  }

  export interface ServiceStreamPort {
    readonly [classBrand]: "ServiceStreamPort";
    read(codec: StreamCodec): Promise<JsReply>;
    cancel(reason: JsMessage, codec: StreamCodec): Promise<JsReply>;
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

  export interface SocketNotifier {
    readonly [classBrand]: "SocketNotifier";
    notify(): void;
  }

  export interface StreamCodec {
    openServiceStream: (arg0: ReadableStream) => StreamSource;
    receiveServiceStream: (arg0: ServiceStreamPort) => ReadableStream;
  }

  export interface StreamRead {
    done: boolean;
    failed: boolean;
    value: JsMessage;
  }

  export interface StreamSource {
    read: () => Promise<StreamRead>;
    cancel: (arg0: JsMessage) => Promise<JsReply>;
  }

  export interface Subscriber {
    deliver: (arg0: PluginReply) => void;
    overflow: Overflow;
  }

  export interface TextDecoderState {
    readonly [classBrand]: "TextDecoderState";
    readonly encoding: string;
    decode(input: Uint8Array, stream: boolean): string;
  }

  export interface UrlParts {
    href: string;
    protocol: string;
    username: string;
    password: string;
    host: string;
    hostname: string;
    port: string;
    pathname: string;
    search: string;
    hash: string;
    origin: string;
  }

  export interface ViewParts {
    buffer: ArrayBufferLike;
    offset: number;
    length: number;
    tracking: boolean;
  }

  export interface WaitUntilObserver {
    readonly [classBrand]: "WaitUntilObserver";
    observe(pending: boolean): void;
  }
}

declare module "tokamak:storage" {
  const classBrand: unique symbol;

  export function d1Query(binding: string, format: string, body: string): Promise<D1Response>;

  export function kvDelete(binding: string, key: string): Promise<void>;

  export function kvGet(binding: string, key: string, cacheTtl: number | undefined): Promise<KvEntry | undefined>;

  export function kvGetMany(binding: string, keys: string[], cacheTtl: number | undefined): Promise<Array<KvEntry | undefined>>;

  export function kvList(binding: string, prefix: string, cursor: string, limit: number): Promise<Page>;

  export function kvPut(binding: string, key: string, writer: ValueWriter, options: PutOptions): Promise<void>;

  export function kvValue(): ValueWriter;

  export function kvWrite(writer: ValueWriter, chunk: Uint8Array): void;

  export function r2AbortUpload(binding: string, key: string, upload: string): Promise<void>;

  export function r2CloseBody(reader: Reader): void;

  export function r2CompleteUpload(binding: string, key: string, upload: string, parts: UploadedPart[]): Promise<R2Object>;

  export function r2CreateUpload(binding: string, key: string, metadata: ObjectMetadata): Promise<string>;

  export function r2Delete(binding: string, keys: string[]): Promise<void>;

  export function r2Get(binding: string, key: string, request: GetRequest): Promise<ObjectRead | undefined>;

  export function r2Head(binding: string, key: string): Promise<R2Object | undefined>;

  export function r2List(binding: string, request: ListRequest): Promise<Listing>;

  export function r2ObjectWriter(binding: string, length: number, checksum: Checksum | undefined): Promise<Writer>;

  export function r2PartWriter(binding: string, length: number): Promise<Writer>;

  export function r2Put(binding: string, key: string, writer: Writer, request: PutRequest): Promise<R2Object | undefined>;

  export function r2Read(reader: Reader): Promise<Uint8Array | undefined>;

  export function r2UploadPart(binding: string, key: string, upload: string, number: number, writer: Writer): Promise<string>;

  export function r2Write(writer: Writer, chunk: Uint8Array): Promise<void>;

  export type Algorithm = "md5" | "sha1" | "sha256" | "sha384" | "sha512";

  export interface Bounds {
    offset?: number | undefined;
    length?: number | undefined;
    suffix?: number | undefined;
  }

  export interface Checksum {
    algorithm: Algorithm;
    value: string;
  }

  export interface Conditional {
    etagMatches?: Etag[] | undefined;
    etagDoesNotMatch?: Etag[] | undefined;
    uploadedBefore?: number | undefined;
    uploadedAfter?: number | undefined;
    secondsGranularity?: boolean;
  }

  export interface D1Response {
    body: string;
    bookmark: string;
  }

  export type Etag = { type: "strong"; value: string } | { type: "weak"; value: string } | { type: "wildcard" };

  export interface GetRequest {
    onlyIf?: Conditional | undefined;
    range?: Bounds | undefined;
    rangeHeader?: string | undefined;
  }

  export type Include = "httpMetadata" | "customMetadata";

  export interface InstalledBinding {
    name: string;
    type: StoreKind;
  }

  export interface Key {
    name: string;
    expiration?: number | undefined;
    metadata?: string | undefined;
  }

  export interface KvEntry {
    value: Uint8Array;
    metadata?: string | undefined;
  }

  export interface ListRequest {
    limit?: number | undefined;
    prefix?: string;
    cursor?: string | undefined;
    delimiter?: string | undefined;
    startAfter?: string | undefined;
    include?: Include[];
  }

  export interface Listing {
    objects: R2Object[];
    truncated: boolean;
    cursor?: string | undefined;
    delimitedPrefixes: string[];
  }

  export interface ObjectBody {
    reader: Reader;
    length: number;
  }

  export interface ObjectMetadata {
    httpMetadata?: unknown;
    customMetadata?: unknown;
    storageClass?: string;
  }

  export interface ObjectRead {
    object: R2Object;
    body?: ObjectBody | undefined;
  }

  export interface Page {
    keys: Key[];
    cursor?: string | undefined;
  }

  export interface PutOptions {
    expirationTtl?: number | undefined;
    expiration?: number | undefined;
    metadata?: string | undefined;
  }

  export interface PutRequest {
    onlyIf?: Conditional | undefined;
    metadata: ObjectMetadata;
  }

  export interface R2Object {
    key: string;
    version: string;
    size: number;
    etag: string;
    uploaded: number;
    checksums: unknown;
    httpMetadata?: unknown | undefined;
    customMetadata?: unknown | undefined;
    storageClass: string;
    range?: Range | undefined;
  }

  export interface Range {
    offset: number;
    length: number;
  }

  export interface Reader {
    readonly [classBrand]: "Reader";
  }

  export type StoreKind = "kv" | "d1" | "r2";

  export interface UploadedPart {
    partNumber: number;
    etag: string;
  }

  export interface ValueWriter {
    readonly [classBrand]: "ValueWriter";
  }

  export interface Writer {
    readonly [classBrand]: "Writer";
  }
}
