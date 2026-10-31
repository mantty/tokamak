// Writes each TypeScript module, named by its path under <from>, to the same
// path under <to> with its types replaced by spaces, so every line and column
// stays where it was. tokamak/build.rs runs it before compiling the runtime's
// modules to QuickJS bytecode, which has no source maps.
//
//   node scripts/strip-types.mts <from> <to> <module>...
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { stripTypeScriptTypes } from "node:module";
import path from "node:path";

const [from, to, ...modules] = process.argv.slice(2);
if (from === undefined || to === undefined) {
  throw new Error("usage: node scripts/strip-types.mts <from> <to> <module>...");
}
for (const name of modules) {
  const source = path.join(from, name);
  const javascript = path.join(to, name);
  mkdirSync(path.dirname(javascript), { recursive: true });
  writeFileSync(javascript, stripped(source));
}

function stripped(source: string): string {
  try {
    return stripTypeScriptTypes(readFileSync(source, "utf8"));
  } catch (error) {
    throw new Error(`${source}: ${String(error)}`, { cause: error });
  }
}
