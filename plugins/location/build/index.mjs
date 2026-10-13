// The location plugin's build script: tok runs it for each platform with the plugin's resolved
// build settings on stdin. It writes the plugin's Info.plist or Android manifest into `output`
// and prints the key that names it.
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { declarations } from "./declarations.mjs";

const { platform, settings, output } = JSON.parse(readFileSync(0, "utf8"));
const { key, name, contents, notices, problems } = declarations(platform, settings);
if (problems) {
  console.error(problems.join("\n"));
  process.exit(1);
}
for (const notice of notices) console.error(notice);
const path = join(output, name);
writeFileSync(path, contents);
console.log(JSON.stringify({ [key]: path }));
