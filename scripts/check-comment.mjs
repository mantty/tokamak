#!/usr/bin/env node
// Builds a concise PR comment comparing reports/ against main's committed reports.
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

function summarize(report) {
  if (!report) return `${dash}`;
  const parts = Object.entries(report.sections ?? {}).map(([name, s]) => {
    const n = (s.errors ?? 0) + (s.warnings ?? 0);
    return `${name}: ${s.status}${n ? ` (${s.errors ?? 0}e/${s.warnings ?? 0}w)` : ""}`;
  });
  const failed = Object.values(report.sections ?? {}).some((s) => s.status === "failed");
  return `${failed ? "\u274c failed" : "\u2705 passed"} \u2014 ${parts.join("; ")}`;
}

let anyFailed = false;
const rows = kinds.map((kind) => {
  const current = reportFor(kind, false);
  const main = reportFor(kind, true);
  if (current && Object.values(current.sections ?? {}).some((s) => s.status === "failed")) anyFailed = true;
  return `| ${kind} | ${summarize(main)} | ${summarize(current)} |`;
});

const body = [
  "<!-- check-results -->",
  `## Check results \u2014 ${project}`,
  "",
  `Checks ${anyFailed ? "**failed**" : "passed"} against the current PR head.`,
  "",
  "| report | main | this PR |",
  "|---|---|---|",
  ...rows,
  "",
  "<!-- end-check-results -->",
  "",
].join("\n");
writeFileSync(".check-comment.md", body);
console.error(body);
process.exit(0);
