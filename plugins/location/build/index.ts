// The location plugin's build script: tok runs it for each platform with the plugin's resolved
// build settings on stdin. It writes the plugin's Info.plist or Android manifest into `output`
// and prints the keys that name it and an Apple build's compilation conditions.
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import type { BuildScriptInput } from "@tokamakdev/plugin/contracts";
import { declarations, type Settings } from "./declarations.ts";

const { platform, settings, output } = JSON.parse(readFileSync(0, "utf8")) as BuildScriptInput;
// tok has checked the settings against the plugin's manifest and applied their defaults.
const result = declarations(platform, settings as unknown as Settings);
if ("problems" in result) {
  console.error(result.problems.join("\n"));
  process.exit(1);
}
const { key, name, contents, notices, conditions } = result;
for (const notice of notices) console.error(notice);
const path = join(output, name);
writeFileSync(path, contents);
console.log(JSON.stringify({ [key]: path, "compilation-conditions": conditions }));
