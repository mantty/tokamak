import fs from "node:fs";
import process from "node:process";
import { byteInputs } from "./bytes.mjs";
import { failure as namedFailure } from "./outcome.mjs";

const SIZE_LIMIT = 128 * 1024 * 1024;

// The name and code of a thrown error and, for a system error, its system
// call details, message and enumerable property names.
function failure(error) {
  const details = namedFailure(error);
  if (!Object.hasOwn(error, "syscall")) return details;
  for (const key of ["errno", "syscall", "path", "dest", "message"]) {
    if (Object.hasOwn(error, key)) details[key] = error[key];
  }
  details.keys = Object.keys(error);
  return details;
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
  offsetAtEnd: [4],
  offsetPastEnd: [5],
  offsetPastEndBeforeLength: [5, "2"],
  offsetPastEndAfterPosition: [5, 1, "1"],
  lengthPastEndOption: [{ offset: 1, length: 4 }],
  nullOptionFields: [{ offset: null, length: null, position: null }],
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

// The data each whole-file write is given, made fresh for every use, and the
// encoding it writes the data in.
const fileWriteData = {
  utf8: [() => "h\u00e9llo", "utf8"],
  latin1: [() => "h\u00e9llo", "latin1"],
  binary: [() => "h\u00e9llo", "binary"],
  ascii: [() => "h\u00e9llo", "ascii"],
  utf16le: [() => "h\u00e9", "utf16le"],
  ucs2: [() => "h\u00e9", "ucs2"],
  base64: [() => "aGk/Pz8+", "base64"],
  urlSafeBase64: [() => "aGk_Pz8-", "base64"],
  base64url: [() => "aGk_Pz8-", "base64url"],
  hex: [() => "6869", "hex"],
  oddHex: [() => "686", "hex"],
  invalidHex: [() => "68zz69", "hex"],
  upperCase: [() => "h\u00e9", "LATIN1"],
  buffer: [() => "hi", "buffer"],
  unknown: [() => "hi", "nope"],
  numberEncoding: [() => "hi", 5],
  emptyEncoding: [() => "hi", ""],
  nullEncoding: [() => "hi", null],
  viewUnknown: [() => Uint8Array.from([1, 2]), "nope"],
  viewNumberEncoding: [() => Uint8Array.from([1, 2]), 5],
  nullData: [() => null, undefined],
  ...Object.fromEntries(Object.entries(byteInputs).map(([name, input]) => [name, [input, undefined]])),
};

// A write stream's write of `data` in `encoding` to `path`, by `write` or by
// `end`, once the stream finishes.
async function streamWrite(path, data, encoding, method = "write") {
  const stream = fs.createWriteStream(path);
  try {
    await new Promise((resolve, reject) => {
      stream.on("error", reject);
      if (method === "end") stream.end(data, encoding, resolve);
      else { stream.write(data, encoding); stream.end(resolve); }
    });
  } finally { stream.destroy(); }
}

// The file handle method `name` applied to `path`.
async function withHandle(path, name, ...args) {
  const handle = await fs.promises.open(path, "r+");
  try { await handle[name](...args); }
  finally { await handle.close(); }
}

// Each whole-file write of `data` in `encoding` to `path`.
const fileWrites = {
  writeFileSync: (path, data, encoding) => fs.writeFileSync(path, data, encoding),
  writeFileSyncOptions: (path, data, encoding) => fs.writeFileSync(path, data, { encoding }),
  appendFileSync: (path, data, encoding) => fs.appendFileSync(path, data, encoding),
  writeFile: (path, data, encoding) => new Promise((resolve, reject) => fs.writeFile(path, data, encoding, error => error ? reject(error) : resolve())),
  appendFile: (path, data, encoding) => new Promise((resolve, reject) => fs.appendFile(path, data, encoding, error => error ? reject(error) : resolve())),
  promisesWriteFile: (path, data, encoding) => fs.promises.writeFile(path, data, encoding),
  handleWriteFile: (path, data, encoding) => withHandle(path, "writeFile", data, encoding),
  handleAppendFile: (path, data, encoding) => withHandle(path, "appendFile", data, encoding),
  streamWrite,
  streamEnd: (path, data, encoding) => streamWrite(path, data, encoding, "end"),
};

// The file each whole-file write leaves from each data and encoding, over a
// file holding [9], or the error it fails with.
async function fileWriteContracts() {
  const path = "/tmp/filesystem-file-write.bin";
  const outcomes = {};
  for (const [writeName, write] of Object.entries(fileWrites)) {
    outcomes[writeName] = {};
    for (const [dataName, [data, encoding]] of Object.entries(fileWriteData)) {
      fs.writeFileSync(path, Uint8Array.from([9]));
      try {
        await write(path, data(), encoding);
        outcomes[writeName][dataName] = { contents: [...fs.readFileSync(path)] };
      } catch (error) { outcomes[writeName][dataName] = { error: failure(error) }; }
    }
  }
  return outcomes;
}

// What `FileHandle.writeFile` resolves with for a string, and whether it
// throws or rejects for a number.
async function handleWriteFileResult() {
  const handle = await fs.promises.open("/tmp/filesystem-handle-write.bin", "w+");
  try {
    const { bytesWritten, buffer } = await handle.writeFile("6869", "hex");
    let rejection = "threw";
    try { const pending = handle.writeFile(5); rejection = "rejected"; await pending; }
    catch (error) { rejection = `${rejection} ${error.code}`; }
    return { bytesWritten, isBuffer: Buffer.isBuffer(buffer), bytes: [...buffer], rejection };
  } finally { await handle.close(); }
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
    topDirents: globOutcome("tmp/.", { cwd: "/", withFileTypes: true }),
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
    excludeStringObjects: at("**", { exclude: [new String("alpha/**"), new String("*.{md,txt}")] }),
    excludeNumbers: globOutcome("*", { cwd: root, exclude: [5] }),
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

// What a one-byte write leaves in a four-byte file opened with each flag,
// whole-file writes of one byte, and a write stream's one-byte write.
async function openFlagContracts() {
  const path = "/tmp/filesystem-open-flags.bin";
  const reset = () => fs.writeFileSync(path, Uint8Array.from([9, 9, 9, 9]));
  const { O_WRONLY, O_RDWR, O_CREAT, O_TRUNC } = fs.constants;
  const flags = { w: "w", "w+": "w+", a: "a", "a+": "a+", "r+": "r+", truncating: O_WRONLY | O_CREAT | O_TRUNC, readTruncating: O_RDWR | O_TRUNC };
  const outcomes = {};
  for (const [name, flag] of Object.entries(flags)) {
    reset();
    const fd = fs.openSync(path, flag);
    fs.writeSync(fd, Uint8Array.from([1]));
    fs.closeSync(fd);
    outcomes[name] = [...fs.readFileSync(path)];
  }
  reset();
  fs.writeFileSync(path, Uint8Array.from([1]), { flag: "r+" });
  outcomes.writeFileReadWrite = [...fs.readFileSync(path)];
  reset();
  const fd = fs.openSync(path, "r+");
  fs.writeFileSync(fd, Uint8Array.from([1]));
  fs.closeSync(fd);
  outcomes.writeFileDescriptor = [...fs.readFileSync(path)];
  reset();
  const stream = fs.createWriteStream(path);
  await new Promise(resolve => { stream.write(Uint8Array.from([1])); stream.end(resolve); });
  outcomes.stream = [...fs.readFileSync(path)];
  return outcomes;
}

// Each entry under `root` in listing order, with a file's text, a
// directory's entries or a symbolic link's target.
function treeOf(root) {
  return fs.readdirSync(root).map(name => {
    const path = `${root}/${name}`;
    const stats = fs.lstatSync(path);
    if (stats.isSymbolicLink()) return [name, `-> ${fs.readlinkSync(path)}`];
    return [name, stats.isDirectory() ? treeOf(path) : fs.readFileSync(path, "utf8")];
  });
}

// What `change` returns, throws or rejects with in a fresh directory, and the
// tree it leaves there. The directory holds files a and b, a hard link to a,
// directories empty and full, and symbolic links to b, to full, to nothing
// and to each other.
async function overwriteOutcome(name, change) {
  const root = `/tmp/filesystem-overwrite/${name}`;
  fs.mkdirSync(`${root}/full`, { recursive: true });
  fs.mkdirSync(`${root}/empty`);
  fs.writeFileSync(`${root}/full/inner`, "inner");
  for (const file of ["a", "b"]) fs.writeFileSync(`${root}/${file}`, file);
  fs.linkSync(`${root}/a`, `${root}/hard`);
  fs.symlinkSync(`${root}/b`, `${root}/link`);
  fs.symlinkSync(`${root}/full`, `${root}/directoryLink`);
  fs.symlinkSync(`${root}/nothing`, `${root}/broken`);
  fs.symlinkSync(`${root}/loopB`, `${root}/loopA`);
  fs.symlinkSync(`${root}/loopA`, `${root}/loopB`);
  let result;
  try { result = (await change(root)) ?? "done"; }
  catch (error) { result = { thrown: failure(error) }; }
  return { result, tree: treeOf(root) };
}

// A file in /bundle: a Text module, which tokamak and workerd both keep there.
const bundleFile = "/bundle/bundle.txt";

// Writes `text` through a descriptor of `path` opened before `change`.
function writeAfter(path, change, text) {
  const fd = fs.openSync(path, "r+");
  try { change(); fs.writeSync(fd, text); }
  finally { fs.closeSync(fd); }
}

// Each rename, by name, from a directory `overwriteOutcome` makes.
const renames = {
  ontoFile: root => fs.renameSync(`${root}/a`, `${root}/b`),
  ontoItself: root => fs.renameSync(`${root}/a`, `${root}/a`),
  ontoHardLink: root => fs.renameSync(`${root}/a`, `${root}/hard`),
  ontoItselfThroughLink: root => fs.renameSync(`${root}/full/inner`, `${root}/directoryLink/inner`),
  ontoEmptyDirectory: root => fs.renameSync(`${root}/a`, `${root}/empty`),
  ontoParent: root => fs.renameSync(`${root}/full/inner`, `${root}/full`),
  ontoSymlink: root => fs.renameSync(`${root}/a`, `${root}/link`),
  ontoBrokenSymlink: root => fs.renameSync(`${root}/a`, `${root}/broken`),
  ontoSymlinkLoop: root => fs.renameSync(`${root}/a`, `${root}/loopA`),
  directoryOntoEmptyDirectory: root => fs.renameSync(`${root}/full`, `${root}/empty`),
  directoryOntoFullDirectory: root => fs.renameSync(`${root}/empty`, `${root}/full`),
  directoryOntoFile: root => fs.renameSync(`${root}/empty`, `${root}/a`),
  directoryOntoDirectoryLink: root => fs.renameSync(`${root}/empty`, `${root}/directoryLink`),
  directoryOntoItself: root => fs.renameSync(`${root}/full`, `${root}/full`),
  directoryIntoItself: root => fs.renameSync(`${root}/full`, `${root}/full/inside`),
  directoryOntoChild: root => fs.renameSync(`${root}/full`, `${root}/full/inner`),
  symlink: root => fs.renameSync(`${root}/link`, `${root}/c`),
  symlinkOntoTarget: root => fs.renameSync(`${root}/link`, `${root}/b`),
  targetOntoSymlink: root => fs.renameSync(`${root}/b`, `${root}/link`),
  directoryLink: root => {
    fs.renameSync(`${root}/directoryLink`, `${root}/moved`);
    fs.writeFileSync(`${root}/full/added`, "added");
  },
  brokenSymlink: root => fs.renameSync(`${root}/broken`, `${root}/c`),
  symlinkLoop: root => fs.renameSync(`${root}/loopA`, `${root}/c`),
  intoSymlinkLoop: root => fs.renameSync(`${root}/a`, `${root}/loopA/c`),
  intoDirectoryLink: root => fs.renameSync(`${root}/a`, `${root}/directoryLink/a`),
  missing: root => fs.renameSync(`${root}/missing`, `${root}/c`),
  missingOntoFile: root => fs.renameSync(`${root}/missing`, `${root}/a`),
  missingParent: root => fs.renameSync(`${root}/a`, `${root}/missing/c`),
  parentIsFile: root => fs.renameSync(`${root}/a`, `${root}/b/c`),
  sourceParentIsFile: root => fs.renameSync(`${root}/a/c`, `${root}/c`),
  throughFile: root => fs.renameSync(`${root}/full`, `${root}/full/inner/c`),
  replacedDescriptor: root => writeAfter(`${root}/b`, () => fs.renameSync(`${root}/a`, `${root}/b`), "Z"),
  renamedDescriptor: root => writeAfter(`${root}/a`, () => fs.renameSync(`${root}/a`, `${root}/c`), "Z"),
  url: root => fs.renameSync(new URL(`file://${root}/a`), new URL(`file://${root}/a?query#hash`)),
  fromBundle: root => fs.renameSync(bundleFile, `${root}/c`),
  missingFromBundle: root => fs.renameSync("/bundle/missing", `${root}/c`),
  intoBundle: root => fs.renameSync(`${root}/a`, "/bundle/a"),
  ontoBundleFile: root => fs.renameSync(`${root}/a`, bundleFile),
  bundleFileOntoItself: () => fs.renameSync(bundleFile, bundleFile),
  ontoBundle: root => fs.renameSync(`${root}/empty`, "/bundle"),
  bundle: root => fs.renameSync("/bundle", `${root}/c`),
  tmpOntoItself: () => fs.renameSync("/tmp", "/tmp"),
  tmp: root => fs.renameSync("/tmp", `${root}/c`),
  ontoTmp: root => fs.renameSync(`${root}/a`, "/tmp"),
  root: root => fs.renameSync("/", `${root}/c`),
  ontoRoot: root => fs.renameSync(`${root}/a`, "/"),
  ontoDevice: root => fs.renameSync(`${root}/a`, "/dev/null"),
  device: root => fs.renameSync("/dev/null", `${root}/c`),
  relative: root => fs.renameSync(`${root}/a`, "c"),
  promise: root => fs.promises.rename(`${root}/a`, `${root}/b`),
  promiseOntoDirectory: root => fs.promises.rename(`${root}/a`, `${root}/empty`),
  callback: root => new Promise((resolve, reject) => fs.rename(`${root}/a`, `${root}/b`, error => error ? reject(error) : resolve())),
  callbackOntoDirectory: root => new Promise((resolve, reject) => fs.rename(`${root}/a`, `${root}/empty`, error => error ? reject(error) : resolve())),
};

// Each copyFile, by name, from a directory `overwriteOutcome` makes.
const copies = {
  ontoFile: root => fs.copyFileSync(`${root}/a`, `${root}/b`),
  ontoItself: root => fs.copyFileSync(`${root}/a`, `${root}/a`),
  ontoHardLink: root => { fs.copyFileSync(`${root}/b`, `${root}/a`); fs.appendFileSync(`${root}/hard`, "+"); },
  ontoFileThroughLink: root => fs.copyFileSync(`${root}/a`, `${root}/directoryLink/inner`),
  ontoEmptyDirectory: root => fs.copyFileSync(`${root}/a`, `${root}/empty`),
  ontoSymlink: root => fs.copyFileSync(`${root}/a`, `${root}/link`),
  ontoBrokenSymlink: root => fs.copyFileSync(`${root}/a`, `${root}/broken`),
  ontoDirectoryLink: root => fs.copyFileSync(`${root}/a`, `${root}/directoryLink`),
  directory: root => fs.copyFileSync(`${root}/empty`, `${root}/c`),
  directoryOntoFile: root => fs.copyFileSync(`${root}/empty`, `${root}/a`),
  symlink: root => fs.copyFileSync(`${root}/link`, `${root}/c`),
  symlinkOntoTarget: root => fs.copyFileSync(`${root}/link`, `${root}/b`),
  brokenSymlink: root => fs.copyFileSync(`${root}/broken`, `${root}/c`),
  symlinkLoop: root => fs.copyFileSync(`${root}/loopA`, `${root}/c`),
  intoDirectoryLink: root => fs.copyFileSync(`${root}/a`, `${root}/directoryLink/a`),
  missing: root => fs.copyFileSync(`${root}/missing`, `${root}/c`),
  missingParent: root => fs.copyFileSync(`${root}/a`, `${root}/missing/c`),
  parentIsFile: root => fs.copyFileSync(`${root}/a`, `${root}/b/c`),
  sourceParentIsFile: root => fs.copyFileSync(`${root}/a/c`, `${root}/c`),
  fromBundle: root => {
    fs.copyFileSync(bundleFile, `${root}/c`);
    const copied = fs.readFileSync(`${root}/c`, "utf8") === fs.readFileSync(bundleFile, "utf8");
    fs.unlinkSync(`${root}/c`);
    return copied;
  },
  intoBundle: root => fs.copyFileSync(`${root}/a`, "/bundle/a"),
  bundleFileOntoItself: () => fs.copyFileSync(bundleFile, bundleFile),
  ontoBundleFile: root => fs.copyFileSync(`${root}/a`, bundleFile),
  devices: root => ["null", "zero", "full", "random"].forEach(name => fs.copyFileSync(`/dev/${name}`, `${root}/${name}`)),
  ontoDevice: root => fs.copyFileSync(`${root}/a`, "/dev/null"),
  exclusive: root => fs.copyFileSync(`${root}/a`, `${root}/b`, fs.constants.COPYFILE_EXCL),
  exclusiveSymlink: root => fs.copyFileSync(`${root}/a`, `${root}/link`, fs.constants.COPYFILE_EXCL),
  exclusiveBrokenSymlink: root => fs.copyFileSync(`${root}/a`, `${root}/broken`, fs.constants.COPYFILE_EXCL),
  exclusiveNew: root => fs.copyFileSync(`${root}/a`, `${root}/c`, fs.constants.COPYFILE_EXCL),
  clone: root => fs.copyFileSync(`${root}/a`, `${root}/b`, fs.constants.COPYFILE_FICLONE),
  cloneForce: root => fs.copyFileSync(`${root}/a`, `${root}/b`, fs.constants.COPYFILE_FICLONE_FORCE),
  promise: root => fs.promises.copyFile(`${root}/a`, `${root}/b`),
  promiseMode: root => fs.promises.copyFile(`${root}/a`, `${root}/b`, 3),
  callback: root => new Promise((resolve, reject) => fs.copyFile(`${root}/a`, `${root}/b`, error => error ? reject(error) : resolve())),
  callbackExclusive: root => fs.copyFile(`${root}/a`, `${root}/b`, fs.constants.COPYFILE_EXCL, () => {}),
  callbackMode: root => fs.copyFile(`${root}/a`, `${root}/b`, 3, () => {}),
};

// rename and copyFile onto existing names, and copyFile's modes.
async function overwriteContracts() {
  const outcomes = { rename: {}, copyFile: {} };
  for (const [name, change] of Object.entries(renames)) outcomes.rename[name] = await overwriteOutcome(`rename-${name}`, change);
  for (const [name, change] of Object.entries(copies)) outcomes.copyFile[name] = await overwriteOutcome(`copy-${name}`, change);
  const root = "/tmp/filesystem-overwrite/modes";
  fs.mkdirSync(root, { recursive: true });
  fs.writeFileSync(`${root}/a`, "a");
  outcomes.copyModes = [3, 5, -1, "1", 1.5, null, undefined, -0].map((mode, index) =>
    attempt(() => fs.copyFileSync(`${root}/a`, `${root}/mode${index}`, mode)) ?? "copied");
  outcomes.copyModesTree = treeOf(root);
  return outcomes;
}

// The trailing arguments each read into a four-byte buffer takes after it.
const readArguments = {
  none: [],
  emptyOptions: [{}],
  undefinedOptions: [undefined],
  nullOptions: [null],
  offset: [1],
  offsetAndLength: [1, 2],
  undefinedLength: [0, undefined],
  nullLength: [0, null],
  position: [0, 2, 3],
  nullPosition: [0, 2, null],
  undefinedPosition: [0, 2, undefined],
  negativePosition: [0, 2, -1],
  bigintPosition: [0, 2, 3n],
  stringPosition: [0, 2, "1"],
  lengthOption: [{ length: 2 }],
  offsetOption: [{ offset: 1 }],
  offsetAndLengthOptions: [{ offset: 1, length: 2 }],
  positionOption: [{ position: 3 }],
  stringOffset: ["1"],
  negativeOffset: [-1],
  fractionalOffset: [0.5],
  negativeLength: [0, -1],
  lengthBeyondBuffer: [0, 5],
  offsetBeyondBuffer: [5, 0],
  lengthBeyondBufferAtPosition: [0, 5, 0],
  lengthBeyondBufferOption: [{ length: 5 }],
  offsetBeyondBufferOption: [{ offset: 5, length: 0 }],
  offsetOptionBeyondBuffer: [{ offset: 5 }],
  bigintLength: [0, 1n],
  nullOffsetOption: [{ offset: null }],
  nullLengthOption: [{ length: null }],
  emptyLength: [0, 0],
  arrayOptions: [[1]],
};

// The sync, callback and file handle forms of a read of `path` into `buffer`.
const readers = {
  sync: (path, buffer, args) => withDescriptor(path, fd => attempt(() => ({ read: fs.readSync(fd, buffer, ...args) }))),
  callback: (path, buffer, args) => withDescriptor(path, fd => new Promise(resolve => {
    try { fs.read(fd, buffer, ...args, (error, read) => resolve(error ? { error: failure(error) } : { read })); }
    catch (error) { resolve({ thrown: failure(error) }); }
  })),
  handle: async (path, buffer, args) => {
    const handle = await fs.promises.open(path, "r");
    try {
      let pending;
      try { pending = handle.read(buffer, ...args); }
      catch (error) { return { thrown: failure(error) }; }
      return await pending.then(result => ({ read: result.bytesRead }), error => ({ error: failure(error) }));
    } finally { await handle.close(); }
  },
};

// Reads that workerd's fs.read and fileHandle.read default differently from
// readSync, by form.
const unmatchedReads = {
  callback: ["emptyOptions", "undefinedOptions", "nullOptions", "offset", "offsetAndLength", "undefinedLength", "nullLength", "nullPosition", "undefinedPosition", "bigintPosition", "stringPosition", "offsetOption", "negativeOffset", "fractionalOffset", "negativeLength", "lengthBeyondBuffer", "offsetBeyondBuffer", "bigintLength", "nullOffsetOption", "nullLengthOption", "emptyLength", "middleView", "data", "string", "stringOffset"],
  handle: ["nullOffsetOption", "nullLengthOption", "stringOffset", "arrayOptions", "string"],
};

// What each reader reports for each argument list over a file of [1, ..., 8],
// with the bytes of the memory under the buffer it read into.
async function readContracts() {
  const path = "/tmp/filesystem-read.bin";
  fs.writeFileSync(path, Uint8Array.from([1, 2, 3, 4, 5, 6, 7, 8]));
  const reads = Object.fromEntries(Object.entries(readArguments).map(([name, args]) => [name, [() => new Uint8Array(4), [args]]]));
  Object.assign(reads, {
    middleView: [() => new Uint8Array(8).subarray(2, 6), [[], [1], [0, 6], [-1, 2], [{ offset: 1 }]]],
    data: [() => new DataView(new ArrayBuffer(4)), [[], [1, 2, 0]]],
    string: [() => "abcd", [[]]],
  });
  const outcomes = {};
  for (const [form, read] of Object.entries(readers)) {
    outcomes[form] = {};
    for (const [name, [buffer, argumentLists]] of Object.entries(reads)) {
      if (unmatchedReads[form]?.includes(name)) continue;
      outcomes[form][name] = [];
      for (const args of argumentLists) {
        const view = buffer();
        const outcome = await read(path, view, args);
        outcomes[form][name].push({ ...outcome, bytes: view.buffer ? [...new Uint8Array(view.buffer)] : null });
      }
    }
  }
  const fd = fs.openSync(path, "r");
  try {
    const vectors = { none: [], nullPosition: [null], position: [3], negativePosition: [-1], stringPosition: ["1"], bigintPosition: [3n] };
    outcomes.vector = {};
    for (const [name, args] of Object.entries(vectors)) {
      const buffers = [new Uint8Array(2), new Uint8Array(8).subarray(2, 5)];
      const outcome = attempt(() => ({ read: fs.readvSync(fd, buffers, ...args) }));
      outcomes.vector[name] = { ...outcome, bytes: buffers.map(buffer => [...new Uint8Array(buffer.buffer)]) };
    }
    outcomes.vector.notArray = attempt(() => fs.readvSync(fd, new Uint8Array(2)));
    outcomes.vector.string = attempt(() => fs.readvSync(fd, ["ab"]));
    outcomes.badDescriptor = ["1", -1, 1.5, 2 ** 31].map(fd => attempt(() => fs.readSync(fd, new Uint8Array(4))));
  } finally { fs.closeSync(fd); }
  return outcomes;
}

// The options each read of a name or file contents is given.
const encodingOptions = {
  unknown: "bogus",
  unknownProperty: { encoding: "bogus" },
  buffer: "buffer",
  hex: "hex",
  upperCase: "UTF8",
  empty: "",
  nullOptions: null,
  nullEncoding: { encoding: null },
  number: 5,
  numberEncoding: { encoding: 5 },
};

function encodedOutcome(value, prefix) {
  if (value instanceof fs.Dir) { value.closeSync(); return "Dir"; }
  if (prefix) return { type: typeof value, prefixed: String(value).startsWith(prefix) };
  if (Array.isArray(value)) return value.map(dataOutcome);
  return dataOutcome(value);
}

// The sync, callback and promise forms of each read that takes an encoding,
// with each of the encoding options.
async function encodingContracts() {
  const root = "/tmp/filesystem-encoding";
  fs.mkdirSync(root, { recursive: true });
  fs.writeFileSync(`${root}/file`, "abc");
  fs.symlinkSync(`${root}/file`, `${root}/link`);
  const prefix = `${root}/t-`;
  const reads = { readFile: `${root}/file`, readdir: root, readlink: `${root}/link`, realpath: `${root}/link`, mkdtemp: prefix, opendir: root };
  const outcomes = {};
  for (const [name, path] of Object.entries(reads)) {
    const outcome = value => encodedOutcome(value, name === "mkdtemp" && prefix);
    outcomes[name] = {};
    for (const [optionName, options] of Object.entries(encodingOptions)) {
      outcomes[name][optionName] = {
        sync: attempt(() => outcome(fs[`${name}Sync`](path, options))),
        callback: await new Promise(resolve => {
          try { fs[name](path, options, (error, value) => resolve(error ? { error: failure(error) } : outcome(value))); }
          catch (error) { resolve({ thrown: failure(error) }); }
        }),
        promise: await Promise.resolve().then(() => fs.promises[name](path, options)).then(outcome, error => ({ error: failure(error) })),
      };
    }
  }
  return outcomes;
}

// The own properties of `value` and of each prototype above it, each with
// its type or "getter".
function shape(value) {
  const levels = [];
  for (let object = value; object && object !== Object.prototype; object = Object.getPrototypeOf(object)) {
    levels.push(Reflect.ownKeys(object).map(key => {
      const descriptor = Object.getOwnPropertyDescriptor(object, key);
      return `${String(key)}:${descriptor.get ? "getter" : typeof descriptor.value}`;
    }));
  }
  return levels;
}

const statFields = ["dev", "ino", "mode", "nlink", "uid", "gid", "rdev", "size", "blksize", "blocks", "atimeMs"];

// What Stats and Dir objects look like.
async function shapeContracts() {
  const root = "/tmp/filesystem-shapes";
  fs.mkdirSync(root, { recursive: true });
  fs.writeFileSync(`${root}/file`, "abc");
  fs.symlinkSync(`${root}/file`, `${root}/link`);
  const stats = {
    file: fs.statSync(`${root}/file`),
    bigint: fs.statSync(`${root}/file`, { bigint: true }),
    directory: fs.statSync(root),
    link: fs.lstatSync(`${root}/link`),
    promise: await fs.promises.stat(`${root}/file`),
  };
  const fd = fs.openSync(`${root}/file`, "r");
  stats.descriptor = fs.fstatSync(fd);
  fs.closeSync(fd);
  const dir = fs.opendirSync(root);
  const dirShape = shape(dir);
  dir.closeSync();
  return {
    stats: Object.fromEntries(Object.entries(stats).map(([name, stat]) => [name, {
      shape: shape(stat),
      values: statFields.map(field => String(stat[field])),
      instance: stat instanceof fs.Stats,
    }])),
    statsClass: [typeof fs.Stats, fs.Stats.name, attempt(() => new fs.Stats()) ?? null],
    dir: { shape: dirShape, instance: dir instanceof fs.Dir, path: dir.path },
    dirClass: [typeof fs.Dir, fs.Dir.name],
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

// A failing call of each operation: its name and arguments, where `$` starts
// a path in a fresh tree from `errorTree`.
const errorCalls = {
  accessMissing: ["access", "$/missing"],
  appendFileDirectory: ["appendFile", "$/directory", "x"],
  chmodMissing: ["chmod", "$/missing", 0o644],
  chownMissing: ["chown", "$/missing", 0, 0],
  copyFileMissing: ["copyFile", "$/missing", "$/copy"],
  copyFileDirectory: ["copyFile", "$/directory", "$/copy"],
  copyFileExclusive: ["copyFile", "$/file", "$/directory/entry", fs.constants.COPYFILE_EXCL],
  cpMissing: ["cp", "$/missing", "$/copy"],
  lchownMissing: ["lchown", "$/missing", 0, 0],
  linkExisting: ["link", "$/file", "$/directory"],
  linkMissing: ["link", "$/missing", "$/link"],
  lstatMissing: ["lstat", "$/missing"],
  lutimesMissing: ["lutimes", "$/missing", 0, 0],
  mkdirExisting: ["mkdir", "$/directory"],
  mkdirUnderMissing: ["mkdir", "$/missing/child"],
  mkdirUnderFile: ["mkdir", "$/file/child", { recursive: true }],
  mkdtempUnderMissing: ["mkdtemp", "$/missing/t-"],
  openMissing: ["open", "$/missing", "r"],
  openDirectoryForWriting: ["open", "$/directory", "w"],
  openExclusive: ["open", "$/file", "wx"],
  opendirMissing: ["opendir", "$/missing"],
  opendirFile: ["opendir", "$/file"],
  readdirFile: ["readdir", "$/file"],
  readdirMissing: ["readdir", "$/missing"],
  readFileMissing: ["readFile", "$/missing"],
  readFileDirectory: ["readFile", "$/directory"],
  readlinkFile: ["readlink", "$/file"],
  readlinkMissing: ["readlink", "$/missing"],
  realpathMissing: ["realpath", "$/missing"],
  realpathLoop: ["realpath", "$/loop"],
  renameMissing: ["rename", "$/missing", "$/renamed"],
  renameDirectoryOntoFile: ["rename", "$/directory", "$/file"],
  rmMissing: ["rm", "$/missing"],
  rmDirectory: ["rm", "$/directory"],
  rmdirFull: ["rmdir", "$/directory"],
  rmdirFile: ["rmdir", "$/file"],
  rmdirMissing: ["rmdir", "$/missing"],
  statMissing: ["stat", "$/missing"],
  statLoop: ["stat", "$/loop"],
  statfsMissing: ["statfs", "$/missing"],
  symlinkExisting: ["symlink", "$/file", "$/directory"],
  truncateMissing: ["truncate", "$/missing"],
  truncateDirectory: ["truncate", "$/directory"],
  unlinkDirectory: ["unlink", "$/directory"],
  unlinkMissing: ["unlink", "$/missing"],
  utimesMissing: ["utimes", "$/missing", 0, 0],
  writeFileDirectory: ["writeFile", "$/directory", "x"],
  writeFileUnderMissing: ["writeFile", "$/missing/child", "x"],
  writeFileBundle: ["writeFile", "/bundle/new", "x"],
  closeBadDescriptor: ["close", 1000],
  fchmodBadDescriptor: ["fchmod", 1000, 0o644],
  fchownBadDescriptor: ["fchown", 1000, 0, 0],
  fdatasyncBadDescriptor: ["fdatasync", 1000],
  fstatBadDescriptor: ["fstat", 1000],
  fsyncBadDescriptor: ["fsync", 1000],
  ftruncateBadDescriptor: ["ftruncate", 1000],
  futimesBadDescriptor: ["futimes", 1000, 0, 0],
  readBadDescriptor: ["read", 1000, new Uint8Array(1)],
  writeBadDescriptor: ["write", 1000, new Uint8Array(1)],
};

// A fresh tree holding a file, a directory with an entry and a symbolic link
// to itself.
function errorTree(index) {
  const root = `/tmp/filesystem-errors/${index}`;
  fs.mkdirSync(`${root}/directory`, { recursive: true });
  fs.writeFileSync(`${root}/file`, "f");
  fs.writeFileSync(`${root}/directory/entry`, "e");
  fs.symlinkSync(`${root}/loop`, `${root}/loop`);
  return root;
}

// What the sync, callback or promise `form` of `operation` reports for `args`:
// "ok", the failure it throws, or the failure it reports later.
async function errorOutcome(form, operation, args) {
  if (form === "promise" && !fs.promises[operation]) return "absent";
  const settled = Promise.withResolvers();
  try {
    if (form === "sync") fs[`${operation}Sync`](...args);
    else if (form === "callback") fs[operation](...args, error => (error ? settled.reject(error) : settled.resolve()));
    else fs.promises[operation](...args).then(settled.resolve, settled.reject);
  } catch (error) { return { thrown: failure(error) }; }
  if (form === "sync") return "ok";
  try { await settled.promise; return "ok"; } catch (error) { return failure(error); }
}

// Each form of each failing call, each in a fresh tree.
async function errorContracts() {
  const outcomes = {};
  let index = 0;
  for (const [name, [operation, ...args]] of Object.entries(errorCalls)) {
    outcomes[name] = {};
    for (const form of ["sync", "callback", "promise"]) {
      const root = errorTree(index++);
      const resolved = args.map(arg => (typeof arg === "string" ? arg.replace(/^\$/, root) : arg));
      outcomes[name][form] = await errorOutcome(form, operation, resolved);
    }
  }
  return outcomes;
}

export async function filesystemContracts() {
  const result = { write: {} };
  for (const [name, write] of Object.entries(writers)) result.write[name] = await writerOutcomes(write);
  result.writev = {};
  for (const [name, write] of Object.entries(vectorWriters)) result.writev[name] = await vectorOutcomes(write);
  result.append = { uint32Bigint: appendOutcome(2n ** 32n), negativeBigint: appendOutcome(-1n), beyondSizeLimit: appendOutcome(SIZE_LIMIT) };
  result.sizeLimit = sizeLimitOutcomes();
  result.fileWrites = await fileWriteContracts();
  result.handleWriteFile = await handleWriteFileResult();
  result.order = await orderContracts();
  result.dirents = await direntContracts();
  result.readData = await readDataContracts();
  result.glob = await globContracts();
  result.globDetails = await globDetailContracts();
  result.paths = pathContracts();
  result.symlinks = await symlinkContracts();
  result.openFlags = await openFlagContracts();
  result.overwrite = await overwriteContracts();
  result.reads = await readContracts();
  result.encodings = await encodingContracts();
  result.shapes = await shapeContracts();
  result.workingDirectory = workingDirectoryContracts();
  result.errors = await errorContracts();
  return result;
}
