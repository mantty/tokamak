import { objectOf } from "../globals/objects.ts";
import { string } from "../globals/conversions.ts";

// A path's parts, as `parse` gives them and `format` reads them.
interface ParsedPath {
  root: string;
  dir: string;
  base: string;
  ext: string;
  name: string;
}

function pathString(value: unknown): string {
  if (typeof value !== "string") throw new TypeError("The path argument must be of type string");
  return value;
}

function normalizeParts(value: string, separator: string, absolute: boolean): string[] {
  const parts: string[] = [];
  for (const part of value.split(separator)) {
    if (part === "" || part === ".") continue;
    if (part === ".." && parts.length && parts.at(-1) !== "..") parts.pop();
    else if (part !== ".." || !absolute) parts.push(part);
  }
  return parts;
}

function joinWith(parts: unknown[], separator: string, normalize: (value: string) => string): string {
  let joined = "";
  for (const part of parts) {
    const value = pathString(part);
    if (value.length === 0) continue;
    joined = joined === "" ? value : joined + separator + value;
  }
  return joined === "" ? "." : normalize(joined);
}

function basenameOf(input: string, separator: string, suffix: unknown): string {
  let end = input.length;
  while (end > 0 && input[end - 1] === separator) end -= 1;
  const start = input.lastIndexOf(separator, end - 1) + 1;
  const result = input.slice(start, end);
  const ending = suffix === undefined ? "" : pathString(suffix);
  return ending.length > 0 && result.endsWith(ending) ? result.slice(0, -ending.length) : result;
}

function extensionOf(base: string): string {
  const dot = base.lastIndexOf(".");
  return dot <= 0 ? "" : base.slice(dot);
}

// The path `value`'s parts name; without a directory, its `base` unchanged.
function formatWith(value: unknown, separator: string, normalizeDir: (dir: unknown) => string): unknown {
  if (value === null || typeof value !== "object") throw new TypeError("The path object must be of type object");
  const parts = objectOf(value);
  const dir = parts.dir ?? parts.root ?? "";
  const base = parts.base ?? `${string(parts.name ?? "")}${string(parts.ext ?? "")}`;
  if (dir === "") return base;
  let path = normalizeDir(dir);
  if (!path.endsWith(separator)) path += separator;
  path += base;
  return path;
}

function posixNormalize(value: unknown): string {
  const input = pathString(value);
  if (input.length === 0) return ".";
  const absolute = input.startsWith("/");
  const trailing = input.endsWith("/");
  const parts = normalizeParts(input, "/", absolute);
  let result = parts.join("/");
  if (absolute) result = "/" + result;
  if (result === "") result = absolute ? "/" : trailing ? "./" : ".";
  else if (trailing) result += "/";
  return result;
}

function posixJoin(...parts: unknown[]): string { return joinWith(parts, "/", posixNormalize); }

function posixResolve(...parts: unknown[]): string {
  let resolved = "";
  let absolute = false;
  for (let index = parts.length - 1; index >= -1 && !absolute; index -= 1) {
    const value = index < 0 ? "/" : pathString(parts[index]);
    if (value.length === 0) continue;
    resolved = value + "/" + resolved;
    absolute = value.startsWith("/");
  }
  const normalized = normalizeParts(resolved, "/", absolute).join("/");
  return absolute ? "/" + normalized : normalized || ".";
}

function posixIsAbsolute(value: unknown): boolean { return pathString(value).startsWith("/"); }

function posixDirname(value: unknown): string {
  const input = pathString(value);
  if (input.length === 0) return ".";
  let end = input.length - 1;
  while (end >= 0 && input[end] === "/") end -= 1;
  if (end < 0) return "/";
  const slash = input.lastIndexOf("/", end);
  if (slash < 0) return ".";
  let resultEnd = slash;
  while (resultEnd > 0 && input[resultEnd - 1] === "/") resultEnd -= 1;
  return resultEnd === 0 ? "/" : input.slice(0, resultEnd);
}

function posixBasename(value: unknown, suffix?: unknown): string { return basenameOf(pathString(value), "/", suffix); }

function posixExtname(value: unknown): string { return extensionOf(posixBasename(value)); }

function posixParse(value: unknown): ParsedPath {
  const input = pathString(value);
  const root = input.startsWith("/") ? "/" : "";
  const base = posixBasename(input);
  const ext = posixExtname(base);
  const dir = posixDirname(input);
  return { root, dir: dir === "." ? "" : dir, base, ext, name: base.slice(0, base.length - ext.length) };
}

function posixFormat(value: unknown): unknown { return formatWith(value, "/", string); }

function posixRelative(from: unknown, to: unknown): string {
  const left = posixResolve(from).split("/").filter(Boolean);
  const right = posixResolve(to).split("/").filter(Boolean);
  let common = 0;
  while (common < left.length && left[common] === right[common]) common += 1;
  return [...left.slice(common).map(() => ".."), ...right.slice(common)].join("/");
}

function posixMatchesGlob(_path: unknown, _pattern: unknown): never {
  throw new Error("path.posix.matchesGlob() is not implemented.");
}

const posix = {
  normalize: posixNormalize,
  join: posixJoin,
  resolve: posixResolve,
  isAbsolute: posixIsAbsolute,
  dirname: posixDirname,
  basename: posixBasename,
  extname: posixExtname,
  format: posixFormat,
  parse: posixParse,
  relative: posixRelative,
  matchesGlob: posixMatchesGlob,
  toNamespacedPath(value: unknown) { return value; },
  sep: "/",
  delimiter: ":",
};

function winValue(value: unknown): string { return pathString(value).replaceAll("/", "\\"); }

function winRoot(value: unknown): string {
  const input = winValue(value);
  if (input.startsWith("\\\\")) {
    const serverEnd = input.indexOf("\\", 2);
    if (serverEnd < 0) return input + "\\";
    const shareEnd = input.indexOf("\\", serverEnd + 1);
    return shareEnd < 0 ? input + "\\" : input.slice(0, shareEnd + 1);
  }
  if (/^[A-Za-z]:\\/.test(input)) return input.slice(0, 3);
  if (input.startsWith("\\")) return "\\";
  if (/^[A-Za-z]:/.test(input)) return input.slice(0, 2);
  return "";
}

function winNormalize(value: unknown): string {
  const input = winValue(value);
  if (input.length === 0) return ".";
  const root = winRoot(input);
  const absolute = root === "\\" || root.endsWith("\\");
  const rest = input.slice(root.length);
  const trailing = input.endsWith("\\");
  const parts = normalizeParts(rest, "\\", absolute);
  let result = root + parts.join("\\");
  if (result === "") result = absolute ? "\\" : ".";
  else if (trailing && result !== root) result += "\\";
  return result;
}

function winJoin(...parts: unknown[]): string { return joinWith(parts, "\\", winNormalize); }

function winResolve(...parts: unknown[]): string {
  let root = "";
  const values: string[] = [];
  for (let index = parts.length - 1; index >= 0; index -= 1) {
    const value = winValue(parts[index]);
    if (value === "") continue;
    if (value.startsWith("\\") || /^[A-Za-z]:/.test(value)) {
      const valueRoot = winRoot(value);
      if (valueRoot.endsWith("\\") || valueRoot === "\\") {
        root = valueRoot;
        values.unshift(value.slice(valueRoot.length));
      } else {
        root = valueRoot + "\\";
        values.unshift(value.slice(valueRoot.length));
      }
      break;
    }
    values.unshift(value);
  }
  if (root === "") root = "\\";
  const normalized = normalizeParts(values.join("\\"), "\\", true).join("\\");
  return root + normalized;
}

function winIsAbsolute(value: unknown): boolean {
  const input = winValue(value);
  return /^[A-Za-z]:\\/.test(input) || input.startsWith("\\");
}

function winBasename(value: unknown, suffix?: unknown): string { return basenameOf(winValue(value), "\\", suffix); }

function winDirname(value: unknown): string {
  const input = winValue(value);
  if (input.length === 0) return ".";
  const root = winRoot(input);
  let end = input.length - 1;
  while (end >= root.length && input[end] === "\\") end -= 1;
  const slash = input.lastIndexOf("\\", end);
  if (slash < root.length) return root || ".";
  return input.slice(0, slash) || root || ".";
}

function winExtname(value: unknown): string { return extensionOf(winBasename(value)); }

function winParse(value: unknown): ParsedPath {
  const input = winValue(value);
  const root = winRoot(input);
  const base = winBasename(input);
  const ext = winExtname(base);
  const dir = winDirname(input);
  return { root, dir: dir === "." ? "" : dir, base, ext, name: base.slice(0, base.length - ext.length) };
}

function winFormat(value: unknown): unknown { return formatWith(value, "\\", winValue); }

function winRelative(from: unknown, to: unknown): string {
  const left = winResolve(from);
  const right = winResolve(to);
  const leftRoot = winRoot(left).toLowerCase();
  const rightRoot = winRoot(right).toLowerCase();
  if (leftRoot !== rightRoot) return right;
  const fromParts = left.slice(winRoot(left).length).split("\\").filter(Boolean).map(value => value.toLowerCase());
  const toParts = right.slice(winRoot(right).length).split("\\").filter(Boolean);
  let common = 0;
  while (common < fromParts.length && common < toParts.length && fromParts[common] === toParts[common].toLowerCase()) common += 1;
  return [...fromParts.slice(common).map(() => ".."), ...toParts.slice(common)].join("\\");
}

function winMatchesGlob(_path: unknown, _pattern: unknown): never {
  throw new Error("path.win32.matchesGlob() is not implemented.");
}

const win32 = {
  normalize: winNormalize,
  join: winJoin,
  resolve: winResolve,
  isAbsolute: winIsAbsolute,
  dirname: winDirname,
  basename: winBasename,
  extname: winExtname,
  format: winFormat,
  parse: winParse,
  relative: winRelative,
  matchesGlob: winMatchesGlob,
  toNamespacedPath(value: unknown) { return value; },
  sep: "\\",
  delimiter: ";",
};

Object.assign(posix, { posix, win32 });
Object.assign(win32, { posix, win32 });

export { posix, win32 };
export const {
  normalize, join, resolve, isAbsolute, dirname, basename, extname, format, parse, relative, matchesGlob, toNamespacedPath, sep,
  delimiter,
} = posix;
export default posix;
