// workerd's node:fs glob functions, over the host's matching.
import { isIterable } from "../../globals/objects.ts";

/** `fs.promises.glob`, which yields what `globSync` matches. */
export function globPromise(globSync: (pattern: unknown, options: unknown) => unknown[]) {
  return async function* glob(pattern: unknown, options: unknown = {}) {
    const results = await Promise.resolve(globSync(pattern, options));
    for (const result of results) {
      yield result;
    }
  };
}

/** workerd's reading of a glob `pattern`: a string, or the values an iterable spreads to. */
export function globPatterns(pattern: unknown) {
  if (typeof pattern === "string") return [pattern];
  if (!isIterable(pattern)) throw new TypeError("pattern is not iterable");
  return [...pattern];
}
