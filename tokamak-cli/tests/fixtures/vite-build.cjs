// A project build over Worker output already in dist/app: as the tokamak
// Vite plugin, it reports vite-report.json in $TOKAMAK_VITE_OUTPUT.
const fs = require("node:fs");
const path = require("node:path");

const output = process.env.TOKAMAK_VITE_OUTPUT;
fs.mkdirSync(output, { recursive: true });
fs.copyFileSync("vite-report.json", path.join(output, "config.json"));
