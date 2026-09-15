import { posix, win32 } from "./path.mjs";

export const { normalize, join, resolve, isAbsolute, dirname, basename, extname, format, parse, relative, matchesGlob, toNamespacedPath, sep, delimiter } = win32;
export { posix, win32 };
export default win32;
