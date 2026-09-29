// Storage binding behaviour compared between tokamak and local Cloudflare.
// Values that differ by implementation (timings, sizes, bookmarks) are
// reduced to their types.

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
  await record("shape", () => Object.getOwnPropertyNames(Object.getPrototypeOf(kv)).sort());
  return results;
}

export default {
  async fetch(request, env) {
    return Response.json({ d1: await d1Contracts(env.DB), kv: await kvContracts(env.KV) });
  },
};
