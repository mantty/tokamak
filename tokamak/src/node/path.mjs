function pathString(value) {
  if (typeof value !== "string") throw new TypeError("The path argument must be of type string");
  return value;
}

function integer(value) {
  const number = Number(value);
  return Number.isNaN(number) || number === 0 ? 0 : number < 0 ? Math.ceil(number) : Math.floor(number);
}

function normalizeParts(value, separator, absolute) {
  const parts = [];
  for (const part of value.split(separator)) {
    if (part === "" || part === ".") continue;
    if (part === ".." && parts.length && parts.at(-1) !== "..") parts.pop();
    else if (part !== ".." || !absolute) parts.push(part);
  }
  return parts;
}

function posixNormalize(value) {
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

function posixJoin(...parts) {
  let joined = "";
  for (const part of parts) {
    const value = pathString(part);
    if (value.length === 0) continue;
    joined = joined === "" ? value : joined + "/" + value;
  }
  return joined === "" ? "." : posixNormalize(joined);
}

function posixResolve(...parts) {
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

function posixIsAbsolute(value) { return pathString(value).startsWith("/"); }

function posixDirname(value) {
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

function posixBasename(value, suffix) {
  const input = pathString(value);
  let end = input.length;
  while (end > 0 && input[end - 1] === "/") end -= 1;
  const start = input.lastIndexOf("/", end - 1) + 1;
  let result = input.slice(start, end);
  if (suffix !== undefined && pathString(suffix).length > 0 && result.endsWith(suffix)) result = result.slice(0, -suffix.length);
  return result;
}

function posixExtname(value) {
  const base = posixBasename(value);
  const dot = base.lastIndexOf(".");
  if (dot <= 0) return "";
  return base.slice(dot);
}

function posixParse(value) {
  const input = pathString(value);
  const root = input.startsWith("/") ? "/" : "";
  const base = posixBasename(input);
  const ext = posixExtname(base);
  const dir = posixDirname(input);
  return { root, dir: dir === "." ? "" : dir, base, ext, name: base.slice(0, base.length - ext.length) };
}

function posixFormat(value) {
  if (value === null || typeof value !== "object") throw new TypeError("The path object must be of type object");
  const dir = value.dir ?? value.root ?? "";
  const base = value.base ?? `${value.name ?? ""}${value.ext ?? ""}`;
  if (dir === "") return base;
  return dir.endsWith("/") ? dir + base : dir + "/" + base;
}

function posixRelative(from, to) {
  const left = posixResolve(pathString(from)).split("/").filter(Boolean);
  const right = posixResolve(pathString(to)).split("/").filter(Boolean);
  let common = 0;
  while (common < left.length && left[common] === right[common]) common += 1;
  return [...left.slice(common).map(() => ".."), ...right.slice(common)].join("/");
}

function posixMatchesGlob(path, pattern) {
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
  toNamespacedPath(value) { return value; },
  sep: "/",
  delimiter: ":",
};

function winValue(value) { return pathString(value).replaceAll("/", "\\"); }

function winRoot(value) {
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

function winNormalize(value) {
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

function winJoin(...parts) {
  let joined = "";
  for (const part of parts) {
    const value = pathString(part);
    if (value.length === 0) continue;
    joined = joined === "" ? value : joined + "\\" + value;
  }
  return joined === "" ? "." : winNormalize(joined);
}

function winResolve(...parts) {
  let root = "";
  const values = [];
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

function winIsAbsolute(value) {
  const input = winValue(value);
  return /^[A-Za-z]:\\/.test(input) || input.startsWith("\\");
}

function winBasename(value, suffix) {
  const input = winValue(value);
  let end = input.length;
  while (end > 0 && input[end - 1] === "\\") end -= 1;
  const start = input.lastIndexOf("\\", end - 1) + 1;
  let result = input.slice(start, end);
  if (suffix !== undefined && pathString(suffix).length > 0 && result.endsWith(suffix)) result = result.slice(0, -suffix.length);
  return result;
}

function winDirname(value) {
  const input = winValue(value);
  if (input.length === 0) return ".";
  const root = winRoot(input);
  let end = input.length - 1;
  while (end >= root.length && input[end] === "\\") end -= 1;
  const slash = input.lastIndexOf("\\", end);
  if (slash < root.length) return root || ".";
  return input.slice(0, slash) || root || ".";
}

function winExtname(value) {
  const base = winBasename(value);
  const dot = base.lastIndexOf(".");
  return dot <= 0 ? "" : base.slice(dot);
}

function winParse(value) {
  const input = winValue(value);
  const root = winRoot(input);
  const base = winBasename(input);
  const ext = winExtname(base);
  const dir = winDirname(input);
  return { root, dir: dir === "." ? "" : dir, base, ext, name: base.slice(0, base.length - ext.length) };
}

function winFormat(value) {
  if (value === null || typeof value !== "object") throw new TypeError("The path object must be of type object");
  const dir = value.dir ?? value.root ?? "";
  const base = value.base ?? `${value.name ?? ""}${value.ext ?? ""}`;
  if (dir === "") return base;
  const normalized = winValue(dir);
  return normalized.endsWith("\\") ? normalized + base : normalized + "\\" + base;
}

function winRelative(from, to) {
  const left = winResolve(pathString(from));
  const right = winResolve(pathString(to));
  const leftRoot = winRoot(left).toLowerCase();
  const rightRoot = winRoot(right).toLowerCase();
  if (leftRoot !== rightRoot) return right;
  const fromParts = left.slice(winRoot(left).length).split("\\").filter(Boolean).map(value => value.toLowerCase());
  const toParts = right.slice(winRoot(right).length).split("\\").filter(Boolean);
  let common = 0;
  while (common < fromParts.length && fromParts[common] === toParts[common].toLowerCase()) common += 1;
  return [...fromParts.slice(common).map(() => ".."), ...toParts.slice(common)].join("\\");
}

function winMatchesGlob(path, pattern) {
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
  toNamespacedPath(value) { return value; },
  sep: "\\",
  delimiter: ";",
};

Object.assign(posix, { posix, win32 });
Object.assign(win32, { posix, win32 });

export { posix, win32 };
export const normalize = posix.normalize;
export const join = posix.join;
export const resolve = posix.resolve;
export const isAbsolute = posix.isAbsolute;
export const dirname = posix.dirname;
export const basename = posix.basename;
export const extname = posix.extname;
export const format = posix.format;
export const parse = posix.parse;
export const relative = posix.relative;
export const matchesGlob = posix.matchesGlob;
export const toNamespacedPath = posix.toNamespacedPath;
export const sep = posix.sep;
export const delimiter = posix.delimiter;
export default posix;
