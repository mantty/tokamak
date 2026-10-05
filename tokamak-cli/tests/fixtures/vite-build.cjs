// A project build over Worker output already in dist/app. As Cloudflare's
// Vite plugin, it applies CLOUDFLARE_ENV to the generated configuration and
// names the Worker after it; as tokamak's, it reports the `config` export of
// src/tokamak.mjs, or of TOKAMAK_CONFIG, in $TOKAMAK_VITE_OUTPUT.
const fs = require("node:fs");
const path = require("node:path");
const { pathToFileURL } = require("node:url");

async function main() {
  const generated = "dist/app/wrangler.json";
  const environment = process.env.CLOUDFLARE_ENV;
  if (environment) {
    const config = JSON.parse(fs.readFileSync(generated, "utf8"));
    config.topLevelName ??= config.name;
    config.name = `${config.topLevelName}-${environment}`;
    config.vars = { ...config.vars, CLOUDFLARE_ENV: environment };
    fs.writeFileSync(generated, JSON.stringify(config));
  }
  const output = process.env.TOKAMAK_VITE_OUTPUT;
  const file = process.env.TOKAMAK_CONFIG ?? path.resolve("src/tokamak.mjs");
  const report = fs.existsSync(file)
    ? { file, config: (await import(pathToFileURL(file).href)).config ?? {} }
    : { config: {} };
  fs.mkdirSync(output, { recursive: true });
  fs.writeFileSync(path.join(output, "config.json"), JSON.stringify(report));
}

main();
