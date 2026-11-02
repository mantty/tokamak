// The notifications plugin's build script: tok runs it for each platform with the plugin's
// resolved build settings on stdin, and it prints the keys to add to the plugin's section.
import { readFileSync } from "node:fs";
import type { BuildScriptInput } from "@tokamakdev/plugin/contracts";
import { bundledSounds } from "./sounds.ts";

const { platform, settings } = JSON.parse(readFileSync(0, "utf8")) as BuildScriptInput;
if (settings.sounds === undefined) {
  console.log("{}");
} else {
  // tok resolves the path setting `sounds` to an absolute path.
  const { keys, problems } = bundledSounds(platform, settings.sounds as string);
  if (problems) {
    console.error(problems.join("\n"));
    process.exit(1);
  }
  console.log(JSON.stringify(keys));
}
