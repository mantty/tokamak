import fs from "node:fs";

// The trailing arguments each write takes after its data.
const writeArguments = {
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
};

const writers = {
  sync: (path, data, args) => withDescriptor(path, fd => fs.writeSync(fd, data, ...args)),
  callback: (path, data, args) => withDescriptor(path, fd => new Promise((resolve, reject) =>
    fs.write(fd, data, ...args, (error, count) => error ? reject(error) : resolve(count)))),
  handle: async (path, data, args) => {
    const handle = await fs.promises.open(path, "r+");
    try { return (await handle.write(data, ...args)).bytesWritten; }
    finally { await handle.close(); }
  },
};

async function withDescriptor(path, use) {
  const fd = fs.openSync(path, "r+");
  try { return await use(fd); }
  finally { fs.closeSync(fd); }
}

// The count `write` reports for [1, 2, 3, 4] over six 9s, and the file it leaves.
async function writeOutcome(write) {
  const path = "/tmp/filesystem-contract.bin";
  fs.writeFileSync(path, new Uint8Array(6).fill(9));
  let written;
  try { written = await write(path, Uint8Array.from([1, 2, 3, 4])); }
  catch (error) { written = { error: error.name }; }
  return { written, contents: [...fs.readFileSync(path)] };
}

// The outcome of each of `writeArguments` with `write`, one at a time.
async function writeOutcomes(write) {
  const outcomes = {};
  for (const [name, args] of Object.entries(writeArguments)) {
    outcomes[name] = await writeOutcome((path, data) => write(path, data, args));
  }
  return outcomes;
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
  const result = { write: {}, writevPositions: [] };
  for (const [name, write] of Object.entries(writers)) result.write[name] = await writeOutcomes(write);
  for (const position of [undefined, null, 1]) {
    result.writevPositions.push(await writeOutcome(path =>
      withDescriptor(path, fd => fs.writevSync(fd, [Uint8Array.from([1, 2])], position))));
  }
  result.glob = await globContracts();
  return result;
}
