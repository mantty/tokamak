// Storage binding behaviour compared between tokamak and local Cloudflare.
// Values that differ by implementation (timings, sizes, bookmarks) are
// reduced to their types.

import { byteOutcomes, bytesOf } from "./bytes.mjs";

async function outcome(callback) {
  try {
    return await callback();
  } catch (error) {
    return { error: String(error?.message ?? error), name: error?.name ?? null };
  }
}

function meta(value) {
  if (value === null || typeof value !== "object") return value;
  const { duration, size_after, served_by, served_by_region, served_by_primary, timings, ...stable } = value;
  return { ...stable, duration: typeof duration, size_after: typeof size_after };
}

function result(value) {
  if (value === null || typeof value !== "object") return value;
  if ("count" in value && "duration" in value) return { ...value, duration: typeof value.duration };
  if (!("meta" in value)) return value;
  return { ...value, meta: meta(value.meta) };
}

async function d1Contracts(db) {
  const results = {};
  const record = async (name, callback) => {
    const value = await outcome(callback);
    results[name] = Array.isArray(value) ? value.map(result) : result(value);
  };
  await record("exec", () => db.exec("CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT NOT NULL, age REAL, data BLOB)\nCREATE INDEX people_name ON people (name)"));
  await record("insert", () => db.prepare("INSERT INTO people (name, age, data) VALUES (?1, ?2, ?3)").bind("ada", 36, new Uint8Array([1, 2, 3])).run());
  await record("insertTypes", () => db.prepare("INSERT INTO people (name, age, data) VALUES (?, ?, ?)").bind("bob", true, [4, 5]).run());
  await record("insertBuffer", () => db.prepare("INSERT INTO people (name, age, data) VALUES (?, ?, ?)").bind("cy", 1.5, new Uint8Array([6]).buffer).run());
  await record("insertNull", () => db.prepare("INSERT INTO people (name, age, data) VALUES (?, ?, ?)").bind("di", null, null).run());
  await record("all", () => db.prepare("SELECT * FROM people ORDER BY id").all());
  await record("indexed", () => db.prepare("SELECT id FROM people WHERE name = ?").bind("bob").all());
  await record("first", () => db.prepare("SELECT name, age FROM people WHERE id = ?").bind(1).first());
  await record("firstColumn", () => db.prepare("SELECT name FROM people WHERE id = 2").first("name"));
  await record("firstMissingRow", () => db.prepare("SELECT name FROM people WHERE id = 99").first());
  await record("firstMissingColumn", () => db.prepare("SELECT name FROM people WHERE id = 1").first("age"));
  await record("raw", () => db.prepare("SELECT id, name FROM people ORDER BY id LIMIT 2").raw());
  await record("rawColumns", () => db.prepare("SELECT id, name FROM people ORDER BY id LIMIT 1").raw({ columnNames: true }));
  await record("types", () => db.prepare("SELECT typeof(?) AS a, typeof(?) AS b, typeof(?) AS c, ? AS d").bind(1, 1.5, "x", 9007199254740993).first());
  await record("values", () => db.prepare("SELECT 9223372036854775807 AS big, 1e308 * 10 AS inf, x'00ff' AS blob, json_object('a', 1) AS json").first());
  await record("duplicateColumns", () => db.prepare("SELECT 1 AS x, 2 AS x").all());
  await record("update", () => db.prepare("UPDATE people SET age = age + 1 WHERE age IS NOT NULL").run());
  await record("delete", () => db.prepare("DELETE FROM people WHERE name = ?").bind("di").run());
  await record("batch", () => db.batch([
    db.prepare("INSERT INTO people (name) VALUES (?)").bind("eve"),
    db.prepare("SELECT count(*) AS count FROM people"),
  ]));
  await record("batchRollback", () => db.batch([
    db.prepare("INSERT INTO people (name) VALUES (?)").bind("fay"),
    db.prepare("INSERT INTO people (id, name) VALUES (1, 'duplicate')"),
  ]));
  await record("afterRollback", () => db.prepare("SELECT count(*) AS count FROM people WHERE name = 'fay'").first("count"));
  await record("multiStatement", () => db.prepare("INSERT INTO people (name) VALUES ('gus'); SELECT name FROM people WHERE name = ?").bind("gus").all());
  await record("multiStatementParams", () => db.prepare("SELECT ?; SELECT 1").bind(1).all());
  await record("wrongParams", () => db.prepare("SELECT ?, ?").bind(1).all());
  await record("syntaxError", () => db.prepare("SELEKT 1").all());
  await record("missingTable", () => db.prepare("SELECT * FROM missing").all());
  await record("constraint", () => db.prepare("INSERT INTO people (id, name) VALUES (1, 'x')").run());
  await record("notNull", () => db.prepare("INSERT INTO people (name) VALUES (NULL)").run());
  await record("begin", () => db.prepare("BEGIN TRANSACTION").run());
  await record("attach", () => db.prepare("ATTACH DATABASE 'x' AS x").run());
  await record("version", () => db.prepare("SELECT sqlite_version()").all());
  await record("reserved", () => db.prepare("CREATE TABLE _cf_mine (x)").run());
  await record("temporary", () => db.prepare("CREATE TEMP TABLE scratch (x)").run());
  await record("objectParam", () => db.prepare("SELECT ?").bind({}).all());
  await record("emptyQuery", () => db.prepare("").all());
  await record("commentQuery", () => db.prepare("-- nothing").all());
  await record("indentedComment", () => db.prepare("  -- nothing").all());
  await record("trailingComment", () => db.prepare("SELECT 1 AS one; -- done").all());
  await record("execError", () => db.exec("SELECT 1\nSELECT * FROM missing"));
  await record("execCount", () => db.exec("INSERT INTO people (name) VALUES ('hal')\nINSERT INTO people (name) VALUES ('ivy')"));
  await record("dump", () => db.dump());
  await record("pragma", () => db.prepare("PRAGMA table_info(people)").all());
  await record("pragmaRefused", () => db.prepare("PRAGMA journal_mode").all());
  await record("json", () => db.prepare("SELECT json_group_array(name) AS names FROM people WHERE id <= 2").first("names"));
  await record("fts", async () => {
    await db.exec("CREATE VIRTUAL TABLE notes USING fts5(body)");
    await db.prepare("INSERT INTO notes VALUES (?), (?)").bind("hello world", "goodbye").run();
    return db.prepare("SELECT body FROM notes WHERE notes MATCH 'hello'").all();
  });
  await record("session", async () => {
    const session = db.withSession("first-primary");
    const before = session.getBookmark();
    await session.prepare("INSERT INTO people (name) VALUES ('jo')").run();
    const after = session.getBookmark();
    const read = await session.prepare("SELECT count(*) AS count FROM people WHERE name = 'jo'").first("count");
    return { before, after: typeof after, read, constraint: db.withSession().getBookmark() };
  });
  await record("sessionBatch", async () => {
    const session = db.withSession("first-unconstrained");
    const rows = await session.batch([session.prepare("SELECT 1 AS one")]);
    return { rows: rows.map(result), bookmark: typeof session.getBookmark() };
  });
  await record("shape", () => ({
    database: Object.getOwnPropertyNames(Object.getPrototypeOf(db)).sort(),
    statement: Object.getOwnPropertyNames(Object.getPrototypeOf(db.prepare("SELECT 1"))).sort(),
    session: Object.getOwnPropertyNames(Object.getPrototypeOf(db.withSession())).sort(),
  }));
  return results;
}

function streamOf(...chunks) {
  return new ReadableStream({ start(controller) { for (const chunk of chunks) controller.enqueue(chunk); controller.close(); } });
}

async function streamText(stream) {
  return new Response(stream).text();
}

function mapEntries(value) {
  return value instanceof Map ? { map: [...value.entries()] } : value;
}

async function kvContracts(kv) {
  const results = {};
  const record = async (name, callback) => { results[name] = mapEntries(await outcome(callback)); };
  const now = () => Math.floor(Date.now() / 1000);
  await record("putText", () => kv.put("text", "héllo"));
  await record("getText", () => kv.get("text"));
  await record("getTypes", async () => ({
    text: await kv.get("text", "text"),
    buffer: [...new Uint8Array(await kv.get("text", "arrayBuffer"))],
    stream: await streamText(await kv.get("text", { type: "stream" })),
    missing: await kv.get("missing"),
  }));
  await record("putJson", () => kv.put("json", JSON.stringify({ a: [1, 2] }), { metadata: { tag: "t", n: 1 } }));
  await record("getJson", () => kv.get("json", { type: "json", cacheTtl: 60 }));
  await record("withMetadata", () => kv.getWithMetadata("json", "json"));
  await record("withoutMetadata", () => kv.getWithMetadata("text"));
  await record("missingWithMetadata", () => kv.getWithMetadata("missing"));
  await record("badJson", () => kv.get("text", "json").then(() => "parsed", error => error.name));
  await record("unknownType", () => kv.get("text", "blob"));
  await record("unknownTypeMissing", () => kv.get("missing", "blob"));
  await record("cacheTtl", () => kv.get("text", { cacheTtl: 10 }));
  await record("putBuffer", async () => {
    await kv.put("buffer", new Uint8Array([0, 255, 1]).buffer);
    await kv.put("view", new Uint8Array([9, 8, 7, 6]).subarray(1, 3));
    await kv.put("stream", streamOf(new TextEncoder().encode("ab"), new TextEncoder().encode("cd")));
    await kv.put("number", 42);
    return [
      [...new Uint8Array(await kv.get("buffer", "arrayBuffer"))],
      [...new Uint8Array(await kv.get("view", "arrayBuffer"))],
      await kv.get("stream"),
      await kv.get("number"),
      await kv.get("buffer"),
    ];
  });
  await record("putObject", () => kv.put("object", {}));
  await record("emptyKey", () => kv.get(""));
  await record("dotKey", () => kv.put(".", "x"));
  await record("dotDotKey", () => kv.delete(".."));
  await record("longKey", () => kv.get("é".repeat(257)));
  await record("longPutKey", () => kv.put("k".repeat(513), "x"));
  await record("specialKey", async () => { await kv.put("a/b?c#d%20 e", "special"); return kv.get("a/b?c#d%20 e"); });
  await record("ttlZero", () => kv.put("ttl", "x", { expirationTtl: 0 }));
  await record("ttlShort", () => kv.put("ttl", "x", { expirationTtl: 30 }));
  await record("expirationPast", () => kv.put("ttl", "x", { expiration: 1000 }));
  await record("expirationSoon", async () => {
    try { await kv.put("ttl", "x", { expiration: now() + 10 }); return "stored"; }
    catch (error) { return error.message.replace(/of \d+\./, "of N."); }
  });
  await record("metadataLarge", () => kv.put("meta", "x", { metadata: "m".repeat(1100) }));
  await record("valueLarge", () => kv.put("large", new Uint8Array(25 * 1024 * 1024 + 1)));
  await record("streamLarge", () => {
    let chunks = 0;
    return kv.put("large", new ReadableStream({ pull(controller) { if (++chunks > 26) controller.close(); else controller.enqueue(new Uint8Array(MIB)); } }));
  });
  await record("expiring", async () => {
    await kv.put("expiring", "x", { expirationTtl: 3600, metadata: [1] });
    await kv.put("absolute", "y", { expiration: now() + 120 });
    const { keys } = await kv.list({ prefix: "expir" });
    const absolute = (await kv.list({ prefix: "absolute" })).keys[0];
    return {
      keys: keys.map(({ expiration, ...key }) => ({ ...key, ttl: Math.abs(expiration - now() - 3600) <= 2 })),
      absolute: Math.abs(absolute.expiration - now() - 120) <= 2,
      value: await kv.get("expiring"),
    };
  });
  await record("delete", async () => { await kv.delete("text"); await kv.delete("never"); return kv.get("text"); });
  await record("list", async () => {
    for (const key of ["list/b", "list/a", "list/é", "list/z", "list/A", "list/\u{1F600}", "lisu"]) await kv.put(key, key, { metadata: key.length });
    return kv.list({ prefix: "list/" });
  });
  await record("listPages", async () => {
    const first = await kv.list({ prefix: "list/", limit: 2 });
    const second = await kv.list({ prefix: "list/", limit: 2, cursor: first.cursor });
    const rest = await kv.list({ prefix: "list/", cursor: second.cursor });
    return { first: { ...first, cursor: typeof first.cursor }, second: second.keys.map(key => key.name), rest: rest.keys.map(key => key.name), complete: rest.list_complete };
  });
  await record("listAll", async () => (await kv.list()).keys.map(key => key.name));
  await record("listLimitZero", async () => (await kv.list({ limit: 0, prefix: "list/" })).keys.length);
  await record("listLimitLarge", () => kv.list({ limit: 1001 }));
  await record("listPrefixLong", () => kv.list({ prefix: "p".repeat(513) }));
  await record("listNullOptions", async () => (await kv.list({ prefix: null, cursor: null })).list_complete);
  await record("bulk", async () => {
    await kv.put("1", "one");
    await kv.put("bulk-json", "{\"x\":1}", { metadata: { m: true } });
    return {
      text: mapEntries(await kv.get(["list/a", "missing", "1", "list/b", "list/a"])),
      json: mapEntries(await kv.get(["bulk-json", "missing"], "json")),
      metadata: mapEntries(await kv.getWithMetadata(["bulk-json", "list/a", "missing"], { type: "json" }).catch(error => error.message)),
      metadataText: mapEntries(await kv.getWithMetadata(["bulk-json", "missing"])),
    };
  });
  await record("bulkTooMany", () => kv.get(Array.from({ length: 101 }, (_, index) => `k${index}`)));
  await record("bulkEmpty", () => kv.get([]));
  await record("bulkType", () => kv.get(["list/a"], "arrayBuffer"));
  await record("bulkBadJson", () => kv.get(["list/a"], "json"));
  await record("bulkBadKey", () => kv.get(["ok", ""]));
  await record("invalidText", async () => { await kv.put("invalid", new Uint8Array([0xff, 0x61])); return kv.get("invalid"); });
  await record("valueBytes", () => byteOutcomes(async value => { await kv.put("bytes", value); return bytesOf(kv.get("bytes", "arrayBuffer")); }));
  await record("streamBytes", () => byteOutcomes(async value => { await kv.put("bytes", streamOf(value)); return bytesOf(kv.get("bytes", "arrayBuffer")); }));
  await record("shape", () => Object.getOwnPropertyNames(Object.getPrototypeOf(kv)).sort());
  return results;
}

const MIB = 1024 * 1024;

// An object's metadata as JSON, with its random version and upload time
// reduced to their types. Local Cloudflare reports no storage class, which
// Cloudflare and tokamak do, so it is left out.
function r2Object(object) {
  if (object === null || typeof object !== "object" || !("etag" in object)) return object;
  const { version, uploaded, storageClass, ...fields } = JSON.parse(JSON.stringify(object));
  return {
    ...fields,
    version: typeof object.version,
    uploaded: object.uploaded instanceof Date,
    type: Object.prototype.toString.call(object),
  };
}

async function r2Body(object) {
  if (object === null || !("body" in object)) return r2Object(object);
  return { ...r2Object(object), text: await object.text() };
}

// A listing's keys and prefixes, with the cursor reduced to the key it follows.
function r2Keys(listing) {
  return {
    objects: listing.objects.map(object => object.key),
    delimitedPrefixes: listing.delimitedPrefixes,
    truncated: listing.truncated,
    cursor: listing.cursor === undefined ? null : new TextDecoder().decode(Uint8Array.from(atob(listing.cursor), character => character.charCodeAt(0))),
  };
}

function sized(length, fill) {
  return new Uint8Array(length).fill(fill);
}

async function r2Contracts(r2) {
  const results = {};
  const record = async (name, callback) => { results[name] = await outcome(callback); };
  const written = await r2.put("a", "hello", { httpMetadata: { contentType: "text/plain", cacheExpiry: new Date(1000) }, customMetadata: { b: "1", a: "2", 10: 3 } });
  await record("put", () => r2Object(written));
  await record("valueBytes", () => byteOutcomes(async value => { await r2.put("bytes", value); return bytesOf((await r2.get("bytes")).arrayBuffer()); }));
  await record("shape", async () => {
    const object = await r2.get("a");
    const upload = await r2.createMultipartUpload("shape");
    return {
      bucket: Object.getOwnPropertyNames(Object.getPrototypeOf(r2)).sort(),
      object: Object.getOwnPropertyNames(written),
      head: Object.getOwnPropertyNames(Object.getPrototypeOf(written)).sort(),
      body: Object.getOwnPropertyNames(Object.getPrototypeOf(object)).sort(),
      checksums: Object.getOwnPropertyNames(written.checksums),
      upload: Object.getOwnPropertyNames(upload),
      uploadMethods: Object.getOwnPropertyNames(Object.getPrototypeOf(upload)).sort(),
      bucketType: Object.prototype.toString.call(r2),
      key: Object.getOwnPropertyDescriptor(written, "key"),
      checksumTypes: [written.checksums.md5 instanceof ArrayBuffer, written.checksums.sha1],
      md5: [...new Uint8Array(written.checksums.md5)],
    };
  });
  await record("head", async () => r2Object(await r2.head("a")));
  await record("metadataKeys", async () => {
    const object = await r2.head("a");
    return { http: Object.keys(object.httpMetadata), checksums: Object.keys(object.checksums.toJSON()), custom: Object.keys(object.customMetadata) };
  });
  await record("nullOptions", async () => ({
    get: (await r2.get("a", null)).key,
    getFields: (await r2.get("a", { range: null, onlyIf: null })).key,
    rangeOffset: (await r2.get("a", { range: { offset: null } })).range,
    put: (await r2.put("n", "x", null)).key,
    putHttp: (await r2.put("n", "x", { httpMetadata: null })).key,
    list: (await r2.list(null)).truncated,
    listLimit: await r2.list({ limit: null }).catch(error => error.message),
    upload: (await r2.createMultipartUpload("n", null)).key,
    deleteUndefined: await r2.delete([undefined]).then(() => "deleted"),
  }));
  const typeError = promise => promise.then(() => "accepted", error => [error.name, error.message]);
  await record("optionTypes", async () => ({
    get: await typeError(r2.get("a", "x")),
    put: await typeError(r2.put("a", "x", "x")),
    list: await typeError(r2.list("x")),
    onlyIf: await typeError(r2.get("a", { onlyIf: "x" })),
    range: await typeError(r2.get("a", { range: 5 })),
    customNull: await typeError(r2.put("t", "x", { customMetadata: null })),
    customArray: await typeError(r2.put("t", "x", { customMetadata: [1] })),
    customText: await typeError(r2.createMultipartUpload("t", { customMetadata: "ab" })),
    http: await typeError(r2.put("t", "x", { httpMetadata: "x" })),
    uploadHttp: await typeError(r2.createMultipartUpload("t", { httpMetadata: "x" })),
    md5: await typeError(r2.put("t", "x", { md5: null })),
    cacheExpiry: await typeError(r2.put("t", "x", { httpMetadata: { cacheExpiry: "2020" } })),
    include: await typeError(r2.list({ include: [5] })),
    cursor: await typeError(r2.list({ cursor: null })),
    delete: await typeError(r2.delete()),
    head: await typeError(r2.head()),
  }));
  await record("get", async () => r2Body(await r2.get("a")));
  await record("missing", async () => [await r2.head("missing"), await r2.get("missing"), await r2.get("missing", { onlyIf: { etagMatches: "x" } })]);
  await record("bodyTypes", async () => {
    await r2.put("json", "[1, \"é\"]", { httpMetadata: { contentType: "application/json" } });
    const blob = await (await r2.get("json")).blob();
    return {
      json: await (await r2.get("json")).json(),
      bytes: Object.prototype.toString.call(await (await r2.get("json")).bytes()),
      buffer: (await (await r2.get("json")).arrayBuffer()).byteLength,
      blob: [blob.size, blob.type],
      stream: await new Response((await r2.get("json")).body).text(),
    };
  });
  await record("bodyUsed", async () => {
    const object = await r2.get("a");
    const before = object.bodyUsed;
    const reader = object.body.getReader();
    await reader.read();
    return { before, after: object.bodyUsed, same: object.body === object.body, again: await object.text().catch(error => [error.name, error.message]) };
  });
  await record("values", async () => {
    const fixed = new FixedLengthStream(3);
    const writer = fixed.writable.getWriter();
    writer.write(new Uint8Array([1, 2, 3]));
    writer.close();
    const sizes = [];
    for (const value of [null, undefined, new Uint8Array([1, 2]).buffer, new Uint8Array([9, 8, 7]).subarray(1), new Blob(["xyz"]), new Response("abcd").body, new Request("http://x", { method: "POST", body: "abcdef" }).body, fixed.readable]) {
      sizes.push((await r2.put("value", value)).size);
    }
    sizes.push((await r2.put("copy", (await r2.get("a")).body)).size);
    const [first, second] = new Response("abcde").body.tee();
    const cloned = new Response("abc");
    cloned.clone();
    const textChunks = new FixedLengthStream(2);
    const textWriter = textChunks.writable.getWriter();
    textWriter.write("ab").catch(() => {});
    textWriter.close().catch(() => {});
    for (const value of [first, second, new Response("abc").clone().body, cloned.body, new Blob(["xy"]).stream(), textChunks.readable]) {
      sizes.push((await r2.put("value", value)).size);
    }
    return { sizes, copy: await (await r2.get("copy")).text() };
  });
  await record("streamedChunks", async () => {
    const length = 2.5 * MIB;
    const chunk = 700_001;
    const fixed = new FixedLengthStream(length);
    const writer = fixed.writable.getWriter();
    const writing = (async () => {
      for (let offset = 0; offset < length; offset += chunk) {
        const bytes = new Uint8Array(Math.min(chunk, length - offset));
        for (let index = 0; index < bytes.length; index += 4096) bytes[index] = (offset + index) % 251;
        await writer.write(bytes);
      }
      await writer.close();
    })();
    const object = await r2.put("streamed", fixed.readable);
    await writing;
    return [object.size, object.etag];
  });
  await record("valueNumber", () => r2.put("n", 5));
  await record("valueObject", () => r2.put("n", {}));
  await record("valueStream", () => r2.put("n", new ReadableStream({ start(controller) { controller.close(); } })));
  await record("keyNumber", async () => (await r2.put(5, "x")).key);
  await record("keyEmpty", async () => (await r2.put("", "x")).key);
  await record("keyLong", () => r2.put("k".repeat(1025), "x"));
  await record("keyLongHead", () => r2.head("k".repeat(1025)));
  await record("keyLongDelete", () => r2.delete(["ok", "k".repeat(1025)]));
  await record("metadataLarge", () => r2.put("m", "x", { customMetadata: { a: "\u0100".repeat(1024) } }));
  await record("metadataFits", async () => (await r2.put("m", "x", { customMetadata: { a: "é".repeat(2047) } })).size);
  await record("httpHeaders", async () => {
    const object = await r2.put("h", "x", { httpMetadata: new Headers({ "content-type": "t/x", expires: "Wed, 21 Oct 2015 07:28:00 GMT", "cache-control": "no-cache", "x-other": "1" }) });
    const headers = new Headers();
    object.writeHttpMetadata(headers);
    return { object: r2Object(object), headers: [...headers] };
  });
  await record("writeHttpMetadataBad", () => written.writeHttpMetadata({}));
  await record("checksums", async () => {
    const sha1 = await r2.put("c", "x", { sha1: "11F6AD8EC52A2984ABAAFD7C3B516503785C2072" });
    const md5 = await r2.put("c", "x", { md5: new Uint8Array([0x9d, 0xd4, 0xe4, 0x61, 0x26, 0x8c, 0x80, 0x34, 0xf5, 0xc8, 0x56, 0x4e, 0x15, 0x5c, 0x67, 0xa6]) });
    return { sha1: sha1.checksums.toJSON(), md5: JSON.stringify(md5.checksums), head: JSON.stringify((await r2.head("c")).checksums) };
  });
  await record("checksumMismatch", () => r2.put("c", "x", { sha256: "0".repeat(64) }));
  await record("checksumTwo", () => r2.put("c", "x", { md5: "0".repeat(32), sha1: "0".repeat(40) }));
  await record("checksumShort", () => r2.put("c", "x", { sha512: "00" }));
  await record("checksumBytes", () => r2.put("c", "x", { sha384: new Uint8Array(4) }));
  await record("checksumHex", () => r2.put("c", "x", { md5: "z".repeat(32) }));
  await r2.put("r", "0123456789");
  const ranges = {
    offset: { offset: 2 }, offsetLength: { offset: 2, length: 3 }, length: { length: 4 }, suffix: { suffix: 3 }, longSuffix: { suffix: 30 },
    zeroSuffix: { suffix: 0 }, atEnd: { offset: 10 }, pastEnd: { offset: 11 }, zeroLength: { offset: 1, length: 0 }, longLength: { offset: 8, length: 10 },
    negative: { offset: -1 }, fraction: { offset: 1.5 }, negativeLength: { length: -2 }, suffixOffset: { suffix: 1, offset: 1 }, suffixLength: { suffix: 1, length: 1 },
    negativeSuffix: { suffix: -1 }, empty: {}, text: { offset: "2" },
  };
  for (const [name, range] of Object.entries(ranges)) await record(`range_${name}`, async () => r2Body(await r2.get("r", { range })));
  for (const header of ["bytes=1-3", "bytes=-2", "bytes=5-", "bytes=1-2,4-5", "bytes=20-", "items=1-2", "bytes=3-1", "bytes=-0", "bytes=-20", "Bytes = 2-4"]) {
    await record(`rangeHeader_${header}`, async () => r2Body(await r2.get("r", { range: new Headers({ range: header }) })));
  }
  await record("rangeHeaderNone", async () => r2Body(await r2.get("r", { range: new Headers() })));
  const r = await r2.head("r");
  const before = new Date(r.uploaded.getTime() - 5000);
  const after = new Date(r.uploaded.getTime() + 5000);
  const conditions = {
    match: { etagMatches: r.etag }, noMatch: { etagMatches: "x" }, wildcard: { etagMatches: "*" }, notMatch: { etagDoesNotMatch: r.etag }, notMatchOther: { etagDoesNotMatch: "x" },
    after: { uploadedAfter: before }, afterLater: { uploadedAfter: after }, before: { uploadedBefore: after }, beforeEarlier: { uploadedBefore: before },
    same: { uploadedBefore: r.uploaded }, sameSecond: { uploadedBefore: new Date(r.uploaded.getTime() + 1), secondsGranularity: true },
    afterOverridden: { uploadedAfter: after, etagDoesNotMatch: "x" }, beforeOverridden: { uploadedBefore: before, etagMatches: r.etag },
    quoted: { etagMatches: `"${r.etag}"` }, dateText: { uploadedAfter: "2020-01-01" }, etagNumber: { etagMatches: 5 },
  };
  for (const [name, onlyIf] of Object.entries(conditions)) await record(`onlyIf_${name}`, async () => r2Body(await r2.get("r", { onlyIf })));
  const headerConditions = {
    ifMatch: { "if-match": `"${r.etag}"` }, ifMatchWeak: { "if-match": `W/"${r.etag}"` }, ifMatchList: { "if-match": `"x", "${r.etag}"` }, ifNoneMatch: { "if-none-match": `"${r.etag}"` },
    ifNoneMatchWeak: { "if-none-match": `W/"${r.etag}"` }, ifNoneMatchAny: { "if-none-match": "*" }, ifMatchUnseparated: { "if-match": `"${r.etag}" "x"` }, ifMatchUnquoted: { "if-match": r.etag },
    ifMatchUnclosed: { "if-match": "\"abc" }, ifMatchWeakBare: { "if-match": "W/abc" }, ifModifiedSince: { "if-modified-since": after.toUTCString() },
    ifUnmodifiedSince: { "if-unmodified-since": before.toUTCString() }, ifModifiedEarlier: { "if-modified-since": before.toUTCString() },
  };
  for (const [name, headers] of Object.entries(headerConditions)) await record(`onlyIfHeaders_${name}`, async () => r2Body(await r2.get("r", { onlyIf: new Headers(headers) })));
  await record("onlyIfWithRange", async () => r2Body(await r2.get("r", { onlyIf: { etagMatches: "x" }, range: { offset: 1 } })));
  await record("putConditions", async () => {
    const outcomes = {};
    outcomes.fails = await r2.put("r", "changed", { onlyIf: { etagMatches: "x" } });
    outcomes.holds = r2Object(await r2.put("r", "changed", { onlyIf: { etagMatches: r.etag } }));
    outcomes.create = r2Object(await r2.put("new", "x", { onlyIf: { etagDoesNotMatch: "*" } }));
    outcomes.exists = await r2.put("new", "x", { onlyIf: { etagDoesNotMatch: "*" } });
    outcomes.missingMatch = await r2.put("new2", "x", { onlyIf: { etagMatches: "x" } });
    outcomes.missingAfter = await r2.put("new3", "x", { onlyIf: { uploadedAfter: before } });
    outcomes.missingBefore = r2Object(await r2.put("new4", "x", { onlyIf: { uploadedBefore: before } }));
    outcomes.text = await (await r2.get("r")).text();
    return outcomes;
  });
  await record("delete", async () => {
    await r2.put("d1", "x");
    await r2.put("d2", "x");
    await r2.delete("d1");
    await r2.delete(["d2", "d3", 5]);
    await r2.delete([]);
    return [await r2.head("d1"), await r2.head("d2")];
  });
  for (const key of ["l/a", "l/b/1", "l/b/2", "l/b/c/3", "l/b/c/4", "l/b/d", "l/b0", "l/c", "l/c/", "l/c//x", "l/é/1", "l/\u{1F600}"]) {
    await r2.put(key, key, { httpMetadata: { contentType: "text/plain" }, customMetadata: { key } });
  }
  await record("list", async () => r2Keys(await r2.list({ prefix: "l/" })));
  await record("listShape", async () => Object.getOwnPropertyNames(await r2.list({ prefix: "l/", limit: 1 })));
  await record("listDelimiter", async () => r2Keys(await r2.list({ prefix: "l/", delimiter: "/" })));
  await record("listNested", async () => r2Keys(await r2.list({ prefix: "l/b/", delimiter: "/" })));
  await record("listLongDelimiter", async () => r2Keys(await r2.list({ prefix: "l/", delimiter: "/c" })));
  await record("listEmptyDelimiter", async () => r2Keys(await r2.list({ prefix: "l/", delimiter: "" })));
  await record("listPages", async () => {
    const pages = [];
    let cursor;
    do {
      const page = await r2.list({ prefix: "l/", delimiter: "/", limit: 2, cursor });
      pages.push(r2Keys(page));
      cursor = page.cursor;
    } while (cursor);
    return pages;
  });
  await record("listStartAfter", async () => r2Keys(await r2.list({ prefix: "l/", startAfter: "l/b/2" })));
  await record("listStartAfterDelimiter", async () => r2Keys(await r2.list({ prefix: "l/", startAfter: "l/b/2", delimiter: "/" })));
  await record("listStartAfterCursor", async () => {
    const page = await r2.list({ prefix: "l/", limit: 3 });
    return [r2Keys(await r2.list({ prefix: "l/", cursor: page.cursor, startAfter: "l/c" })), r2Keys(await r2.list({ prefix: "l/", cursor: page.cursor, startAfter: "l/a" }))];
  });
  await record("listInclude", async () => (await r2.list({ prefix: "l/b/", include: ["httpMetadata", "customMetadata"] })).objects.map(r2Object));
  await record("listIncludeCustom", async () => (await r2.list({ prefix: "l/b/c", include: ["customMetadata"] })).objects.map(object => object.customMetadata));
  await record("listIncludeBad", () => r2.list({ include: ["x"] }));
  await record("listIncludeText", () => r2.list({ include: "httpMetadata" }));
  await record("listLimits", async () => ({
    zero: await r2.list({ limit: 0 }).catch(error => error.message),
    large: await r2.list({ limit: 1001 }).catch(error => error.message),
    text: (await r2.list({ prefix: "l/", limit: "2" })).objects.length,
    fraction: (await r2.list({ prefix: "l/", limit: 1.5 })).objects.length,
    notNumber: await r2.list({ limit: NaN }).catch(error => error.message),
    minusOne: (await r2.list({ prefix: "l/", limit: -1 })).objects.length,
    huge: await r2.list({ limit: 2 ** 32 + 5 }).catch(error => [error.name, error.message]),
  }));
  await record("listPrefixNumber", () => r2.list({ prefix: 5 }));
  await record("listPrefixLong", async () => r2Keys(await r2.list({ prefix: "k".repeat(1100) })));
  await record("multipart", async () => {
    const upload = await r2.createMultipartUpload("big", { httpMetadata: { contentType: "x/y" }, customMetadata: { a: "b" } });
    const second = await upload.uploadPart(2, "tail");
    const first = await upload.uploadPart(1, sized(5 * MIB, 1));
    const object = await upload.complete([first, second]);
    const tail = await r2.get("big", { range: { offset: 5 * MIB - 2 } });
    return {
      upload: { key: upload.key, id: typeof upload.uploadId, json: Object.keys(JSON.parse(JSON.stringify(upload))) },
      part: { keys: Object.keys(first), number: first.partNumber, etag: typeof first.etag },
      object: r2Object(object),
      head: r2Object(await r2.head("big")),
      tail: [...new Uint8Array(await tail.arrayBuffer())],
      again: await upload.complete([first, second]).catch(error => error.message),
      abort: await upload.abort().then(() => "aborted", error => error.message),
      part3: await upload.uploadPart(3, "x").then(() => "uploaded", error => error.message),
    };
  });
  await record("multipartOrder", async () => {
    const upload = await r2.createMultipartUpload("ordered");
    const parts = [await upload.uploadPart(1, sized(5 * MIB, 1)), await upload.uploadPart(2, sized(5 * MIB, 2)), await upload.uploadPart(3, "end")];
    const misordered = await upload.complete([parts[2], parts[1], parts[0]]).catch(error => error.message);
    const object = await upload.complete([parts[1], parts[0], parts[2]]);
    const bytes = new Uint8Array(await (await r2.get("ordered")).arrayBuffer());
    return { misordered, size: object.size, etag: object.etag.slice(-2), bytes: [bytes[0], bytes[5 * MIB], bytes[bytes.length - 1]] };
  });
  await record("multipartSingle", async () => {
    const upload = await r2.createMultipartUpload("single");
    const object = await upload.complete([await upload.uploadPart(1, "abc")]);
    return { object: r2Object(object), text: await (await r2.get("single")).text() };
  });
  await record("multipartEmpty", async () => r2Object(await (await r2.createMultipartUpload("empty")).complete([])));
  await record("multipartSubset", async () => {
    const upload = await r2.createMultipartUpload("subset");
    const first = await upload.uploadPart(1, "a");
    await upload.uploadPart(2, "b");
    return (await upload.complete([first])).size;
  });
  await record("multipartReplacesPart", async () => {
    const upload = await r2.createMultipartUpload("replaced");
    const stale = await upload.uploadPart(1, "a");
    const fresh = await upload.uploadPart(1, "bb");
    return { stale: await upload.complete([stale]).catch(error => error.message), fresh: (await upload.complete([fresh])).size };
  });
  await record("multipartReplacesObject", async () => {
    await r2.put("replacedObject", "old");
    const upload = await r2.createMultipartUpload("replacedObject");
    await upload.complete([await upload.uploadPart(1, "new")]);
    return (await r2.get("replacedObject")).text();
  });
  await record("multipartSmall", async () => {
    const upload = await r2.createMultipartUpload("small");
    return upload.complete([await upload.uploadPart(1, "a"), await upload.uploadPart(2, "b")]);
  });
  await record("multipartUneven", async () => {
    const upload = await r2.createMultipartUpload("uneven");
    return upload.complete([await upload.uploadPart(1, sized(5 * MIB, 0)), await upload.uploadPart(2, sized(5 * MIB + 1, 0)), await upload.uploadPart(3, "x")]);
  });
  await record("multipartLastLarger", async () => {
    const upload = await r2.createMultipartUpload("larger");
    return upload.complete([await upload.uploadPart(1, sized(5 * MIB, 0)), await upload.uploadPart(2, sized(5 * MIB + 1, 0))]);
  });
  await record("multipartDuplicate", async () => {
    const upload = await r2.createMultipartUpload("duplicate");
    const part = await upload.uploadPart(1, "a");
    return upload.complete([part, part]);
  });
  await record("multipartUnknownEtag", async () => {
    const upload = await r2.createMultipartUpload("unknown");
    await upload.uploadPart(1, "a");
    return upload.complete([{ partNumber: 1, etag: "nope" }]);
  });
  await record("multipartStates", async () => {
    const unknown = r2.resumeMultipartUpload("k", "nope");
    const upload = await r2.createMultipartUpload("states");
    const other = r2.resumeMultipartUpload("other", upload.uploadId);
    const outcomes = {
      resumed: [unknown.key, unknown.uploadId],
      unknownPart: await unknown.uploadPart(1, "x").catch(error => error.message),
      unknownComplete: await unknown.complete([]).catch(error => error.message),
      unknownAbort: await unknown.abort().catch(error => error.message),
      otherPart: await other.uploadPart(1, "x").catch(error => error.message),
      otherAbort: await other.abort().catch(error => error.message),
    };
    await upload.abort();
    outcomes.abortAgain = await upload.abort().then(() => "aborted");
    outcomes.partAfterAbort = await upload.uploadPart(1, "x").catch(error => error.message);
    outcomes.completeAfterAbort = await upload.complete([]).catch(error => error.message);
    return outcomes;
  });
  await record("multipartArguments", async () => {
    const upload = await r2.createMultipartUpload("arguments");
    return {
      zero: await upload.uploadPart(0, "a").catch(error => [error.name, error.message]),
      text: (await upload.uploadPart("2", "a")).partNumber,
      stream: await upload.uploadPart(1, new ReadableStream({ start(controller) { controller.close(); } })).catch(error => [error.name, error.message]),
      nothing: await upload.uploadPart(1, null).catch(error => [error.name, error.message]),
      completeNothing: await upload.complete().catch(error => [error.name, error.message]),
      completeZero: await upload.complete([{ partNumber: 0, etag: "x" }]).catch(error => [error.name, error.message]),
      resume: (() => { try { return r2.resumeMultipartUpload("k").uploadId; } catch (error) { return [error.name, error.message]; } })(),
      longKey: await r2.createMultipartUpload("k".repeat(1025)).catch(error => error.message),
    };
  });
  return results;
}

export default {
  async fetch(request, env) {
    return Response.json({ d1: await d1Contracts(env.DB), kv: await kvContracts(env.KV), r2: await r2Contracts(env.R2) });
  },
};
