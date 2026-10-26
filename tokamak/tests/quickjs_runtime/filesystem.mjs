import fs from "node:fs";
import process from "node:process";
import { byteInputs } from "./bytes.mjs";

const SIZE_LIMIT = 128 * 1024 * 1024;

function failure(error) {
  return { error: error.name, code: error.code ?? null };
}

// What `run` returns, or the failure it throws.
function attempt(run) {
  try { return run(); } catch (error) { return { thrown: failure(error) }; }
}

// The trailing arguments each write of [1, 2, 3, 4] takes after its data.
const bufferArguments = {
  none: [],
  emptyOptions: [{}],
  undefinedOptions: [undefined],
  nullOptions: [null],
  undefinedLength: [0, undefined],
  nullLength: [0, null],
  undefinedEverything: [undefined, undefined, undefined],
  nullEverything: [null, null, null],
  lengthOption: [{ length: 2 }],
  offsetAndLengthOptions: [{ offset: 1, length: 2 }],
  positionOption: [{ position: 2 }],
  undefinedPosition: [0, 2, undefined],
  nullPosition: [0, 2, null],
  position: [1, 2, 1],
  offsetWithoutLength: [1],
  offsetOptionWithoutLength: [{ offset: 1 }],
  nullLengthOption: [{ length: null }],
  nullOffsetOption: [{ offset: null }],
  nullPositionOption: [{ position: null }],
  stringOffsetOption: [{ offset: "1" }],
  arrayOptions: [[1]],
  offsetBeyondLength: [3, 1],
  offsetAtLength: [2, 2],
  emptyAtEnd: [4, 0],
  lengthBeyondBuffer: [0, 5],
  offsetBeyondBuffer: [5, 0],
  negativeOffset: [-1],
  fractionalOffset: [0.5],
  nanOffset: [NaN],
  stringOffset: ["1"],
  booleanOffset: [true],
  negativeLength: [0, -1],
  fractionalLength: [0, 1.5],
  infiniteLength: [0, Infinity],
  stringLength: [0, "2"],
  negativePosition: [0, 2, -1],
  fractionalPosition: [0, 2, 1.5],
  largePosition: [0, 2, 2 ** 32],
  bigintPosition: [0, 2, 1n],
  negativeBigintPosition: [0, 2, -1n],
  hugeBigintPosition: [0, 2, 2n ** 64n],
  signedBigintPosition: [0, 2, 2n ** 63n],
  uint32BigintPosition: [0, 2, 2n ** 32n],
  uint32Position: [0, 2, 2 ** 32 - 1],
  beyondSizeLimit: [0, 2, SIZE_LIMIT - 1],
  stringPosition: [0, 2, "1"],
  negativeLengthAndBigint: [0, -1, -1n],
};

// The trailing arguments each write of "abcd" takes after its data.
const stringArguments = {
  none: [],
  position: [1],
  nullPosition: [null],
  undefinedPosition: [undefined],
  positionAndEncoding: [1, "utf8"],
  hex: [null, "hex"],
  base64: [null, "base64"],
  latin1: [null, "latin1"],
  unknownEncoding: [null, "nope"],
  options: [{ position: 1 }],
  stringPosition: ["1"],
  negativePosition: [-1],
};

// More data, each with the trailing argument lists to write it with.
const otherWrites = {
  middleView: [() => Uint8Array.from([1, 2, 3, 4, 5, 6, 7, 8]).subarray(2, 6), [[], [1], [{ offset: 1, length: 2 }]]],
  tailView: [() => Uint8Array.from([1, 2, 3, 4, 5, 6, 7, 8]).subarray(5, 7), [[], [0, 1]]],
  oddHex: [() => "abc", [[null, "hex"]]],
};

function bufferOutcome(buffer, data) {
  if (buffer === data) return "data";
  return { tag: Object.prototype.toString.call(buffer), constructor: buffer?.constructor?.name ?? null };
}

// The sync, callback and file handle forms of a write, by their function names
// and the name of the data a file handle write reports back.
function writeForms(syncName, callbackName, handleName, reportedName) {
  return {
    sync: (path, data, args) => withDescriptor(path, fd => {
      try { return { written: fs[syncName](fd, data, ...args) }; }
      catch (error) { return { thrown: failure(error) }; }
    }),
    callback: (path, data, args) => withDescriptor(path, fd => new Promise(resolve => {
      try {
        fs[callbackName](fd, data, ...args, (error, written, reported) =>
          resolve(error ? { error: failure(error) } : { written, reported: bufferOutcome(reported, data) }));
      } catch (error) { resolve({ thrown: failure(error) }); }
    })),
    handle: async (path, data, args) => {
      const handle = await fs.promises.open(path, "r+");
      try {
        let pending;
        try { pending = handle[handleName](data, ...args); }
        catch (error) { return { thrown: failure(error) }; }
        return await pending.then(
          result => ({ written: result.bytesWritten, reported: bufferOutcome(result[reportedName], data) }),
          error => ({ error: failure(error) }));
      } finally { await handle.close(); }
    },
  };
}

const writers = writeForms("writeSync", "write", "write", "buffer");
const vectorWriters = writeForms("writevSync", "writev", "writev", "buffers");

// The trailing arguments each vector write of [[1, 2]] takes after its buffers.
const vectorArguments = {
  none: [],
  undefinedPosition: [undefined],
  nullPosition: [null],
  position: [1],
  negativePosition: [-1],
  fractionalPosition: [1.5],
  stringPosition: ["1"],
  bigintPosition: [1n],
  negativeBigintPosition: [-1n],
  hugeBigintPosition: [2n ** 64n],
  signedBigintPosition: [2n ** 63n],
  uint32BigintPosition: [2n ** 32n],
  uint32Position: [2 ** 32 - 1],
};

// Other buffers, each with the trailing arguments to write them with.
const otherVectorWrites = {
  views: [() => [Uint8Array.from([1, 2, 3, 4]).subarray(1, 3), new DataView(Uint8Array.from([5, 6, 7]).buffer, 1, 1)], []],
  emptyWithBigint: [() => [], [-1n]],
  emptyWithNegative: [() => [], [-1]],
  string: [() => ["ab"], []],
  notArray: [() => Uint8Array.from([1, 2]), []],
  stringBeforeBadPosition: [() => [Uint8Array.from([1]), "x"], [-1]],
};

async function withDescriptor(path, use) {
  const fd = fs.openSync(path, "r+");
  try { return await use(fd); }
  finally { fs.closeSync(fd); }
}

// What `write` reports for `data` and `args` over a file of six 9s, and the file it leaves.
async function writeOutcome(write, data, args) {
  const path = "/tmp/filesystem-contract.bin";
  fs.writeFileSync(path, new Uint8Array(6).fill(9));
  const outcome = await write(path, data, args);
  return { ...outcome, contents: [...fs.readFileSync(path)] };
}

// The outcome of each write with `write`, one at a time.
async function writerOutcomes(write) {
  const outcomes = { buffer: {}, string: {}, data: {}, other: {} };
  for (const [name, args] of Object.entries(bufferArguments)) {
    outcomes.buffer[name] = await writeOutcome(write, Uint8Array.from([1, 2, 3, 4]), args);
  }
  for (const [name, args] of Object.entries(stringArguments)) {
    outcomes.string[name] = await writeOutcome(write, "abcd", args);
  }
  for (const [name, input] of Object.entries(byteInputs)) {
    outcomes.data[name] = await writeOutcome(write, input(), []);
  }
  for (const [name, [data, argumentLists]] of Object.entries(otherWrites)) {
    outcomes.other[name] = [];
    for (const args of argumentLists) outcomes.other[name].push(await writeOutcome(write, data(), args));
  }
  return outcomes;
}

// The outcome of each vector write with `write`, one at a time.
async function vectorOutcomes(write) {
  const outcomes = {};
  for (const [name, args] of Object.entries(vectorArguments)) {
    outcomes[name] = await writeOutcome(write, [Uint8Array.from([1, 2])], args);
  }
  for (const [name, [buffers, args]] of Object.entries(otherVectorWrites)) {
    outcomes[name] = await writeOutcome(write, buffers(), args);
  }
  return outcomes;
}

// A write at `position` through a descriptor opened for appending.
function appendOutcome(position) {
  const path = "/tmp/filesystem-append.bin";
  fs.writeFileSync(path, Uint8Array.from([9]));
  const fd = fs.openSync(path, "a");
  let outcome;
  try { outcome = { written: fs.writeSync(fd, Uint8Array.from([1]), 0, 1, position) }; }
  catch (error) { outcome = { thrown: failure(error) }; }
  fs.closeSync(fd);
  return { ...outcome, contents: [...fs.readFileSync(path)] };
}

function sizeLimitOutcomes() {
  const path = "/tmp/filesystem-limit.bin";
  fs.writeFileSync(path, "");
  const attempt = resize => {
    try { resize(); return { size: fs.statSync(path).size }; }
    catch (error) { return { thrown: failure(error) }; }
  };
  const fd = fs.openSync(path, "r+");
  try {
    return {
      truncate: attempt(() => fs.truncateSync(path, SIZE_LIMIT + 1)),
      ftruncate: attempt(() => fs.ftruncateSync(fd, SIZE_LIMIT + 1)),
      emptyWriteBeyond: attempt(() => fs.writeSync(fd, new Uint8Array(0), 0, 0, SIZE_LIMIT + 1)),
    };
  } finally { fs.closeSync(fd); }
}

// A fresh directory under /tmp holding `names` as files, created in order.
function directoryOf(name, names) {
  const root = `/tmp/filesystem-order/${name}`;
  fs.mkdirSync(root, { recursive: true });
  for (const entry of names) fs.writeFileSync(`${root}/${entry}`, entry);
  return root;
}

// What `change` leaves `readdirSync` listing in a directory of `names`.
function listingAfter(name, names, change) {
  const root = directoryOf(name, names);
  change(root);
  return fs.readdirSync(root);
}

async function orderContracts() {
  const created = directoryOf("created", ["zeta", "beta", "Capital", "_under", "10", "9"]);
  fs.mkdirSync(`${created}/alpha`);
  const dir = fs.opendirSync(created);
  const opened = [];
  for (let entry; (entry = dir.readSync()) !== null;) opened.push(entry.name);
  dir.closeSync();
  const letters = ["a", "b", "c", "d", "e"];
  return {
    created: fs.readdirSync(created),
    createdTypes: fs.readdirSync(created, { withFileTypes: true }).map(entry => entry.name),
    createdPromise: await fs.promises.readdir(created),
    opened,
    unlinkFirstOfThree: listingAfter("unlinkFirstOfThree", ["a", "b", "c"], root => fs.unlinkSync(`${root}/a`)),
    unlinkFirst: listingAfter("unlinkFirst", letters, root => fs.unlinkSync(`${root}/a`)),
    unlinkTwo: listingAfter("unlinkTwo", letters, root => { fs.unlinkSync(`${root}/a`); fs.unlinkSync(`${root}/b`); }),
    unlinkLast: listingAfter("unlinkLast", letters, root => fs.unlinkSync(`${root}/e`)),
    renameInPlace: listingAfter("renameInPlace", letters, root => fs.renameSync(`${root}/b`, `${root}/z`)),
    renameAway: listingAfter("renameAway", letters, root => fs.renameSync(`${root}/a`, "/tmp/filesystem-order/renamed")),
    renameInto: listingAfter("renameInto", letters, root => fs.renameSync(`${root}/../renamed`, `${root}/0`)),
    rmdir: listingAfter("rmdir", letters, root => { fs.mkdirSync(`${root}/0`); fs.writeFileSync(`${root}/1`, ""); fs.rmdirSync(`${root}/0`); }),
    rmRecursive: listingAfter("rmRecursive", letters, root => { fs.mkdirSync(`${root}/0/1`, { recursive: true }); fs.rmSync(`${root}/b`); fs.rmSync(`${root}/0`, { recursive: true }); }),
    recreated: listingAfter("recreated", letters, root => { fs.unlinkSync(`${root}/b`); fs.writeFileSync(`${root}/b`, "again"); }),
    overwritten: listingAfter("overwritten", letters, root => { fs.writeFileSync(`${root}/a`, "again"); fs.appendFileSync(`${root}/b`, "more"); fs.truncateSync(`${root}/c`); }),
    linked: listingAfter("linked", ["b", "a"], root => { fs.linkSync(`${root}/a`, `${root}/0`); fs.symlinkSync(`${root}/a`, `${root}/1`); fs.copyFileSync(`${root}/a`, `${root}/2`); }),
    copiedDirectory: (() => {
      const source = directoryOf("copySource", ["z", "a", "m"]);
      fs.cpSync(source, "/tmp/filesystem-order/copyTarget", { recursive: true });
      return fs.readdirSync("/tmp/filesystem-order/copyTarget");
    })(),
    root: fs.readdirSync("/"),
    devices: fs.readdirSync("/dev"),
  };
}

// A dirent's visible shape and type.
function direntOutcome(entry) {
  return {
    name: entry.name,
    parentPath: entry.parentPath,
    path: entry.path ?? null,
    keys: Reflect.ownKeys(entry).map(String),
    json: JSON.stringify(entry),
    types: ["isFile", "isDirectory", "isSymbolicLink", "isCharacterDevice", "isBlockDevice", "isFIFO", "isSocket"]
      .filter(method => entry[method]()),
    prototype: Object.getPrototypeOf(entry) === fs.Dirent.prototype,
  };
}

// The dirents `opendirSync` reads from `path`.
function openedDirents(path, options) {
  const dir = fs.opendirSync(path, options);
  const entries = [];
  for (let entry; (entry = dir.readSync()) !== null;) entries.push(direntOutcome(entry));
  dir.closeSync();
  return entries;
}

async function direntContracts() {
  const root = "/tmp/filesystem-dirents";
  fs.mkdirSync(`${root}/sub/deeper`, { recursive: true });
  fs.writeFileSync(`${root}/file`, "f");
  fs.writeFileSync(`${root}/sub/inner`, "i");
  fs.symlinkSync(`${root}/file`, `${root}/link`);
  const typed = (path, options = {}) => fs.readdirSync(path, { ...options, withFileTypes: true }).map(direntOutcome);
  const constructed = (() => {
    try { return direntOutcome(new fs.Dirent("made", 1, "/somewhere")); }
    catch (error) { return { thrown: failure(error) }; }
  })();
  return {
    names: fs.readdirSync(root),
    typed: typed(root),
    trailingSlash: typed(`${root}/`),
    doubledSlash: typed("/tmp//filesystem-dirents"),
    dotSegments: typed("/tmp/./filesystem-dirents/sub/.."),
    devices: typed("/dev"),
    root: typed("/").filter(entry => entry.name !== "bundle"),
    recursive: fs.readdirSync(root, { recursive: true }),
    recursiveTyped: typed(root, { recursive: true }),
    recursiveTrailingSlash: fs.readdirSync(`${root}/`, { recursive: true }),
    recursiveDotSegments: fs.readdirSync("/tmp/./filesystem-dirents/sub/..", { recursive: true }),
    recursiveRelative: fs.readdirSync("../tmp/filesystem-dirents", { recursive: true }),
    recursivePromise: await fs.promises.readdir(root, { recursive: true }),
    recursiveTypedPromise: (await fs.promises.readdir(root, { recursive: true, withFileTypes: true })).map(direntOutcome),
    recursiveCallback: await new Promise((resolve, reject) =>
      fs.readdir(root, { recursive: true }, (error, names) => error ? reject(error) : resolve(names))),
    recursiveFromRoot: fs.readdirSync("/", { recursive: true })
      .filter(name => name.includes("filesystem-dirents") || name.startsWith("dev")),
    opened: openedDirents(root),
    openedRecursive: openedDirents(root, { recursive: true }),
    openedTrailingSlash: openedDirents(`${root}/`, { recursive: true }),
    openedPath: fs.opendirSync(`${root}/`).path,
    constructed,
  };
}

// The kind of data a read returns, and its text.
function dataOutcome(value) {
  return {
    type: typeof value,
    buffer: Buffer.isBuffer(value),
    constructor: value?.constructor?.name ?? null,
    text: String(value),
  };
}

function attemptData(read) {
  try { return dataOutcome(read()); }
  catch (error) { return { thrown: failure(error) }; }
}

async function readDataContracts() {
  const root = "/tmp/filesystem-data";
  const file = `${root}/hello.txt`;
  fs.mkdirSync(`${root}/sub`, { recursive: true });
  fs.writeFileSync(file, "h\u00e9llo");
  fs.symlinkSync(file, `${root}/link`);
  const handle = await fs.promises.open(file, "r");
  const handleData = await handle.readFile();
  await handle.close();
  const dir = fs.opendirSync(root, { encoding: "buffer" });
  const opened = dir.readSync();
  dir.closeSync();
  return {
    file: attemptData(() => fs.readFileSync(file)),
    fileOptions: attemptData(() => fs.readFileSync(file, { encoding: null })),
    fileLatin1: attemptData(() => fs.readFileSync(file, "latin1")),
    fileBase64url: attemptData(() => fs.readFileSync(file, "base64url")),
    fileUcs2: attemptData(() => fs.readFileSync(file, "ucs2")),
    fileHex: attemptData(() => fs.readFileSync(file, "hex")),
    promise: dataOutcome(await fs.promises.readFile(file)),
    callback: dataOutcome(await new Promise((resolve, reject) =>
      fs.readFile(file, (error, data) => error ? reject(error) : resolve(data)))),
    handle: dataOutcome(handleData),
    names: fs.readdirSync(root, "buffer").map(dataOutcome),
    namesHex: fs.readdirSync(root, { encoding: "hex" }),
    namesLatin1: fs.readdirSync(root, { encoding: "latin1" }),
    recursiveNames: fs.readdirSync(root, { recursive: true, encoding: "buffer" }).map(dataOutcome),
    direntNames: fs.readdirSync(root, { withFileTypes: true, encoding: "buffer" }).map(entry => dataOutcome(entry.name)),
    openedName: dataOutcome(opened.name),
    readlinkHex: attemptData(() => fs.readlinkSync(`${root}/link`, "hex")),
  };
}

async function globContracts() {
  const root = "/tmp/filesystem-glob";
  fs.mkdirSync(`${root}/sub`, { recursive: true });
  fs.writeFileSync(`${root}/a.md`, "a");
  fs.writeFileSync(`${root}/sub/b.md`, "b");
  return {
    absoluteOutsideCwd: fs.globSync(`${root}/*.md`),
    absoluteFromRoot: fs.globSync(`${root}/*.md`, { cwd: "/" }),
    absoluteBelowCwd: fs.globSync(`${root}/*.md`, { cwd: "/tmp" }),
    absoluteWithinCwd: fs.globSync("/filesystem-glob/*.md", { cwd: "/tmp" }),
    absoluteRecursive: fs.globSync("/**/b.md", { cwd: root }),
    absoluteExclude: fs.globSync("/**/*.md", { cwd: root, exclude: ["/sub/**"] }),
    absoluteDirents: fs.globSync("/sub/*.md", { cwd: root, withFileTypes: true }).map(entry => [entry.name, entry.parentPath]),
    absolutePromise: await Array.fromAsync(fs.promises.glob("/sub/*.md", { cwd: root })),
    absoluteCallback: await new Promise((resolve, reject) =>
      fs.glob("/sub/*.md", { cwd: root }, (error, matches) => error ? reject(error) : resolve(matches))),
    overlappingPatterns: fs.globSync(["*.md", "a.*"], { cwd: root }),
  };
}

// What `globSync` returns for `pattern` and `options`, or throws.
function globOutcome(pattern, options) {
  try {
    const matches = fs.globSync(pattern, options);
    return options?.withFileTypes ? matches.map(direntOutcome) : matches;
  } catch (error) { return { thrown: failure(error) }; }
}

async function globDetailContracts() {
  const root = "/tmp/filesystem-glob-order";
  fs.mkdirSync(`${root}/zeta`, { recursive: true });
  fs.writeFileSync(`${root}/zeta/inner.txt`, "i");
  fs.writeFileSync(`${root}/beta.txt`, "b");
  fs.mkdirSync(`${root}/alpha/deep`, { recursive: true });
  fs.writeFileSync(`${root}/alpha/inner.txt`, "i");
  fs.writeFileSync(`${root}/alpha/deep/x.md`, "x");
  for (const name of ["Capital.txt", "_under.txt", "10.txt", "9.txt", "a*b.txt", "a[b].txt"]) fs.writeFileSync(`${root}/${name}`, name);
  fs.symlinkSync(`${root}/beta.txt`, `${root}/link`);
  const at = (pattern, options = {}) => globOutcome(pattern, { cwd: root, ...options });
  return {
    everything: at("**"),
    everythingTwice: at("**/**"),
    textFiles: at("**/*.txt"),
    top: at("*"),
    directoryContents: at("alpha/**"),
    braces: at("{zeta,alpha}/*.txt"),
    nestedBraces: at("{zeta,alpha/{deep,inner.txt}}"),
    singleBrace: at("{beta}.txt"),
    patterns: at(["zeta/*", "alpha/*", "*.txt"]),
    patternSet: at(new Set(["beta.txt", "9.txt"])),
    characterClass: at("[ab]*"),
    negatedClass: at("[!ab_]*"),
    rangeClass: at("[0-9]*"),
    unclosedClass: at("[ab"),
    question: at("?.txt"),
    escaped: at("a\\*b.txt"),
    literalBrackets: at("a\\[b\\].txt"),
    exactlyOne: at("@(alpha|zeta)"),
    noneOf: at("!(alpha|zeta)"),
    zeroOrMore: at("*(1|0).txt"),
    oneOrMore: at("+(1|0).txt"),
    zeroOrOne: at("?(1)0.txt"),
    dot: at("./zeta/../alpha/*"),
    dotSegment: at("zeta/./inner.txt"),
    parentEscape: at("../filesystem-glob-order/*"),
    parentAtEnd: at("alpha/deep/.."),
    currentAtEnd: at("alpha/."),
    trailingSlash: at("zeta/"),
    doubledSlashes: at("alpha//deep///x.md"),
    leadingDot: at("./beta.txt"),
    throughFile: at("beta.txt/*"),
    missingCwd: globOutcome("*", { cwd: "/tmp/filesystem-glob-missing" }),
    empty: at(""),
    slash: at("/"),
    encoding: at("beta.txt", { encoding: "buffer" }),
    rootDirents: globOutcome("tmp/filesystem-glob-order/*", { cwd: "/", withFileTypes: true }),
    slashDirents: globOutcome("filesystem-glob-order/zeta/*", { cwd: "/tmp/", withFileTypes: true }),
    dotDirents: globOutcome("zeta/*", { cwd: "/tmp/./filesystem-glob-order", withFileTypes: true }),
    relativeCwd: globOutcome("*.txt", { cwd: "../tmp/filesystem-glob-order" }),
    relativeDirents: globOutcome("zeta/*", { cwd: "../tmp/filesystem-glob-order", withFileTypes: true }),
    urlCwd: globOutcome("zeta/*", { cwd: new URL(`file://${root}`), withFileTypes: true }),
    currentDirents: at("alpha/.", { withFileTypes: true }),
    everythingDirents: at("**", { withFileTypes: true }),
    excludePatterns: at("**", { exclude: ["alpha/**", "*.{md,txt}"] }),
    excludeMiddle: at("**", { exclude: ["**/deep/**"] }),
    excludeParent: at("**", { exclude: ["zeta/../alpha"] }),
    excludeNames: at("**", { exclude: name => name.includes("a") }),
    excludeDirents: at("**", { withFileTypes: true, exclude: entry => entry.isDirectory() }),
    excludeString: at("*", { exclude: "beta.txt" }),
    numberPattern: globOutcome(5),
    numberInPatterns: globOutcome(["*", 5]),
    nullOptions: globOutcome("*", null),
    stringOptions: globOutcome("*", "x"),
    functionOptions: globOutcome("*", () => {}),
    numberCwd: globOutcome("*", { cwd: 5 }),
    callbackRequired: (() => {
      try { fs.glob("*", { cwd: root }); return "returned"; }
      catch (error) { return failure(error); }
    })(),
    callbackError: await new Promise(resolve => fs.glob(5, (error, matches) =>
      resolve(error ? { error: failure(error) } : { matches }))),
    callbackDefaultOptions: await new Promise(resolve => fs.glob("*.mjs", (error, matches) =>
      resolve(error ? { error: failure(error) } : { matches: Array.isArray(matches) }))),
    promiseError: await Array.fromAsync(fs.promises.glob(5)).then(matches => ({ matches }), error => ({ error: failure(error) })),
    promiseDirents: (await Array.fromAsync(fs.promises.glob("zeta/*", { cwd: root, withFileTypes: true }))).map(direntOutcome),
    promiseGenerator: await (async () => {
      const matches = fs.promises.glob("zeta/*", { cwd: root });
      const first = await matches.next();
      const returned = await matches.return("done");
      return [Object.prototype.toString.call(matches), matches[Symbol.asyncIterator]() === matches,
        fs.promises.glob.name, fs.promises.glob.length, first, returned, await matches.next()];
    })(),
  };
}

// Paths whose `..` segments climb above the root, which stops them.
function pathContracts() {
  const root = "/tmp/filesystem-paths";
  const encoded = `${root}/encoded`;
  fs.mkdirSync(encoded, { recursive: true });
  fs.writeFileSync(`${root}/file.txt`, "f");
  const names = ["a b", "\u00e9", "x#y", "x?y", "pct%41", "lt<gt>", "q\"b`c{d}", "tab\tx", "trail ", "caret^",
    "brack[]|';@", "%2f%3c%e9", "%2541", "50%", "c\u0001d", "\u0001lead"];
  for (const name of names) fs.writeFileSync(`${encoded}/${name}`, name);
  return {
    encodedNames: fs.readdirSync(encoded),
    readEncoded: attempt(() => fs.readFileSync(`${encoded}/a%20b`, "utf8")),
    readDecoded: attempt(() => fs.readFileSync(`${encoded}/pctA`, "utf8")),
    readLowercase: attempt(() => fs.readFileSync(`${encoded}/%c3%a9`, "utf8")),
    doubleEncodedDots: attempt(() => fs.readdirSync(`${encoded}/%252e%252e`)),
    backslash: attempt(() => fs.readdirSync("/tmp\\filesystem-paths")),
    host: attempt(() => fs.readdirSync("///tmp")),
    driveLetter: attempt(() => fs.readdirSync("/C|/x")),
    emptyPath: attempt(() => fs.statSync("").isDirectory()),
    nullByte: attempt(() => fs.readdirSync("/tmp/\0")),
    number: attempt(() => fs.readdirSync(5)),
    uint8Array: attempt(() => fs.readdirSync(new Uint8Array([47]))),
    buffer: attempt(() => fs.readdirSync(Buffer.from(root))),
    fileUrl: attempt(() => fs.readdirSync(new URL(`file://${root}`))),
    encodedFileUrl: attempt(() => fs.readFileSync(new URL(`file://${encoded}/a%20b`), "utf8")),
    httpUrl: attempt(() => fs.readdirSync(new URL("http://example.com/tmp"))),
    hostUrl: attempt(() => fs.readdirSync(new URL("file://host/tmp"))),
    urlString: attempt(() => fs.readdirSync("http://example.com/tmp")),
    writeOutsideRoots: attempt(() => fs.writeFileSync("/outside", "x")),
    mkdirOutsideRoots: attempt(() => fs.mkdirSync("/outside/inner", { recursive: true })),
    statOutsideRoots: attempt(() => fs.statSync("/outside")),
    symlinkOutsideRoots: attempt(() => fs.symlinkSync("/tmp", "/outside")),
    writeBundle: attempt(() => fs.writeFileSync("/bundle/outside", "x")),
    removeTmp: attempt(() => fs.rmdirSync("/tmp")),
    rootParent: attempt(() => fs.readdirSync("/..")),
    readdirAboveRoot: attempt(() => fs.readdirSync("/../tmp/filesystem-paths")),
    readAboveRoot: attempt(() => fs.readFileSync("/../../tmp/filesystem-paths/file.txt", "utf8")),
    relativeAboveRoot: attempt(() => fs.readFileSync("../../../tmp/filesystem-paths/file.txt", "utf8")),
    existsAboveRoot: fs.existsSync("/../tmp/filesystem-paths/file.txt"),
    globAboveRoot: globOutcome("*", { cwd: "/../tmp/filesystem-paths" }),
  };
}

// Symbolic links: their targets, and listings and copies that reach them.
async function symlinkContracts() {
  const root = "/tmp/filesystem-links";
  const alias = `${root}/alias`;
  fs.mkdirSync(`${root}/real/sub`, { recursive: true });
  fs.writeFileSync(`${root}/real/file.txt`, "f");
  fs.symlinkSync(`${root}/real`, alias);
  fs.symlinkSync(alias, `${root}/chain`);
  fs.symlinkSync("real", `${root}/relative`);
  fs.symlinkSync(`${root}/real/file.txt`, `${root}/file-link`);
  fs.symlinkSync(`${root}/missing`, `${root}/dangling`);
  const cycle = "/tmp/filesystem-links-cycle";
  fs.mkdirSync(`${cycle}/sub`, { recursive: true });
  fs.symlinkSync(cycle, `${cycle}/sub/up`);
  return {
    readlinkRelative: attempt(() => fs.readlinkSync(`${root}/relative`)),
    readThroughRelative: attempt(() => fs.readFileSync(`${root}/relative/file.txt`, "utf8")),
    readlinkSpaced: attempt(() => { fs.symlinkSync("a b/./c/../d", `${root}/spaced`); return fs.readlinkSync(`${root}/spaced`); }),
    readlinkUrl: attempt(() => { fs.symlinkSync(new URL(`file://${root}/real`), `${root}/url`); return fs.readlinkSync(`${root}/url`); }),
    numberTarget: attempt(() => fs.symlinkSync(5, `${root}/number`)),
    readThroughLink: attempt(() => fs.readFileSync(`${alias}/file.txt`, "utf8")),
    readdirLink: attempt(() => fs.readdirSync(alias)),
    readdirChain: attempt(() => fs.readdirSync(`${root}/chain`)),
    readdirFileLink: attempt(() => fs.readdirSync(`${root}/file-link`)),
    readdirDangling: attempt(() => fs.readdirSync(`${root}/dangling`)),
    readdirThroughLink: attempt(() => fs.readdirSync(`${alias}/sub`)),
    readdirThroughFile: attempt(() => fs.readdirSync(`${root}/real/file.txt/sub`)),
    readdirFile: attempt(() => fs.readdirSync(`${root}/real/file.txt`)),
    opendirLink: attempt(() => fs.opendirSync(alias).closeSync()),
    promiseLink: await fs.promises.readdir(alias).then(names => names, error => ({ thrown: failure(error) })),
    recursive: attempt(() => fs.readdirSync(root, { recursive: true })),
    recursiveDirents: attempt(() => fs.readdirSync(root, { recursive: true, withFileTypes: true }).map(direntOutcome)),
    recursiveOpendir: attempt(() => openedDirents(root, { recursive: true })),
    cycle: attempt(() => fs.readdirSync(cycle, { recursive: true })),
    cycleOpendir: attempt(() => fs.opendirSync(cycle, { recursive: true })),
    glob: globOutcome("alias/*", { cwd: root }),
    globCwd: globOutcome("*", { cwd: alias }),
    globstar: globOutcome("**", { cwd: root }),
    copyLink: attempt(() => {
      fs.cpSync(alias, `${root}/copy`, { recursive: true });
      return [fs.lstatSync(`${root}/copy`).isSymbolicLink(), fs.readlinkSync(`${root}/copy`)];
    }),
    copyDereferenced: attempt(() => {
      fs.cpSync(alias, `${root}/copy-dereferenced`, { recursive: true, dereference: true });
      return [fs.lstatSync(`${root}/copy-dereferenced`).isSymbolicLink(), fs.readdirSync(`${root}/copy-dereferenced`)];
    }),
    copyTreeWithDanglingLink: attempt(() => {
      fs.mkdirSync(`${root}/tree`);
      fs.symlinkSync(`${root}/missing`, `${root}/tree/dangling`);
      fs.cpSync(`${root}/tree`, `${root}/tree-copy`, { recursive: true });
      return fs.readlinkSync(`${root}/tree-copy/dangling`);
    }),
    copyFileLink: attempt(() => {
      fs.cpSync(`${root}/file-link`, `${root}/copy-file`);
      return fs.lstatSync(`${root}/copy-file`).isSymbolicLink();
    }),
  };
}

// process.chdir, and the paths and globs that resolve against the directory it sets.
function workingDirectoryContracts() {
  const root = "/tmp/filesystem-cwd";
  fs.mkdirSync(`${root}/inner`, { recursive: true });
  fs.writeFileSync(`${root}/file`, "f");
  fs.writeFileSync(`${root}/inner/nested.txt`, "n");
  fs.symlinkSync(`${root}/inner`, `${root}/link`);
  const change = directory => ({ result: attempt(() => process.chdir(directory)) ?? null, cwd: process.cwd() });
  try {
    return {
      initial: process.cwd(),
      absolute: change(root),
      relativeRead: attempt(() => fs.readFileSync("file", "utf8")),
      relativeList: attempt(() => fs.readdirSync(".")),
      relative: change("inner"),
      glob: attempt(() => fs.globSync("*")),
      globDirents: attempt(() => fs.globSync("*", { withFileTypes: true }).map(direntOutcome)),
      parent: change(".."),
      dots: change("./inner/../inner//"),
      link: change(`${root}/link`),
      missing: change(`${root}/missing`),
      file: change(`${root}/file`),
      throughFile: change(`${root}/file/x`),
      empty: change(""),
      escaped: change(`${root}/%69nner`),
      backslash: change("/tmp\\filesystem-cwd"),
      number: change(5),
      buffer: change(Buffer.from(root)),
      aboveRoot: change("/../tmp"),
      root: change("/"),
      fromRoot: attempt(() => fs.readdirSync("tmp").includes("filesystem-cwd")),
      functions: [process.chdir.name, process.chdir.length, process.cwd.name, process.cwd.length],
    };
  } finally { process.chdir("/bundle"); }
}

export async function filesystemContracts() {
  const result = { write: {} };
  for (const [name, write] of Object.entries(writers)) result.write[name] = await writerOutcomes(write);
  result.writev = {};
  for (const [name, write] of Object.entries(vectorWriters)) result.writev[name] = await vectorOutcomes(write);
  result.append = { uint32Bigint: appendOutcome(2n ** 32n), negativeBigint: appendOutcome(-1n), beyondSizeLimit: appendOutcome(SIZE_LIMIT) };
  result.sizeLimit = sizeLimitOutcomes();
  result.order = await orderContracts();
  result.dirents = await direntContracts();
  result.readData = await readDataContracts();
  result.glob = await globContracts();
  result.globDetails = await globDetailContracts();
  result.paths = pathContracts();
  result.symlinks = await symlinkContracts();
  result.workingDirectory = workingDirectoryContracts();
  return result;
}
