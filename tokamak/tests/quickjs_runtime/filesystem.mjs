import fs from "node:fs";
import { byteInputs } from "./bytes.mjs";

const SIZE_LIMIT = 128 * 1024 * 1024;

function failure(error) {
  return { error: error.name, code: error.code ?? null };
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

export async function filesystemContracts() {
  const result = { write: {} };
  for (const [name, write] of Object.entries(writers)) result.write[name] = await writerOutcomes(write);
  result.writev = {};
  for (const [name, write] of Object.entries(vectorWriters)) result.writev[name] = await vectorOutcomes(write);
  result.append = { uint32Bigint: appendOutcome(2n ** 32n), negativeBigint: appendOutcome(-1n), beyondSizeLimit: appendOutcome(SIZE_LIMIT) };
  result.sizeLimit = sizeLimitOutcomes();
  result.glob = await globContracts();
  return result;
}
