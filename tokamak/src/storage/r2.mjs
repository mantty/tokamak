import {
  r2AbortUpload, r2CloseBody, r2CompleteUpload, r2CreateUpload, r2Delete, r2Get, r2Head, r2List,
  r2ObjectWriter, r2PartWriter, r2Put, r2Read, r2UploadPart, r2Write,
} from "tokamak:storage";
import { bytes, consumeStream } from "../network/fetch.mjs";
import { isDisturbed, nativeReadableStream, setStreamLength, streamLength } from "../streams/web.mjs";

// Bytes each native write takes at most.
const WRITE_SIZE = 1024 * 1024;
const VALUE_TYPES = "JsReadableStream or ArrayBuffer or ArrayBufferView or string or Blob";
const UNKNOWN_LENGTH = "Provided readable stream must have a known length (request/response body or readable half of FixedLengthStream)";
const BODY_USED = "Body has already been used. It can only be used once. Use tee() first if you need to read it twice.";
const ENCRYPTION_KEYS = "Customer-provided encryption keys (ssecKey) are not supported: device storage is encrypted by the operating system.";
const PUT_CANCELLED = "Stream cancelled because the associated put operation encountered an error.";
const NOT_BYTES = "This ReadableStream did not return bytes.";
const NO_ETAG = "Comma was expected to separate etags.";
// Checksum fields, their lengths in bytes, and their names in messages.
const HASHES = [["md5", 16, "MD5"], ["sha1", 20, "SHA-1"], ["sha256", 32, "SHA-256"], ["sha384", 48, "SHA-384"], ["sha512", 64, "SHA-512"]];
// HTTP metadata fields, and the headers that carry them.
const HTTP_FIELDS = [
  ["contentType", "content-type"],
  ["contentLanguage", "content-language"],
  ["contentDisposition", "content-disposition"],
  ["contentEncoding", "content-encoding"],
  ["cacheControl", "cache-control"],
];

// Run the native work of `action`, reporting its failure as the binding does.
async function native(action, work) {
  try {
    return await work();
  } catch (error) {
    throw new Error(`${action}: ${error?.message ?? error}`);
  }
}

function parameterError(method, owner, index, type) {
  return new TypeError(`Failed to execute '${method}' on '${owner}': parameter ${index} is not of type '${type}'.`);
}

function fieldError(field, struct, type) {
  return new TypeError(`Incorrect type for the '${field}' field on '${struct}': the provided value is not of type '${type}'.`);
}

// A string parameter, converted as the runtime converts one.
function stringParameter(value, method, owner, index) {
  if (value === undefined) throw parameterError(method, owner, index, "string");
  return String(value);
}

// Options of `method`, or none when absent.
function options(value, method, index, type) {
  if (value == null) return {};
  if (typeof value !== "object") throw parameterError(method, "R2Bucket", index, type);
  return value;
}

// An optional object field, or none when absent.
function objectField(settings, field, struct, type) {
  const value = settings[field];
  if (value == null) return undefined;
  if (typeof value !== "object") throw fieldError(field, struct, type);
  return value;
}

// An optional field that must already be a string.
function stringField(settings, field, struct) {
  const value = settings[field];
  if (value === undefined) return undefined;
  if (typeof value !== "string") throw fieldError(field, struct, "string");
  return value;
}

// An optional date field, as milliseconds since the epoch.
function dateField(settings, field, struct) {
  const value = settings[field];
  if (value === undefined) return undefined;
  if (value instanceof Date) return value.getTime();
  if (typeof value === "number") return value;
  throw fieldError(field, struct, "date");
}

// A number converted to a 32-bit integer, as the runtime converts one.
function int32(value) {
  const number = Number(value);
  if (Number.isNaN(number)) return 0;
  const integer = Math.trunc(number);
  if (integer < -(2 ** 31) || integer > 2 ** 31 - 1) {
    throw new TypeError("Value out of range. Must be between -2147483648 and 2147483647 (inclusive).");
  }
  return integer;
}

function rejectEncryptionKey(settings) {
  if (settings.ssecKey !== undefined) throw new TypeError(ENCRYPTION_KEYS);
}

function hex(data) {
  return Array.from(data, byte => byte.toString(16).padStart(2, "0")).join("");
}

function viewBytes(value) {
  if (value instanceof ArrayBuffer) return new Uint8Array(value);
  if (ArrayBuffer.isView(value)) return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
  throw new TypeError(NOT_BYTES);
}

// The etags a conditional header lists: all of them, or a wildcard alone.
function headerEtags(header) {
  const etags = [];
  let rest = header;
  let separated = true;
  while (true) {
    rest = rest.replace(/^[ \t]+/, "");
    const next = rest[0];
    if (next === undefined) return etags;
    if (next === ",") {
      rest = rest.slice(1);
      separated = true;
      continue;
    }
    if (next === "*") {
      if (!separated) throw new Error(`${NO_ETAG} Encountered a wildcard character '*' instead.`);
      return [{ type: "wildcard" }];
    }
    if (next === "W") {
      if (!separated) throw new Error(`${NO_ETAG} Encountered a weak quotation character 'W' instead. This would otherwise indicate the start of a new weak etag.`);
      if (!rest.startsWith("W/\"")) throw new Error("Weak etags must start with W/ and their value must be quoted");
      rest = rest.slice(3);
    } else if (next === "\"") {
      if (!separated) throw new Error(`${NO_ETAG} Encountered a double quote character '"' instead. This would otherwise indicate the start of a new strong etag.`);
      rest = rest.slice(1);
    } else {
      return etags;
    }
    const end = rest.indexOf("\"");
    if (end === -1) throw new Error("Unclosed double quote for Etag");
    etags.push({ type: next === "W" ? "weak" : "strong", value: rest.slice(0, end) });
    rest = rest.slice(end + 1);
    separated = false;
  }
}

function headerConditional(headers) {
  const etags = name => {
    const header = headers.get(name);
    if (header === null) return undefined;
    const list = headerEtags(header);
    if (list.length === 0) throw new Error(`Invalid ETag in ${name} header`);
    return list;
  };
  const date = name => {
    const header = headers.get(name);
    return header === null ? undefined : Date.parse(header);
  };
  return {
    etagMatches: etags("if-match"),
    etagDoesNotMatch: etags("if-none-match"),
    uploadedAfter: date("if-modified-since"),
    uploadedBefore: date("if-unmodified-since"),
    secondsGranularity: true,
  };
}

// The conditions of `onlyIf`, an object or conditional headers.
function conditional(settings, struct) {
  const onlyIf = objectField(settings, "onlyIf", struct, "Conditional or Headers");
  if (onlyIf === undefined) return undefined;
  if (onlyIf instanceof Headers) return headerConditional(onlyIf);
  const etag = field => {
    const value = stringField(onlyIf, field, "Conditional");
    if (value === undefined) return undefined;
    if (value.startsWith("\"") && value.endsWith("\"")) throw new TypeError(`Conditional ETag should not be wrapped in quotes (${value}).`);
    return [value === "*" ? { type: "wildcard" } : { type: "strong", value }];
  };
  return {
    etagMatches: etag("etagMatches"),
    etagDoesNotMatch: etag("etagDoesNotMatch"),
    uploadedBefore: dateField(onlyIf, "uploadedBefore", "Conditional"),
    uploadedAfter: dateField(onlyIf, "uploadedAfter", "Conditional"),
    secondsGranularity: Boolean(onlyIf.secondsGranularity),
  };
}

function rangeNumber(value, label, name) {
  if (value === undefined) return undefined;
  const number = Number(value);
  if (!(number >= 0)) throw new RangeError(`${label} (${number}) must be greater than or equal to 0.`);
  if (!Number.isInteger(number)) throw new RangeError(`Invalid range. ${name} (${number}) must be an integer, not floating point.`);
  return number;
}

// The range a read asks for, as an offset and length, a suffix, or a
// `Range` header.
function range(settings) {
  const value = objectField(settings, "range", "GetOptions", "Range or Headers");
  if (value === undefined) return {};
  if (value instanceof Headers) {
    const header = value.get("range");
    return header === null ? {} : { rangeHeader: header };
  }
  const bounds = {
    offset: rangeNumber(value.offset, "Invalid range. Starting offset", "Starting offset"),
    length: rangeNumber(value.length, "Invalid range. Length", "Length"),
  };
  if (value.suffix !== undefined) {
    if (bounds.offset !== undefined) throw new TypeError("Suffix is incompatible with offset.");
    if (bounds.length !== undefined) throw new TypeError("Suffix is incompatible with length.");
    bounds.suffix = rangeNumber(value.suffix, "Invalid suffix. Suffix", "Suffix");
  }
  return { range: bounds };
}

// HTTP metadata from an object of fields, or from request headers.
function httpFields(settings, struct) {
  const value = objectField(settings, "httpMetadata", struct, "HttpMetadata or Headers");
  const fields = {};
  if (value === undefined) return fields;
  if (value instanceof Headers) {
    for (const [field, header] of HTTP_FIELDS) {
      const text = value.get(header);
      if (text !== null) fields[field] = text;
    }
    const expires = value.get("expires");
    if (expires !== null) fields.cacheExpiry = Date.parse(expires);
    return fields;
  }
  for (const [field] of HTTP_FIELDS) {
    if (value[field] !== undefined) fields[field] = String(value[field]);
  }
  const expiry = dateField(value, "cacheExpiry", "HttpMetadata");
  if (expiry !== undefined) fields.cacheExpiry = expiry;
  return fields;
}

function customFields(settings, struct) {
  const value = settings.customMetadata;
  if (value === undefined) return {};
  if (value === null || typeof value !== "object" || Array.isArray(value)) throw fieldError("customMetadata", struct, "object");
  return Object.fromEntries(Object.entries(value).map(([name, field]) => [name, String(field)]));
}

// The metadata a write of `struct` options gives an object.
function metadata(settings, struct) {
  const fields = { customMetadata: customFields(settings, struct), httpMetadata: httpFields(settings, struct) };
  if (settings.storageClass !== undefined) fields.storageClass = String(settings.storageClass);
  rejectEncryptionKey(settings);
  return fields;
}

// The one checksum `settings` provides, in lower-case hex.
function checksum(settings) {
  let provided;
  for (const [field, length, name] of HASHES) {
    const value = settings[field];
    if (value === undefined) continue;
    if (provided !== undefined) throw new TypeError("You cannot specify multiple hashing algorithms.");
    provided = { algorithm: field, value: checksumValue(value, field, length, name) };
  }
  return provided;
}

function checksumValue(value, field, length, name) {
  if (typeof value === "string") {
    if (value.length !== length * 2) throw new TypeError(`${name} is ${length * 2} hex characters, not ${value.length}`);
    if (!/^[0-9a-fA-F]*$/.test(value)) throw new TypeError(`Provided ${name} wasn't a valid hex string`);
    return value.toLowerCase();
  }
  if (!(value instanceof ArrayBuffer) && !ArrayBuffer.isView(value)) throw fieldError(field, "PutOptions", "BufferSource or string");
  const digest = viewBytes(value);
  if (digest.byteLength !== length) throw new TypeError(`${name} is ${length} bytes, not ${digest.byteLength}`);
  return hex(digest);
}

// A value to write: a copy of its bytes when in memory, or its stream.
function body(value, method, owner, optional) {
  if (optional && value == null) return { bytes: new Uint8Array(0) };
  if (typeof value === "string" || value instanceof ArrayBuffer || ArrayBuffer.isView(value) || value instanceof Blob) {
    return { bytes: bytes(value) };
  }
  if (value instanceof ReadableStream) return { stream: value };
  throw parameterError(method, owner, 2, VALUE_TYPES);
}

// The length of a value to write, which a stream must know.
function bodyLength(value) {
  if (value.bytes !== undefined) return value.bytes.byteLength;
  const length = streamLength(value.stream);
  if (length === undefined) throw new TypeError(UNKNOWN_LENGTH);
  return length;
}

// Write `value` to the native `writer`, `WRITE_SIZE` bytes at a time.
async function writeBody(action, writer, value) {
  const write = chunk => native(action, () => r2Write(writer, chunk));
  if (value.stream !== undefined) return writeStream(write, value.stream);
  for (let offset = 0; offset < value.bytes.byteLength; offset += WRITE_SIZE) {
    await write(value.bytes.subarray(offset, offset + WRITE_SIZE));
  }
}

// Write the chunks of `stream`, gathered into `WRITE_SIZE` bytes. `write`
// copies its chunk before it returns, so the buffer is reused.
async function writeStream(write, stream) {
  const buffer = new Uint8Array(WRITE_SIZE);
  let filled = 0;
  const reader = stream.getReader();
  try {
    for (let result = await reader.read(); !result.done; result = await reader.read()) {
      let chunk = viewBytes(result.value);
      while (filled + chunk.byteLength >= WRITE_SIZE) {
        const taken = WRITE_SIZE - filled;
        buffer.set(chunk.subarray(0, taken), filled);
        await write(buffer);
        filled = 0;
        chunk = chunk.subarray(taken);
      }
      buffer.set(chunk, filled);
      filled += chunk.byteLength;
    }
    if (filled > 0) await write(buffer.subarray(0, filled));
  } catch (error) {
    await reader.cancel(new Error(PUT_CANCELLED)).catch(() => {});
    throw error;
  } finally {
    reader.releaseLock();
  }
}

// A stream of the body `reader` reads, which delivers `length` bytes.
function bodyStream(reader, length) {
  const stream = nativeReadableStream({
    type: "bytes",
    async pull(controller) {
      const chunk = await r2Read(reader);
      if (chunk == null) {
        controller.close();
        controller.byobRequest?.respond(0);
      } else {
        controller.enqueue(chunk);
      }
    },
    cancel() { r2CloseBody(reader); },
  });
  return setStreamLength(stream, length);
}

function readonly(object, values) {
  for (const [name, value] of Object.entries(values)) {
    Object.defineProperty(object, name, { value, writable: false, enumerable: true, configurable: true });
  }
}

function tag(constructor, name) {
  Object.defineProperty(constructor.prototype, Symbol.toStringTag, { value: name, configurable: true });
}

class Checksums {
  #hex;

  constructor(hexes) {
    this.#hex = hexes;
    const buffer = value => (value === undefined ? undefined : Uint8Array.from(value.match(/../g) ?? [], byte => parseInt(byte, 16)).buffer);
    readonly(this, Object.fromEntries(HASHES.map(([field]) => [field, buffer(hexes[field])]).reverse()));
  }

  toJSON() {
    return Object.fromEntries(HASHES.map(([field]) => [field, this.#hex[field]]));
  }
}
tag(Checksums, "Checksums");

// An object's metadata, from the fields the store reports.
class HeadResult {
  constructor(fields) {
    let httpMetadata;
    if (fields.httpMetadata !== undefined) {
      httpMetadata = Object.fromEntries(HTTP_FIELDS.map(([field]) => [field, fields.httpMetadata[field]]));
      const expiry = fields.httpMetadata.cacheExpiry;
      httpMetadata.cacheExpiry = expiry === undefined ? undefined : new Date(expiry);
    }
    readonly(this, {
      ssecKeyMd5: undefined,
      storageClass: fields.storageClass,
      range: fields.range,
      customMetadata: fields.customMetadata,
      httpMetadata,
      uploaded: new Date(fields.uploaded),
      checksums: new Checksums(fields.checksums),
      httpEtag: `"${fields.etag}"`,
      etag: fields.etag,
      size: fields.size,
      version: fields.version,
      key: fields.key,
    });
  }

  writeHttpMetadata(headers) {
    if (!(headers instanceof Headers)) throw parameterError("writeHttpMetadata", "HeadResult", 1, "Headers");
    const metadata = this.httpMetadata;
    if (metadata === undefined) {
      throw new TypeError(`HTTP metadata unknown for key \`${this.key}\`. Did you forget to add 'httpMetadata' to \`include\` when listing?`);
    }
    for (const [field, header] of HTTP_FIELDS) {
      if (metadata[field] !== undefined) headers.set(header, metadata[field]);
    }
    if (metadata.cacheExpiry !== undefined) headers.set("expires", metadata.cacheExpiry.toUTCString());
  }
}
tag(HeadResult, "HeadResult");

// An object's metadata and body.
class GetResult extends HeadResult {
  #body;

  constructor(fields, reader) {
    super(fields);
    this.#body = bodyStream(reader, fields.range.length);
  }

  get body() { return this.#body; }
  get bodyUsed() { return isDisturbed(this.#body); }

  async arrayBuffer() { return (await this.#read()).buffer; }
  async bytes() { return this.#read(); }
  async text() { return new TextDecoder().decode(await this.#read()); }
  async json() { return JSON.parse(await this.text()); }
  async blob() { return new Blob([await this.#read()], { type: this.httpMetadata.contentType ?? "" }); }

  async #read() {
    if (isDisturbed(this.#body) || this.#body.locked) throw new TypeError(BODY_USED);
    return consumeStream(this.#body);
  }
}
for (const name of ["body", "bodyUsed"]) {
  Object.defineProperty(GetResult.prototype, name, { ...Object.getOwnPropertyDescriptor(GetResult.prototype, name), enumerable: true });
}
tag(GetResult, "GetResult");

export class R2MultipartUpload {
  #binding;

  constructor(binding, key, uploadId) {
    this.#binding = binding;
    readonly(this, { uploadId, key });
  }

  async uploadPart(partNumber, value, partOptions) {
    const number = int32(partNumber);
    const part = body(value, "uploadPart", "R2MultipartUpload", false);
    validatePartNumber(number);
    rejectEncryptionKey(partOptions ?? {});
    const length = bodyLength(part);
    const writer = await native("uploadPart", () => r2PartWriter(this.#binding, length));
    await writeBody("uploadPart", writer, part);
    const etag = await native("uploadPart", () => r2UploadPart(this.#binding, this.key, this.uploadId, number, writer));
    return { partNumber: number, etag };
  }

  async abort() {
    await native("abortMultipartUpload", () => r2AbortUpload(this.#binding, this.key, this.uploadId));
  }

  async complete(uploadedParts) {
    if (!Array.isArray(uploadedParts)) throw parameterError("complete", "R2MultipartUpload", 1, "Array");
    const parts = uploadedParts.map(part => ({ partNumber: int32(part?.partNumber), etag: String(part?.etag) }));
    for (const part of parts) validatePartNumber(part.partNumber);
    const object = await native("completeMultipartUpload", () => r2CompleteUpload(this.#binding, this.key, this.uploadId, JSON.stringify(parts)));
    return new HeadResult(JSON.parse(object));
  }
}
tag(R2MultipartUpload, "R2MultipartUpload");

function validatePartNumber(number) {
  if (number < 1 || number > 10000) throw new TypeError(`Part number must be between 1 and 10000 (inclusive). Actual value was: ${number}`);
}

export class R2Bucket {
  #binding;

  constructor(binding) {
    this.#binding = binding;
  }

  async head(key) {
    key = stringParameter(key, "head", "R2Bucket", 1);
    const object = await native("head", () => r2Head(this.#binding, key));
    return object == null ? null : new HeadResult(JSON.parse(object));
  }

  async get(key, getOptions) {
    key = stringParameter(key, "get", "R2Bucket", 1);
    const settings = options(getOptions, "get", 2, "GetOptions");
    const request = { onlyIf: conditional(settings, "GetOptions"), ...range(settings) };
    rejectEncryptionKey(settings);
    const result = await native("get", () => r2Get(this.#binding, key, JSON.stringify(request)));
    if (result == null) return null;
    const fields = JSON.parse(result.object);
    return result.body == null ? new HeadResult(fields) : new GetResult(fields, result.body);
  }

  async put(key, value, putOptions) {
    key = stringParameter(key, "put", "R2Bucket", 1);
    const object = body(value, "put", "R2Bucket", true);
    const settings = options(putOptions, "put", 3, "PutOptions");
    const onlyIf = conditional(settings, "PutOptions");
    const fields = metadata(settings, "PutOptions");
    const provided = checksum(settings);
    const length = bodyLength(object);
    const writer = await native("put", () => r2ObjectWriter(this.#binding, length, provided === undefined ? undefined : JSON.stringify(provided)));
    await writeBody("put", writer, object);
    const stored = await native("put", () => r2Put(this.#binding, key, writer, JSON.stringify({ onlyIf, metadata: fields })));
    return stored == null ? null : new HeadResult(JSON.parse(stored));
  }

  async delete(keys) {
    if (keys === undefined) throw parameterError("delete", "R2Bucket", 1, "string or Array");
    const list = Array.isArray(keys) ? keys.map(String) : [String(keys)];
    await native("delete", () => r2Delete(this.#binding, list));
  }

  async list(listOptions) {
    const settings = options(listOptions, "list", 1, "ListOptions");
    const request = {};
    if (settings.limit !== undefined) {
      const limit = int32(settings.limit) >>> 0;
      if (limit !== 0xffffffff) request.limit = limit;
    }
    for (const field of ["prefix", "cursor", "delimiter", "startAfter"]) {
      const value = stringField(settings, field, "ListOptions");
      if (value !== undefined) request[field] = value;
    }
    if (settings.include !== undefined) {
      if (!Array.isArray(settings.include)) throw fieldError("include", "ListOptions", "Array");
      request.include = settings.include.map((value, index) => {
        if (typeof value !== "string") throw new TypeError(`Incorrect type for array element ${index}: the provided value is not of type 'string'.`);
        if (value !== "httpMetadata" && value !== "customMetadata") throw new RangeError(`Unsupported include value ${value}`);
        return value;
      });
    }
    const page = JSON.parse(await native("list", () => r2List(this.#binding, JSON.stringify(request))));
    return {
      objects: page.objects.map(fields => new HeadResult(fields)),
      truncated: page.truncated,
      cursor: page.cursor,
      delimitedPrefixes: page.delimitedPrefixes,
    };
  }

  async createMultipartUpload(key, uploadOptions) {
    key = stringParameter(key, "createMultipartUpload", "R2Bucket", 1);
    const fields = metadata(options(uploadOptions, "createMultipartUpload", 2, "MultipartOptions"), "MultipartOptions");
    const uploadId = await native("createMultipartUpload", () => r2CreateUpload(this.#binding, key, JSON.stringify(fields)));
    return new R2MultipartUpload(this.#binding, key, uploadId);
  }

  resumeMultipartUpload(key, uploadId) {
    return new R2MultipartUpload(
      this.#binding,
      stringParameter(key, "resumeMultipartUpload", "R2Bucket", 1),
      stringParameter(uploadId, "resumeMultipartUpload", "R2Bucket", 2),
    );
  }
}
tag(R2Bucket, "R2Bucket");
