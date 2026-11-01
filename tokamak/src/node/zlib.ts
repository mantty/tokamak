import { type CodecOptions, type NodeCodec, createNodeCompression } from "tokamak:host";
import { type AnyFunction, isFunction, isObject, objectOf, propertyOf } from "../globals/objects.ts";
import { queueMicrotask } from "../globals/timers.ts";
import { Buffer } from "./buffer.ts";
import { type StreamCallback, Transform } from "../streams/node.js";
import { codedError } from "./errors.ts";
import { validateFunction } from "./validators.ts";

const empty = Buffer.alloc(0);
const flushMarker = Symbol("zlib flush");

// A chunk written to a zlib stream; an empty one marked with a flush value flushes the stream.
type FlushChunk = Buffer & { [flushMarker]?: number };

// Input being passed through a stream's codec.
interface Job {
  input: Uint8Array;
  offset: number;
  flush: number;
  callback: StreamCallback;
  complete: boolean;
}

type ZlibConstructor = new (options?: unknown) => ZlibTransform;

function invalidType(name: string, expected: string) {
  return codedError(TypeError, 'The "' + name + '" argument must be of type ' + expected, "ERR_INVALID_ARG_TYPE");
}

function outOfRange(name: string) {
  return codedError(RangeError, name + " is out of range", "ERR_OUT_OF_RANGE");
}

function numberOption<T>(value: unknown, name: string, min: number, max: number, fallback: T): number | T {
  if (value === undefined || Number.isNaN(value)) return fallback;
  if (typeof value !== "number") throw invalidType(name, "number");
  if (value < min || value > max) throw outOfRange(name);
  return Math.trunc(value);
}

function inputBytes(input: unknown): Buffer {
  if (typeof input === "string") return Buffer.from(input);
  if (input instanceof ArrayBuffer) return Buffer.from(input);
  if (ArrayBuffer.isView(input)) return Buffer.from(input.buffer, input.byteOffset, input.byteLength);
  throw invalidType("buffer", "string, Buffer, TypedArray, DataView, or ArrayBuffer");
}

function codecOptions(mode: number, options: Record<PropertyKey, unknown>): CodecOptions {
  const minWindow = mode === 3 ? 9 : mode === 2 || mode === 4 || mode === 7 ? 0 : 8;
  const zlibOptions: Record<PropertyKey, unknown> = mode < 8 ? options : {};
  const result: CodecOptions = {
    windowBits: numberOption(zlibOptions.windowBits, "options.windowBits", minWindow, 15, 15),
    level: numberOption(zlibOptions.level, "options.level", -1, 9, -1),
    memLevel: numberOption(zlibOptions.memLevel, "options.memLevel", 1, 9, 8),
    strategy: numberOption(zlibOptions.strategy, "options.strategy", 0, 4, 0),
    params: {},
  };
  if (mode === 5 && result.windowBits === 8) result.windowBits = 9;
  const { params } = options;
  if (mode >= 8 && params != null) {
    const values = objectOf(params);
    for (const key of Object.keys(params)) {
      const id = Number(key);
      const max = mode < 10 ? 8 : mode === 11 ? 100 : 402;
      if (!Number.isInteger(id) || id < 0 || id > max || Object.hasOwn(result.params, id)) {
        throw codedError(RangeError, key + " is not a valid compression parameter", mode < 10 ? "ERR_BROTLI_INVALID_PARAM" : "ERR_ZSTD_INVALID_PARAM");
      }
      const value = values[key];
      const number = typeof value === "boolean" ? Number(value) : value;
      if (typeof number !== "number") throw invalidType("options.params[" + key + "]", "number");
      if ((number | 0) !== -1) result.params[id] = number | 0;
    }
  }
  if (options.pledgedSrcSize !== undefined && mode === 10) {
    result.pledgedSrcSize = numberOption(options.pledgedSrcSize, "options.pledgedSrcSize", 0, Number.MAX_SAFE_INTEGER, undefined);
  }
  return result;
}

// The engine's codec; a destroy or a synchronous operation closes it.
function codecOf(engine: ZlibTransform): NodeCodec {
  if (engine._codec === null) throw new TypeError("Compression stream is closed");
  return engine._codec;
}

class ZlibTransform extends Transform {
  declare _codec: NodeCodec | null;
  declare _chunkSize: number;
  declare _maxOutputLength: number;
  declare _defaultFlush: number;
  declare _finishFlush: number;
  declare _fullFlush: number;
  declare _maxFlush: number;
  declare _info: boolean;
  declare _job: Job | null;
  declare _pumping: boolean;
  declare bytesWritten: number;

  constructor(mode: number, options: unknown = {}) {
    const given = options ?? {};
    if (typeof given !== "object") throw invalidType("options", "object");
    const settings = objectOf(given);
    const familyFinish = mode < 8 ? 4 : 2;
    const maxFlush = mode < 8 ? 5 : mode < 10 ? 3 : 2;
    const size = numberOption(settings.chunkSize, "options.chunkSize", 64, Number.MAX_SAFE_INTEGER, 16384);
    const maxOutput = numberOption(settings.maxOutputLength, "options.maxOutputLength", 1, 0x7fffffff, 0x7fffffff);
    const flush = numberOption(settings.flush, "options.flush", 0, maxFlush, 0);
    const finishFlush = numberOption(settings.finishFlush, "options.finishFlush", 0, maxFlush, familyFinish);
    const config = codecOptions(mode, settings);
    let dictionary = empty;
    if (mode < 8 && settings.dictionary !== undefined) {
      if (!(settings.dictionary instanceof ArrayBuffer) && !ArrayBuffer.isView(settings.dictionary)) throw invalidType("options.dictionary", "Buffer, TypedArray, DataView, or ArrayBuffer");
      dictionary = inputBytes(settings.dictionary);
    }
    const codec = createNodeCompression(mode, config, dictionary);
    super({ ...settings, objectMode: false, writableObjectMode: false, readableObjectMode: false, encoding: undefined, autoDestroy: true });
    this._codec = codec;
    this._chunkSize = Math.min(size, 65536);
    this._maxOutputLength = maxOutput;
    this._defaultFlush = flush;
    this._finishFlush = finishFlush;
    this._fullFlush = mode < 8 ? 3 : 1;
    this._maxFlush = maxFlush;
    this._info = Boolean(settings.info);
    this._job = null;
    this._pumping = false;
    this.bytesWritten = 0;
  }

  _transform(chunk: FlushChunk, _encoding: unknown, callback: StreamCallback): void {
    startJob(this, chunk, chunk[flushMarker] ?? this._defaultFlush, callback);
  }

  _flush(callback: StreamCallback): void {
    startJob(this, empty, this._finishFlush, callback);
  }

  _read(size: number): void {
    super._read(size);
    this._pump();
  }

  _pump(): void {
    if (this._pumping || !this._job) return;
    this._pumping = true;
    try {
      while (this._job) {
        const job = this._job;
        if (job.complete) {
          this._job = null;
          job.callback();
          continue;
        }
        const step = codecOf(this).step(job.input.subarray(job.offset), job.flush, this._chunkSize);
        job.offset += step.consumed;
        this.bytesWritten += step.consumed;
        job.complete = step.ended || (job.offset === job.input.length && step.output.length < this._chunkSize);
        if (step.output.length && !this.push(Buffer.from(step.output))) break;
        if (!job.complete && !step.consumed && !step.output.length) throw new Error("Compression made no progress");
      }
    } catch (error) {
      const job = this._job;
      this._job = null;
      if (job) job.callback(error);
      else throw error;
    } finally { this._pumping = false; }
  }

  _destroy(error: unknown, callback: StreamCallback): void {
    this._codec?.close();
    this._codec = null;
    this._job = null;
    callback(error);
  }

  flush(kind?: unknown, callback?: unknown): void {
    if (typeof kind === "function") { callback = kind; kind = undefined; }
    const flush = numberOption(kind, "kind", 0, this._maxFlush, this._fullFlush);
    if (this.writableFinished) { if (callback) queueMicrotask(callback); return; }
    if (this.writableEnded) {
      if (callback) {
        validateFunction(callback, "listener");
        this.once("end", callback);
      }
      return;
    }
    const marker: FlushChunk = Buffer.alloc(0);
    marker[flushMarker] = flush;
    // A write ignores a callback that is not a function.
    this.write(marker, isFunction(callback) ? callback : undefined);
  }

  params(level: unknown, strategy: unknown, callback: unknown): void {
    const levelValue = numberOption(level, "level", -1, 9, -1);
    const strategyValue = numberOption(strategy, "strategy", 0, 4, 0);
    if (!isFunction(callback)) throw invalidType("callback", "function");
    this.flush(2, (error?: unknown) => {
      if (error) { callback(error); return; }
      try {
        const output = codecOf(this).params(levelValue, strategyValue);
        if (output.length) this.push(Buffer.from(output));
      }
      catch (error) { callback(error); return; }
      callback();
    });
  }

  reset(): void { codecOf(this).reset(); }
  close(callback?: unknown): void {
    if (callback) {
      if (this.closed) queueMicrotask(callback);
      else {
        validateFunction(callback, "listener");
        this.once("close", callback);
      }
    }
    this.destroy();
  }
  get bytesRead(): number { return this.bytesWritten; }
  get _closed(): boolean { return this._codec === null; }

  _processChunk(chunk: unknown, flush: number, callback?: unknown): Buffer | undefined {
    if (isFunction(callback)) {
      startJob(this, inputBytes(chunk), flush, callback);
      return;
    }
    return processSync(this, inputBytes(chunk), flush);
  }
}

function startJob(engine: ZlibTransform, input: Uint8Array, flush: number, callback: StreamCallback): void {
  engine._job = { input, offset: 0, flush, callback, complete: false };
  engine._pump();
}

function processSync(engine: ZlibTransform, input: Uint8Array, flush: number): Buffer {
  const chunks: Buffer[] = [];
  let size = 0;
  let offset = 0;
  try {
    while (true) {
      const step = codecOf(engine).step(input.subarray(offset), flush, engine._chunkSize);
      offset += step.consumed;
      engine.bytesWritten += step.consumed;
      size += step.output.length;
      if (size > engine._maxOutputLength) throw new RangeError("Cannot create a Buffer larger than maxOutputLength");
      if (step.output.length) chunks.push(Buffer.from(step.output));
      if (step.ended || (offset === input.length && step.output.length < engine._chunkSize)) break;
      if (!step.consumed && !step.output.length) throw new Error("Compression made no progress");
    }
    return Buffer.concat(chunks, size);
  } catch (error) {
    // Workerd's synchronous codec throws an Error; async streams additionally
    // expose the codec code through their error event/callback.
    if (isObject(error)) delete error.code;
    throw error;
  } finally {
    codecOf(engine).close();
    engine._codec = null;
  }
}

function operation(Type: ZlibConstructor, input: unknown, options: unknown): Buffer | { buffer: Buffer; engine: ZlibTransform } {
  const bytes = inputBytes(input);
  const engine = new Type(options);
  const buffer = processSync(engine, bytes, engine._finishFlush);
  return engine._info ? { buffer, engine } : buffer;
}

function callbackOperation(Type: ZlibConstructor, input: unknown, options: unknown, callback: unknown): void {
  if (typeof options === "function") { callback = options; options = undefined; }
  if (!isFunction(callback)) throw invalidType("callback", "function");
  const report: AnyFunction = callback;
  const bytes = inputBytes(input);
  const engine = new Type(options);
  const chunks: Buffer[] = [];
  let size = 0;
  let called = false;
  const done = (error: unknown, value?: unknown) => {
    if (called) return;
    called = true;
    report(error, value);
  };
  engine.on("data", (chunk: Buffer) => {
    size += chunk.length;
    if (size > engine._maxOutputLength) engine.destroy(new RangeError("Cannot create a Buffer larger than maxOutputLength"));
    else chunks.push(chunk);
  });
  engine.once("error", error => done(error));
  engine.once("end", () => {
    const buffer = Buffer.concat(chunks, size);
    done(null, engine._info ? { buffer, engine } : buffer);
  });
  engine.end(bytes);
}

export class Deflate extends ZlibTransform { constructor(options?: unknown) { super(1, options); } }
export class Inflate extends ZlibTransform { constructor(options?: unknown) { super(2, options); } }
export class Gzip extends ZlibTransform { constructor(options?: unknown) { super(3, options); } }
export class Gunzip extends ZlibTransform { constructor(options?: unknown) { super(4, options); } }
export class DeflateRaw extends ZlibTransform { constructor(options?: unknown) { super(5, options); } }
export class InflateRaw extends ZlibTransform { constructor(options?: unknown) { super(6, options); } }
export class Unzip extends ZlibTransform { constructor(options?: unknown) { super(7, options); } }
export class BrotliDecompress extends ZlibTransform { constructor(options?: unknown) { super(8, options); } }
export class BrotliCompress extends ZlibTransform { constructor(options?: unknown) { super(9, options); } }
export class ZstdCompress extends ZlibTransform { constructor(options?: unknown) { super(10, options); } }
export class ZstdDecompress extends ZlibTransform { constructor(options?: unknown) { super(11, options); } }

export const createGzip = (options?: unknown) => new Gzip(options);
export const gzipSync = (input: unknown, options?: unknown) => operation(Gzip, input, options);
export function gzip(input: unknown, options?: unknown, callback?: unknown): void { callbackOperation(Gzip, input, options, callback); }
export const createGunzip = (options?: unknown) => new Gunzip(options);
export const gunzipSync = (input: unknown, options?: unknown) => operation(Gunzip, input, options);
export function gunzip(input: unknown, options?: unknown, callback?: unknown): void { callbackOperation(Gunzip, input, options, callback); }
export const createDeflate = (options?: unknown) => new Deflate(options);
export const deflateSync = (input: unknown, options?: unknown) => operation(Deflate, input, options);
export function deflate(input: unknown, options?: unknown, callback?: unknown): void { callbackOperation(Deflate, input, options, callback); }
export const createInflate = (options?: unknown) => new Inflate(options);
export const inflateSync = (input: unknown, options?: unknown) => operation(Inflate, input, options);
export function inflate(input: unknown, options?: unknown, callback?: unknown): void { callbackOperation(Inflate, input, options, callback); }
export const createDeflateRaw = (options?: unknown) => new DeflateRaw(options);
export const deflateRawSync = (input: unknown, options?: unknown) => operation(DeflateRaw, input, options);
export function deflateRaw(input: unknown, options?: unknown, callback?: unknown): void { callbackOperation(DeflateRaw, input, options, callback); }
export const createInflateRaw = (options?: unknown) => new InflateRaw(options);
export const inflateRawSync = (input: unknown, options?: unknown) => operation(InflateRaw, input, options);
export function inflateRaw(input: unknown, options?: unknown, callback?: unknown): void { callbackOperation(InflateRaw, input, options, callback); }
export const createUnzip = (options?: unknown) => new Unzip(options);
export const unzipSync = (input: unknown, options?: unknown) => operation(Unzip, input, options);
export function unzip(input: unknown, options?: unknown, callback?: unknown): void { callbackOperation(Unzip, input, options, callback); }
export const createBrotliCompress = (options?: unknown) => new BrotliCompress(options);
export const brotliCompressSync = (input: unknown, options?: unknown) => operation(BrotliCompress, input, options);
export function brotliCompress(input: unknown, options?: unknown, callback?: unknown): void { callbackOperation(BrotliCompress, input, options, callback); }
export const createBrotliDecompress = (options?: unknown) => new BrotliDecompress(options);
export const brotliDecompressSync = (input: unknown, options?: unknown) => operation(BrotliDecompress, input, options);
export function brotliDecompress(input: unknown, options?: unknown, callback?: unknown): void { callbackOperation(BrotliDecompress, input, options, callback); }
export const createZstdCompress = (options?: unknown) => new ZstdCompress(options);
export const zstdCompressSync = (input: unknown, options?: unknown) => operation(ZstdCompress, input, options);
export function zstdCompress(input: unknown, options?: unknown, callback?: unknown): void { callbackOperation(ZstdCompress, input, options, callback); }
export const createZstdDecompress = (options?: unknown) => new ZstdDecompress(options);
export const zstdDecompressSync = (input: unknown, options?: unknown) => operation(ZstdDecompress, input, options);
export function zstdDecompress(input: unknown, options?: unknown, callback?: unknown): void { callbackOperation(ZstdDecompress, input, options, callback); }

const codeNames = ["Z_OK", "Z_STREAM_END", "Z_NEED_DICT", "Z_ERRNO", "Z_STREAM_ERROR", "Z_DATA_ERROR", "Z_MEM_ERROR", "Z_BUF_ERROR", "Z_VERSION_ERROR"];
const codeEntries = codeNames.map((name, index): [string, number] => [name, index <= 2 ? index : 2 - index]);
export const codes = Object.assign(Object.fromEntries(codeEntries), Object.fromEntries(codeEntries.map(([name, code]) => [code, name])));
export const constants = {
  Z_NO_FLUSH: 0, Z_PARTIAL_FLUSH: 1, Z_SYNC_FLUSH: 2, Z_FULL_FLUSH: 3, Z_FINISH: 4, Z_BLOCK: 5, Z_TREES: 6,
  Z_OK: 0, Z_STREAM_END: 1, Z_NEED_DICT: 2, Z_ERRNO: -1, Z_STREAM_ERROR: -2, Z_DATA_ERROR: -3, Z_MEM_ERROR: -4,
  Z_BUF_ERROR: -5, Z_VERSION_ERROR: -6, Z_DEFAULT_COMPRESSION: -1, Z_DEFAULT_LEVEL: -1, Z_MIN_LEVEL: -1, Z_MAX_LEVEL: 9,
  Z_FILTERED: 1, Z_HUFFMAN_ONLY: 2, Z_RLE: 3, Z_FIXED: 4, Z_BINARY: 0, Z_TEXT: 1, Z_UNKNOWN: 2,
  Z_DEFLATED: 8, Z_BEST_COMPRESSION: 9, Z_BEST_SPEED: 1, Z_DEFAULT_CHUNK: 16384, Z_DEFAULT_MEMLEVEL: 8,
  Z_DEFAULT_STRATEGY: 0, Z_DEFAULT_WINDOWBITS: 15, Z_MAX_CHUNK: 0x7fffffff, Z_MAX_MEMLEVEL: 9, Z_MAX_WINDOWBITS: 15,
  Z_MIN_CHUNK: 64, Z_MIN_MEMLEVEL: 1, Z_MIN_WINDOWBITS: 8, Z_NO_COMPRESSION: 0,
  GUNZIP: 4, GZIP: 3, INFLATE: 2, DEFLATE: 1, INFLATERAW: 6, DEFLATERAW: 5, UNZIP: 7,
  BROTLI_OPERATION_PROCESS: 0, BROTLI_OPERATION_FLUSH: 1, BROTLI_OPERATION_FINISH: 2, BROTLI_OPERATION_EMIT_METADATA: 3,
  BROTLI_PARAM_MODE: 0, BROTLI_PARAM_QUALITY: 1, BROTLI_PARAM_SIZE_HINT: 5, BROTLI_PARAM_DISABLE_LITERAL_CONTEXT_MODELING: 4,
  BROTLI_PARAM_LGWIN: 2, BROTLI_PARAM_LGBLOCK: 3, BROTLI_PARAM_NPOSTFIX: 7, BROTLI_PARAM_NDIRECT: 8, BROTLI_PARAM_LARGE_WINDOW: 6,
  BROTLI_MODE_GENERIC: 0, BROTLI_MODE_TEXT: 1, BROTLI_MODE_FONT: 2, BROTLI_DEFAULT_MODE: 0, BROTLI_DEFAULT_QUALITY: 11,
  BROTLI_DEFAULT_WINDOW: 22, BROTLI_MIN_QUALITY: 0, BROTLI_MAX_QUALITY: 11, BROTLI_MIN_WINDOW_BITS: 10, BROTLI_MAX_WINDOW_BITS: 24,
  BROTLI_LARGE_MAX_WINDOW_BITS: 30, BROTLI_MIN_INPUT_BLOCK_BITS: 16, BROTLI_MAX_INPUT_BLOCK_BITS: 24,
  BROTLI_DECODER_RESULT_ERROR: 0, BROTLI_DECODER_RESULT_NEEDS_MORE_INPUT: 2, BROTLI_DECODER_RESULT_NEEDS_MORE_OUTPUT: 3,
  BROTLI_DECODER_RESULT_SUCCESS: 1, BROTLI_DECODER_NO_ERROR: 0, BROTLI_DECODER_NEEDS_MORE_INPUT: 2, BROTLI_DECODER_NEEDS_MORE_OUTPUT: 3,
  BROTLI_DECODER_SUCCESS: 1, BROTLI_DECODER_ERROR_INVALID_ARGUMENTS: -20,
  BROTLI_DECODER_PARAM_DISABLE_RING_BUFFER_REALLOCATION: 0, BROTLI_DECODER_PARAM_LARGE_WINDOW: 1,
  BROTLI_DECODE: 8, BROTLI_ENCODE: 9,
  BROTLI_DECODER_ERROR_ALLOC_BLOCK_TYPE_TREES: -30, BROTLI_DECODER_ERROR_ALLOC_CONTEXT_MAP: -25,
  BROTLI_DECODER_ERROR_ALLOC_CONTEXT_MODES: -21, BROTLI_DECODER_ERROR_ALLOC_RING_BUFFER_1: -26,
  BROTLI_DECODER_ERROR_ALLOC_RING_BUFFER_2: -27, BROTLI_DECODER_ERROR_ALLOC_TREE_GROUPS: -22,
  BROTLI_DECODER_ERROR_DICTIONARY_NOT_SET: -19, BROTLI_DECODER_ERROR_FORMAT_BLOCK_LENGTH_1: -9,
  BROTLI_DECODER_ERROR_FORMAT_BLOCK_LENGTH_2: -10, BROTLI_DECODER_ERROR_FORMAT_CL_SPACE: -6,
  BROTLI_DECODER_ERROR_FORMAT_CONTEXT_MAP_REPEAT: -8, BROTLI_DECODER_ERROR_FORMAT_DICTIONARY: -12,
  BROTLI_DECODER_ERROR_FORMAT_DISTANCE: -16, BROTLI_DECODER_ERROR_FORMAT_EXUBERANT_META_NIBBLE: -3,
  BROTLI_DECODER_ERROR_FORMAT_EXUBERANT_NIBBLE: -1, BROTLI_DECODER_ERROR_FORMAT_HUFFMAN_SPACE: -7,
  BROTLI_DECODER_ERROR_FORMAT_PADDING_1: -14, BROTLI_DECODER_ERROR_FORMAT_PADDING_2: -15,
  BROTLI_DECODER_ERROR_FORMAT_RESERVED: -2, BROTLI_DECODER_ERROR_FORMAT_SIMPLE_HUFFMAN_ALPHABET: -4,
  BROTLI_DECODER_ERROR_FORMAT_SIMPLE_HUFFMAN_SAME: -5, BROTLI_DECODER_ERROR_FORMAT_TRANSFORM: -11,
  BROTLI_DECODER_ERROR_FORMAT_WINDOW_BITS: -13, BROTLI_DECODER_ERROR_UNREACHABLE: -31,
  ZLIB_VERNUM: 4897,
  ZSTD_CLEVEL_DEFAULT: 3, ZSTD_COMPRESS: 10, ZSTD_DECODE: 11, ZSTD_DECOMPRESS: 11, ZSTD_ENCODE: 10,
  ZSTD_btlazy2: 6, ZSTD_btopt: 7, ZSTD_btultra: 8, ZSTD_btultra2: 9, ZSTD_c_chainLog: 103,
  ZSTD_c_checksumFlag: 201, ZSTD_c_compressionLevel: 100, ZSTD_c_contentSizeFlag: 200, ZSTD_c_dictIDFlag: 202,
  ZSTD_c_enableLongDistanceMatching: 160, ZSTD_c_hashLog: 102, ZSTD_c_jobSize: 401,
  ZSTD_c_ldmBucketSizeLog: 163, ZSTD_c_ldmHashLog: 161, ZSTD_c_ldmHashRateLog: 164, ZSTD_c_ldmMinMatch: 162,
  ZSTD_c_minMatch: 105, ZSTD_c_nbWorkers: 400, ZSTD_c_overlapLog: 402, ZSTD_c_searchLog: 104,
  ZSTD_c_strategy: 107, ZSTD_c_targetLength: 106, ZSTD_c_windowLog: 101, ZSTD_d_windowLogMax: 100,
  ZSTD_dfast: 2, ZSTD_e_continue: 0, ZSTD_e_end: 2, ZSTD_e_flush: 1, ZSTD_error_GENERIC: 1,
  ZSTD_error_checksum_wrong: 22, ZSTD_error_corruption_detected: 20, ZSTD_error_dictionaryCreation_failed: 34,
  ZSTD_error_dictionary_corrupted: 30, ZSTD_error_dictionary_wrong: 32, ZSTD_error_dstBuffer_null: 74,
  ZSTD_error_dstSize_tooSmall: 70, ZSTD_error_frameParameter_unsupported: 14, ZSTD_error_frameParameter_windowTooLarge: 16,
  ZSTD_error_init_missing: 62, ZSTD_error_literals_headerWrong: 24, ZSTD_error_maxSymbolValue_tooLarge: 46,
  ZSTD_error_maxSymbolValue_tooSmall: 48, ZSTD_error_memory_allocation: 64, ZSTD_error_noForwardProgress_destFull: 80,
  ZSTD_error_noForwardProgress_inputEmpty: 82, ZSTD_error_no_error: 0, ZSTD_error_parameter_combination_unsupported: 41,
  ZSTD_error_parameter_outOfBound: 42, ZSTD_error_parameter_unsupported: 40, ZSTD_error_prefix_unknown: 10,
  ZSTD_error_srcSize_wrong: 72, ZSTD_error_stabilityCondition_notRespected: 50, ZSTD_error_stage_wrong: 60,
  ZSTD_error_tableLog_tooLarge: 44, ZSTD_error_version_unsupported: 12, ZSTD_error_workSpace_tooSmall: 66,
  ZSTD_fast: 1, ZSTD_greedy: 3, ZSTD_lazy: 4, ZSTD_lazy2: 5,
};

export const {
  Z_NO_FLUSH, Z_PARTIAL_FLUSH, Z_SYNC_FLUSH, Z_FULL_FLUSH, Z_FINISH, Z_BLOCK, Z_OK, Z_STREAM_END, Z_NEED_DICT,
  Z_ERRNO, Z_STREAM_ERROR, Z_DATA_ERROR, Z_MEM_ERROR, Z_BUF_ERROR, Z_VERSION_ERROR, Z_DEFAULT_COMPRESSION, Z_DEFAULT_LEVEL,
  Z_MIN_LEVEL, Z_MAX_LEVEL, Z_FILTERED, Z_HUFFMAN_ONLY, Z_RLE, Z_FIXED,
  Z_BEST_COMPRESSION, Z_BEST_SPEED, Z_DEFAULT_CHUNK, Z_DEFAULT_MEMLEVEL, Z_DEFAULT_STRATEGY, Z_DEFAULT_WINDOWBITS,
  Z_MAX_CHUNK, Z_MAX_MEMLEVEL, Z_MAX_WINDOWBITS, Z_MIN_CHUNK, Z_MIN_MEMLEVEL, Z_MIN_WINDOWBITS, Z_NO_COMPRESSION,
  GUNZIP, GZIP, INFLATE, DEFLATE, INFLATERAW, DEFLATERAW, UNZIP, BROTLI_OPERATION_PROCESS, BROTLI_OPERATION_FLUSH,
  BROTLI_OPERATION_FINISH, BROTLI_OPERATION_EMIT_METADATA, BROTLI_PARAM_MODE, BROTLI_PARAM_QUALITY, BROTLI_PARAM_SIZE_HINT,
  BROTLI_PARAM_DISABLE_LITERAL_CONTEXT_MODELING, BROTLI_PARAM_LGWIN, BROTLI_PARAM_LGBLOCK, BROTLI_PARAM_NPOSTFIX,
  BROTLI_PARAM_NDIRECT, BROTLI_MODE_GENERIC, BROTLI_MODE_TEXT, BROTLI_MODE_FONT, BROTLI_DEFAULT_MODE, BROTLI_DEFAULT_QUALITY,
  BROTLI_DEFAULT_WINDOW, BROTLI_MIN_QUALITY, BROTLI_MAX_QUALITY, BROTLI_MIN_WINDOW_BITS, BROTLI_MAX_WINDOW_BITS,
  BROTLI_LARGE_MAX_WINDOW_BITS, BROTLI_MIN_INPUT_BLOCK_BITS, BROTLI_MAX_INPUT_BLOCK_BITS, BROTLI_DECODER_RESULT_ERROR,
  BROTLI_DECODER_RESULT_NEEDS_MORE_INPUT, BROTLI_DECODER_RESULT_NEEDS_MORE_OUTPUT, BROTLI_DECODER_RESULT_SUCCESS,
  BROTLI_DECODER_NO_ERROR, BROTLI_DECODER_NEEDS_MORE_INPUT, BROTLI_DECODER_NEEDS_MORE_OUTPUT, BROTLI_DECODER_SUCCESS,
  BROTLI_DECODER_ERROR_INVALID_ARGUMENTS, BROTLI_DECODER_PARAM_DISABLE_RING_BUFFER_REALLOCATION, BROTLI_DECODER_PARAM_LARGE_WINDOW,
  BROTLI_DECODE, BROTLI_ENCODE, BROTLI_DECODER_ERROR_ALLOC_BLOCK_TYPE_TREES, BROTLI_DECODER_ERROR_ALLOC_CONTEXT_MAP,
  BROTLI_DECODER_ERROR_ALLOC_CONTEXT_MODES, BROTLI_DECODER_ERROR_ALLOC_RING_BUFFER_1, BROTLI_DECODER_ERROR_ALLOC_RING_BUFFER_2,
  BROTLI_DECODER_ERROR_ALLOC_TREE_GROUPS, BROTLI_DECODER_ERROR_DICTIONARY_NOT_SET, BROTLI_DECODER_ERROR_FORMAT_BLOCK_LENGTH_1,
  BROTLI_DECODER_ERROR_FORMAT_BLOCK_LENGTH_2, BROTLI_DECODER_ERROR_FORMAT_CL_SPACE, BROTLI_DECODER_ERROR_FORMAT_CONTEXT_MAP_REPEAT,
  BROTLI_DECODER_ERROR_FORMAT_DICTIONARY, BROTLI_DECODER_ERROR_FORMAT_DISTANCE, BROTLI_DECODER_ERROR_FORMAT_EXUBERANT_META_NIBBLE,
  BROTLI_DECODER_ERROR_FORMAT_EXUBERANT_NIBBLE, BROTLI_DECODER_ERROR_FORMAT_HUFFMAN_SPACE, BROTLI_DECODER_ERROR_FORMAT_PADDING_1,
  BROTLI_DECODER_ERROR_FORMAT_PADDING_2, BROTLI_DECODER_ERROR_FORMAT_RESERVED, BROTLI_DECODER_ERROR_FORMAT_SIMPLE_HUFFMAN_ALPHABET,
  BROTLI_DECODER_ERROR_FORMAT_SIMPLE_HUFFMAN_SAME, BROTLI_DECODER_ERROR_FORMAT_TRANSFORM, BROTLI_DECODER_ERROR_FORMAT_WINDOW_BITS,
  BROTLI_DECODER_ERROR_UNREACHABLE, ZLIB_VERNUM, ZSTD_CLEVEL_DEFAULT, ZSTD_COMPRESS, ZSTD_DECODE, ZSTD_DECOMPRESS, ZSTD_ENCODE,
  ZSTD_btlazy2, ZSTD_btopt, ZSTD_btultra, ZSTD_btultra2, ZSTD_c_chainLog, ZSTD_c_checksumFlag, ZSTD_c_compressionLevel,
  ZSTD_c_contentSizeFlag, ZSTD_c_dictIDFlag, ZSTD_c_enableLongDistanceMatching, ZSTD_c_hashLog, ZSTD_c_jobSize,
  ZSTD_c_ldmBucketSizeLog, ZSTD_c_ldmHashLog, ZSTD_c_ldmHashRateLog, ZSTD_c_ldmMinMatch, ZSTD_c_minMatch, ZSTD_c_nbWorkers,
  ZSTD_c_overlapLog, ZSTD_c_searchLog, ZSTD_c_strategy, ZSTD_c_targetLength, ZSTD_c_windowLog, ZSTD_d_windowLogMax, ZSTD_dfast,
  ZSTD_e_continue, ZSTD_e_end, ZSTD_e_flush, ZSTD_error_GENERIC, ZSTD_error_checksum_wrong, ZSTD_error_corruption_detected,
  ZSTD_error_dictionaryCreation_failed, ZSTD_error_dictionary_corrupted, ZSTD_error_dictionary_wrong, ZSTD_error_dstBuffer_null,
  ZSTD_error_dstSize_tooSmall, ZSTD_error_frameParameter_unsupported, ZSTD_error_frameParameter_windowTooLarge, ZSTD_error_init_missing,
  ZSTD_error_literals_headerWrong, ZSTD_error_maxSymbolValue_tooLarge, ZSTD_error_maxSymbolValue_tooSmall, ZSTD_error_memory_allocation,
  ZSTD_error_noForwardProgress_destFull, ZSTD_error_noForwardProgress_inputEmpty, ZSTD_error_no_error,
  ZSTD_error_parameter_combination_unsupported, ZSTD_error_parameter_outOfBound, ZSTD_error_parameter_unsupported,
  ZSTD_error_prefix_unknown, ZSTD_error_srcSize_wrong, ZSTD_error_stabilityCondition_notRespected, ZSTD_error_stage_wrong,
  ZSTD_error_tableLog_tooLarge, ZSTD_error_version_unsupported, ZSTD_error_workSpace_tooSmall, ZSTD_fast, ZSTD_greedy, ZSTD_lazy,
  ZSTD_lazy2,
} = constants;

const exportedConstants = Object.fromEntries(
  Object.entries(constants).filter(([name]) => !["Z_BINARY", "Z_DEFLATED", "Z_TEXT", "Z_TREES", "Z_UNKNOWN"].includes(name)),
);

export function crc32(input: unknown, value: unknown = 0): number {
  if (typeof value !== "number") throw invalidType("value", "number");
  if (!Number.isInteger(value) || value < 0 || value > 0xffffffff) throw outOfRange("value");
  let crc = (value ^ -1) >>> 0;
  for (const byte of inputBytes(input)) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit += 1) crc = (crc >>> 1) ^ (crc & 1 ? 0xedb88320 : 0);
  }
  return (crc ^ -1) >>> 0;
}

export default {
  Deflate, Inflate, Gzip, Gunzip, DeflateRaw, InflateRaw, Unzip, BrotliCompress, BrotliDecompress, ZstdCompress, ZstdDecompress,
  gzip, gunzip, deflate, inflate, deflateRaw, inflateRaw, gzipSync, gunzipSync, deflateSync, inflateSync, deflateRawSync,
  inflateRawSync, unzip, unzipSync, createGzip, createGunzip, createDeflate, createInflate, createDeflateRaw, createInflateRaw, createUnzip,
  brotliCompress, brotliCompressSync, brotliDecompress, brotliDecompressSync, createBrotliCompress, createBrotliDecompress,
  zstdCompress, zstdCompressSync, zstdDecompress, zstdDecompressSync, createZstdCompress, createZstdDecompress, crc32, codes, constants,
  ...exportedConstants,
};
