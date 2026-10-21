// A project build over Worker output already in dist/app: as the tokamak
// Vite plugin, it reports vite-report.json in $TOKAMAK_VITE_OUTPUT.
const fs = require("node:fs");
const path = require("node:path");

const output = process.env.TOKAMAK_VITE_OUTPUT;
// Node and Vite mishandle Windows verbatim paths (\\?\D:\...), which tok must not pass.
for (const value of [output, process.cwd()]) {
  if (value.startsWith("\\\\?\\")) throw new Error(`tok passed a verbatim path: ${value}`);
}
fs.mkdirSync(output, { recursive: true });
fs.copyFileSync("vite-report.json", path.join(output, "config.json"));
