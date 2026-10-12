import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";

import { appleSound, bundledSounds } from "../build/sounds.mjs";

/** A WAV file of `seconds` of 8 kHz mono audio in WAVE format `format`. */
function wav(seconds: number, format = 1): Uint8Array {
  const rate = 8000;
  const data = rate * 2 * seconds;
  const bytes = new Uint8Array(44 + data);
  const view = new DataView(bytes.buffer);
  bytes.set(new TextEncoder().encode("RIFF"), 0);
  view.setUint32(4, 36 + data, true);
  bytes.set(new TextEncoder().encode("WAVEfmt "), 8);
  view.setUint32(16, 16, true);
  view.setUint16(20, format, true);
  view.setUint16(22, 1, true);
  view.setUint32(24, rate, true);
  view.setUint32(28, rate * 2, true);
  view.setUint16(32, 2, true);
  view.setUint16(34, 16, true);
  bytes.set(new TextEncoder().encode("data"), 36);
  view.setUint32(40, data, true);
  return bytes;
}

/** An AIFF-C file of `seconds` at 44.1 kHz with `compression`. */
function aifc(seconds: number, compression: string): Uint8Array {
  const bytes = new Uint8Array(12 + 8 + 22);
  const view = new DataView(bytes.buffer);
  bytes.set(new TextEncoder().encode("FORM"), 0);
  view.setUint32(4, bytes.length - 8);
  bytes.set(new TextEncoder().encode("AIFCCOMM"), 8);
  view.setUint32(16, 22);
  view.setUint16(20, 1);
  view.setUint32(22, 44100 * seconds);
  view.setUint16(26, 16);
  // 44100 as an 80-bit extended number.
  view.setUint16(28, 0x400e);
  view.setBigUint64(30, 0xac44000000000000n);
  bytes.set(new TextEncoder().encode(compression), 38);
  return bytes;
}

/** A CAF file of `packets` packets of `format`, each `bytesPerPacket` bytes of 64 frames at 32 kHz. */
function caf(format: string, packets: number, bytesPerPacket: number): Uint8Array {
  const data = packets * bytesPerPacket;
  const bytes = new Uint8Array(8 + 12 + 32 + 12 + 4 + data);
  const view = new DataView(bytes.buffer);
  bytes.set(new TextEncoder().encode("caff"), 0);
  view.setUint16(4, 1);
  bytes.set(new TextEncoder().encode("desc"), 8);
  view.setBigInt64(12, 32n);
  view.setFloat64(20, 32000);
  bytes.set(new TextEncoder().encode(format), 28);
  view.setUint32(36, bytesPerPacket);
  view.setUint32(40, 64);
  view.setUint32(44, 1);
  bytes.set(new TextEncoder().encode("data"), 52);
  view.setBigInt64(56, -1n);
  return bytes;
}

/** `bytes` with the four-byte tag `tag` at `offset`. */
function tagged(bytes: Uint8Array, offset: number, tag: string): Uint8Array {
  bytes.set(new TextEncoder().encode(tag), offset);
  return bytes;
}

void test("reads extensible WAVs, plain AIFFs and CAFs of a given size", () => {
  const extensible = new Uint8Array(12 + 8 + 40 + 8 + 16000);
  const view = new DataView(extensible.buffer);
  tagged(extensible, 0, "RIFF");
  tagged(extensible, 8, "WAVE");
  tagged(extensible, 12, "fmt ");
  view.setUint32(16, 40, true);
  view.setUint16(20, 0xfffe, true);
  view.setUint32(28, 16000, true);
  view.setUint16(44, 1, true);
  tagged(extensible, 60, "data");
  view.setUint32(64, 16000, true);
  assert.deepEqual(appleSound(extensible), { format: "WAVE format 0x1", playable: true, seconds: 1 });

  const aiff = aifc(2, "NONE");
  tagged(aiff, 8, "AIFF");
  assert.deepEqual(appleSound(aiff), { format: "AIFF NONE", playable: true, seconds: 2 });
  assert.throws(() => appleSound(tagged(aifc(2, "NONE"), 12, "SSND")), /is not a valid AIFF file/);

  const sized = caf("lpcm", 32000, 2);
  new DataView(sized.buffer).setBigInt64(56, BigInt(4 + 32000 * 2));
  assert.deepEqual(appleSound(sized), { format: "CAF lpcm", playable: true, seconds: 64 });
});

void test("reads the format and length of WAV, AIFF and CAF files", () => {
  assert.deepEqual(appleSound(wav(2)), { format: "WAVE format 0x1", playable: true, seconds: 2 });
  assert.deepEqual(appleSound(wav(1, 0x55)), { format: "WAVE format 0x55", playable: false, seconds: 1 });
  assert.deepEqual(appleSound(aifc(3, "ulaw")), { format: "AIFF ulaw", playable: true, seconds: 3 });
  assert.deepEqual(appleSound(aifc(3, "ACE2")), { format: "AIFF ACE2", playable: false, seconds: 3 });
  assert.deepEqual(appleSound(caf("ima4", 1000, 34)), { format: "CAF ima4", playable: true, seconds: 2 });
  assert.deepEqual(appleSound(caf("aac ", 1000, 0)), { format: "CAF aac", playable: false, seconds: 0 });
  assert.equal(appleSound(new TextEncoder().encode("ID3 an mp3 file")), undefined);
  assert.throws(() => appleSound(wav(1).subarray(0, 30)), /is not a valid WAV file/);
});

/** A directory holding `files`, by name. */
function directory(files: Record<string, Uint8Array>): string {
  const path = mkdtempSync(join(tmpdir(), "sounds-"));
  for (const [name, bytes] of Object.entries(files)) writeFileSync(join(path, name), bytes);
  return path;
}

void test("bundles Apple sounds at the top level and Android sounds as raw resources", () => {
  const path = directory({ "chime.wav": wav(1), "alert.caf": caf("ima4", 10, 34), ".DS_Store": new Uint8Array() });

  assert.deepEqual(bundledSounds("ios", path), {
    keys: { resources: [join(path, "alert.caf"), join(path, "chime.wav")] },
  });
  assert.equal(bundledSounds("macos", path).keys?.resources.length, 2);
  const android = directory({ "chime.wav": wav(1), "ping.ogg": new Uint8Array() });
  assert.deepEqual(bundledSounds("android", android), {
    keys: { raw: [join(android, "chime.wav"), join(android, "ping.ogg")] },
  });
});

void test("follows symbolic links to sound files", () => {
  const target = directory({ "chime.wav": wav(1) });
  const path = directory({});
  symlinkSync(join(target, "chime.wav"), join(path, "linked.wav"));

  assert.deepEqual(bundledSounds("android", path), { keys: { raw: [join(path, "linked.wav")] } });

  symlinkSync(join(target, "missing.wav"), join(path, "broken.wav"));
  assert.deepEqual(bundledSounds("android", path).problems, ["notifications sound broken.wav is a link to nothing"]);
});

const BUILD_SCRIPT = join(dirname(fileURLToPath(import.meta.url)), "../build/index.mjs");

/** What the build script prints and exits with for `input`. */
function build(input: unknown): { status: number | null; stdout: string; stderr: string } {
  const { status, stdout, stderr } = spawnSync(process.execPath, [BUILD_SCRIPT], {
    input: JSON.stringify(input),
    encoding: "utf8",
  });
  return { status, stdout: stdout.trim(), stderr: stderr.trim() };
}

void test("the build script adds the sounds, nothing without them, and fails on bad ones", () => {
  const path = directory({ "chime.wav": wav(1) });
  assert.deepEqual(build({ platform: "ios", settings: {} }), { status: 0, stdout: "{}", stderr: "" });
  assert.deepEqual(build({ platform: "macos", settings: { sounds: path } }), {
    status: 0,
    stdout: JSON.stringify({ resources: [join(path, "chime.wav")] }),
    stderr: "",
  });
  assert.deepEqual(build({ platform: "android", settings: { sounds: "/no/such" } }), {
    status: 1,
    stdout: "",
    stderr: "notifications.sounds must be a directory: /no/such",
  });
});

void test("reports each sound a platform cannot play or name", () => {
  const apple = bundledSounds("ios", directory({ "long.wav": wav(31), "song.mp3": new Uint8Array(4), "ace.aifc": aifc(1, "ACE2") }));
  assert.deepEqual(apple.problems, [
    "notifications sound ace.aifc must be Linear PCM, IMA4, µLaw or aLaw on Apple platforms, not AIFF ACE2",
    "notifications sound long.wav must be at most 30 seconds on Apple platforms, not 31.0",
    "notifications sound song.mp3 must be AIFF, WAV or CAF on Apple platforms",
  ]);

  const android = bundledSounds("android", directory({ "Chime.wav": wav(1), "ping.wav": wav(1), "ping.ogg": new Uint8Array(), "alert.caf": new Uint8Array(), "loud.WAV": wav(1) }));
  assert.deepEqual(android.problems, [
    "notifications sound Chime.wav must be named with lowercase letters, digits and underscores, starting with a letter, on Android",
    "notifications sound alert.caf must be .wav, .mp3, .ogg, .m4a, .aac, .flac on Android",
    "notifications sound loud.WAV must be named with lowercase letters, digits and underscores, starting with a letter, on Android",
    "notifications sound ping.wav has the same Android resource name as ping.ogg",
  ]);

  assert.deepEqual(bundledSounds("ios", "/no/such/directory").problems, [
    "notifications.sounds must be a directory: /no/such/directory",
  ]);
});
