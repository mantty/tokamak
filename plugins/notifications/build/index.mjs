// The notifications plugin's build script: tok runs it for each platform with the plugin's
// resolved build settings on stdin, and it prints the keys to add to the plugin's section.
import { readFileSync } from "node:fs";
import { bundledSounds } from "./sounds.mjs";

const { platform, settings } = JSON.parse(readFileSync(0, "utf8"));
if (settings.sounds === undefined) {
  console.log("{}");
} else {
  const { keys, problems } = bundledSounds(platform, settings.sounds);
  if (problems) {
    console.error(problems.join("\n"));
    process.exit(1);
  }
  console.log(JSON.stringify(keys));
}
