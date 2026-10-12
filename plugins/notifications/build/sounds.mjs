// Checks the files of the `sounds` build setting for the platform being built.
import { readdirSync, readFileSync, statSync } from "node:fs";
import { extname, join } from "node:path";

/** The longest sound Apple plays; it plays the default sound for longer ones. */
const APPLE_MAX_SECONDS = 30;

/** Audio Android plays from a raw resource, by file extension. */
const ANDROID_EXTENSIONS = new Set([".wav", ".mp3", ".ogg", ".m4a", ".aac", ".flac"]);

/** WAVE format codes Apple plays: Linear PCM, IEEE float, aLaw, µLaw and IMA ADPCM. */
const WAVE_FORMATS = new Set([0x0001, 0x0003, 0x0006, 0x0007, 0x0011]);

/** AIFF-C compression types Apple plays. */
const AIFC_COMPRESSIONS = new Set(["NONE", "sowt", "twos", "raw ", "fl32", "FL32", "fl64", "FL64", "ima4", "ulaw", "ULAW", "alaw", "ALAW"]);

/** Core Audio format IDs Apple plays. */
const CAF_FORMATS = new Set(["lpcm", "ima4", "ulaw", "alaw"]);

/**
 * The plugin section keys that bundle the sound files in `directory` on `platform`, or the
 * problems with them: Apple's `resources` and Android's `raw`.
 */
export function bundledSounds(platform, directory) {
  let names;
  try {
    names = readdirSync(directory);
  } catch {
    return { problems: [`notifications.sounds must be a directory: ${directory}`] };
  }
  const problems = [];
  const files = names
    .filter((name) => {
      if (name.startsWith(".")) return false;
      try {
        // statSync follows symbolic links to the files they name.
        return statSync(join(directory, name)).isFile();
      } catch {
        problems.push(`notifications sound ${name} is a link to nothing`);
        return false;
      }
    })
    .sort();
  const resources = new Map();
  for (const name of files) {
    const path = join(directory, name);
    const problem = platform === "android" ? androidProblem(name, resources) : appleProblem(readFileSync(path));
    if (problem) problems.push(`notifications sound ${name} ${problem}`);
    resources.set(resourceName(name), name);
  }
  if (problems.length > 0) return { problems };
  const paths = files.map((name) => join(directory, name));
  return { keys: platform === "android" ? { raw: paths } : { resources: paths } };
}

/** Android's raw resource name for the file `name`: the name without its extension. */
function resourceName(name) {
  return name.slice(0, name.length - extname(name).length);
}

function androidProblem(name, resources) {
  if (!ANDROID_EXTENSIONS.has(extname(name).toLowerCase())) {
    return `must be ${[...ANDROID_EXTENSIONS].join(", ")} on Android`;
  }
  if (!/^[a-z][a-z0-9_]*\.[a-z0-9]+$/.test(name)) {
    return "must be named with lowercase letters, digits and underscores, starting with a letter, on Android";
  }
  const other = resources.get(resourceName(name));
  if (other !== undefined) return `has the same Android resource name as ${other}`;
  return undefined;
}

function appleProblem(bytes) {
  let sound;
  try {
    sound = appleSound(bytes);
  } catch (error) {
    return error.message;
  }
  if (sound === undefined) {
    return "must be AIFF, WAV or CAF on Apple platforms";
  }
  if (!sound.playable) {
    return `must be Linear PCM, IMA4, µLaw or aLaw on Apple platforms, not ${sound.format}`;
  }
  if (!(sound.seconds <= APPLE_MAX_SECONDS)) {
    return `must be at most ${String(APPLE_MAX_SECONDS)} seconds on Apple platforms, not ${sound.seconds.toFixed(1)}`;
  }
  return undefined;
}

/**
 * The format of the AIFF, WAV or CAF file in `bytes`, whether Apple plays it in a notification,
 * and its length in seconds; undefined for another kind of file. Throws when the file is
 * malformed.
 */
export function appleSound(bytes) {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const tag = (offset) => String.fromCharCode(...bytes.subarray(offset, offset + 4));
  const [kind, read] = reader(view, tag);
  if (read === undefined) return undefined;
  try {
    return read();
  } catch (error) {
    // A chunk that runs past the end of the file.
    if (error instanceof RangeError) throw malformed(kind);
    throw error;
  }
}

/** The kind of sound file in `view` and the function that reads it, or nothing. */
function reader(view, tag) {
  const length = view.byteLength;
  if (length >= 12 && tag(0) === "RIFF" && tag(8) === "WAVE") return ["WAV", () => wave(view, tag)];
  if (length >= 12 && tag(0) === "FORM" && (tag(8) === "AIFF" || tag(8) === "AIFC")) {
    return ["AIFF", () => aiff(view, tag, tag(8) === "AIFC")];
  }
  if (length >= 8 && tag(0) === "caff") return ["CAF", () => caf(view, tag)];
  return [];
}

/** Each chunk after the header at `start`: its tag, the offset of its data and its size. */
function* chunks(view, tag, start, sizeOf, headerSize) {
  let offset = start;
  while (offset + headerSize <= view.byteLength) {
    const size = sizeOf(offset + 4);
    yield { tag: tag(offset), data: offset + headerSize, size };
    if (size < 0) return;
    offset += headerSize + size + (headerSize === 8 ? size % 2 : 0);
  }
}

function malformed(kind) {
  return new Error(`is not a valid ${kind} file`);
}

function wave(view, tag) {
  let format;
  let byteRate;
  let dataSize;
  for (const chunk of chunks(view, tag, 12, (offset) => view.getUint32(offset, true), 8)) {
    if (chunk.tag === "fmt " && chunk.size >= 16) {
      format = view.getUint16(chunk.data, true);
      byteRate = view.getUint32(chunk.data + 8, true);
      // WAVE_FORMAT_EXTENSIBLE names the format in its subformat GUID.
      if (format === 0xfffe && chunk.size >= 40) format = view.getUint16(chunk.data + 24, true);
    }
    if (chunk.tag === "data") dataSize = Math.min(chunk.size, view.byteLength - chunk.data);
  }
  if (format === undefined || dataSize === undefined || !(byteRate > 0)) throw malformed("WAV");
  return { format: `WAVE format 0x${format.toString(16)}`, playable: WAVE_FORMATS.has(format), seconds: dataSize / byteRate };
}

function aiff(view, tag, compressed) {
  for (const chunk of chunks(view, tag, 12, (offset) => view.getUint32(offset), 8)) {
    if (chunk.tag !== "COMM" || chunk.size < 18) continue;
    const frames = view.getUint32(chunk.data + 2);
    const rate = extended(view, chunk.data + 8);
    const compression = compressed && chunk.size >= 22 ? tag(chunk.data + 18) : "NONE";
    if (!(rate > 0)) throw malformed("AIFF");
    return { format: `AIFF ${compression}`, playable: AIFC_COMPRESSIONS.has(compression), seconds: frames / rate };
  }
  throw malformed("AIFF");
}

/** The 80-bit IEEE 754 extended-precision number at `offset`, as AIFF stores sample rates. */
function extended(view, offset) {
  const exponent = view.getUint16(offset) & 0x7fff;
  const mantissa = Number(view.getBigUint64(offset + 2));
  return mantissa * 2 ** (exponent - 16383 - 63);
}

function caf(view, tag) {
  let description;
  let dataSize;
  for (const chunk of chunks(view, tag, 8, (offset) => Number(view.getBigInt64(offset)), 12)) {
    if (chunk.tag === "desc" && chunk.size >= 32) {
      description = {
        rate: view.getFloat64(chunk.data),
        format: tag(chunk.data + 8),
        bytesPerPacket: view.getUint32(chunk.data + 16),
        framesPerPacket: view.getUint32(chunk.data + 20),
      };
    }
    if (chunk.tag === "data") {
      // A size of -1 runs to the end of the file; the data starts with a 4-byte edit count.
      const size = chunk.size < 0 ? view.byteLength - chunk.data : Math.min(chunk.size, view.byteLength - chunk.data);
      dataSize = size - 4;
    }
  }
  if (description === undefined || dataSize === undefined || !(description.rate > 0)) throw malformed("CAF");
  const { rate, format, bytesPerPacket, framesPerPacket } = description;
  // The formats Apple plays have constant packet sizes.
  const playable = CAF_FORMATS.has(format) && bytesPerPacket > 0 && framesPerPacket > 0;
  const seconds = playable ? (Math.floor(dataSize / bytesPerPacket) * framesPerPacket) / rate : 0;
  return { format: `CAF ${format.trim()}`, playable, seconds };
}
