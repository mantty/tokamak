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

export default {
  async fetch(request, env) {
    return Response.json({ d1: await d1Contracts(env.DB) });
  },
};
