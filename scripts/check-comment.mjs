#!/usr/bin/env node
// Builds a single PR comment from reports/: each check section's result,
// compared against main's committed reports.
// Usage: node scripts/check-comment.mjs <project>
// Writes the comment to .check-comment.md and exits 0; comment content states failures.
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync, existsSync } from "node:fs";

const project = process.argv[2];
const kinds = ["lint", "format", "typecheck", "test"];
const dash = "\u2014";

function reportFor(kind, fromGit) {
  const file = `reports/${project}-${kind}.json`;
  try {
    if (fromGit) {
      return JSON.parse(execFileSync("git", ["show", `origin/main:${file}`], { maxBuffer: 1024 * 1024 * 8 }).toString());
    }
    if (existsSync(file)) return JSON.parse(readFileSync(file, "utf8"));
  } catch {
    return null;
  }
  return null;
}

function cell(section) {
  if (!section) return dash;
  const errors = section.errors ?? 0;
  const warnings = section.warnings ?? 0;
  const detail = errors || warnings ? ` (${errors}e/${warnings}w)` : "";
  return section.status === "failed" ? `\u274c failed${detail}` : `\u2705 passed${detail}`;
}

let anyFailed = false;
const rows = [];
for (const kind of kinds) {
  const current = reportFor(kind, false);
  if (!current || !Object.keys(current.sections ?? {}).length) {
    rows.push(`| ${kind} | | ${dash} | missing |`);
    continue;
  }
  const main = reportFor(kind, true);
  for (const [name, section] of Object.entries(current.sections)) {
    if (section.status === "failed") anyFailed = true;
    rows.push(`| ${kind} | ${name} | ${cell(main?.sections?.[name])} | ${cell(section)} |`);
  }
}

const head = execFileSync("git", ["rev-parse", "--short", "HEAD"]).toString().trim();
const body = [
  "<!-- check-results -->",
  `## Check results \u2014 ${project}`,
  "",
  `Checks ${anyFailed ? "**failed**" : "passed"} at \`${head}\`.`,
  "",
  "| report | section | main | this PR |",
  "|---|---|---|---|",
  ...rows,
  "",
  "<!-- end-check-results -->",
  "",
].join("\n");
writeFileSync(".check-comment.md", body);
console.error(body);
process.exit(0);
