#!/usr/bin/env node
// Prints the PR comment that compares each check section in reports/ with
// main's reports, which may be missing.
// Usage: node scripts/check-comment.mjs <project> <directory of main's reports>
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import path from "node:path";

const [project, mainReports] = process.argv.slice(2);
const kinds = ["lint", "format", "typecheck", "test"];
const dash = "—";

function report(directory, kind) {
  try {
    return JSON.parse(readFileSync(path.join(directory, `${project}-${kind}.json`), "utf8"));
  } catch {
    return null;
  }
}

function cell(section) {
  if (!section) return dash;
  const detail = section.errors || section.warnings ? ` (${section.errors}e/${section.warnings}w)` : "";
  return section.status === "failed" ? `❌ failed${detail}` : `✅ passed${detail}`;
}

let failed = false;
const rows = [];
for (const kind of kinds) {
  const current = report("reports", kind);
  if (!current) {
    failed = true;
    rows.push(`| ${kind} | | ${dash} | missing |`);
    continue;
  }
  const main = report(mainReports, kind);
  for (const [name, section] of Object.entries(current.sections)) {
    if (section.status === "failed") failed = true;
    rows.push(`| ${kind} | ${name} | ${cell(main?.sections[name])} | ${cell(section)} |`);
  }
}

const head = execFileSync("git", ["rev-parse", "--short", "HEAD"], { encoding: "utf8" }).trim();
console.log([
  "<!-- check-results -->",
  `## Check results — ${project}`,
  "",
  `Checks ${failed ? "**failed**" : "passed"} at \`${head}\`.`,
  "",
  "| report | section | main | this PR |",
  "|---|---|---|---|",
  ...rows,
].join("\n"));
