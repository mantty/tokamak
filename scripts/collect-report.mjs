#!/usr/bin/env node
// Generates JSON quality reports in reports/ for each check kind.
// Usage: node scripts/collect-report.mjs [lint|format|typecheck|test|all]
import { spawnSync } from "node:child_process";
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import path from "node:path";

const PROJECT = "tokamak";
const LANGUAGE = "rust";
const REPORTS = path.resolve("reports");
const kind = process.argv[2] ?? "all";
mkdirSync(REPORTS, { recursive: true });

const CARGO_LINT_ARGS = [
  "clippy", "--workspace", "--exclude", "windows-shell", "--all-targets", "--message-format=json", "--",
  "-D", "warnings",
  // Supplementary pedantic lints not enabled by default:
  "-D", "clippy::excessive_nesting",
  "-D", "clippy::fn_params_excessive_bools",
  "-D", "clippy::type_complexity",
  "-D", "clippy::too_many_arguments",
];

function run(cmd, args) {
  const parts = cmd.split(" ");
  return spawnSync(parts[0], [...parts.slice(1), ...(args ?? [])], { encoding: "utf8", maxBuffer: 1024 * 1024 * 64 });
}

function write(kindName, sections) {
  const report = {
    schema_version: 1,
    project: PROJECT,
    language: LANGUAGE,
    kind: kindName,
    sections,
  };
  writeFileSync(path.join(REPORTS, `${PROJECT}-${kindName}.json`), JSON.stringify(report, null, 2));
  const failed = Object.values(sections).some((s) => s.status === "failed");
  console.error(`report ${kindName}: ${failed ? "FAILED" : "passed"}`);
  if (failed) process.exitCode = 1;
}

function section(status, errors = 0, warnings = 0) {
  return { status, errors, warnings };
}

function lint() {
  const buckets = { lint: { e: 0, w: 0 }, cognitive_complexity: { e: 0, w: 0 }, function_length: { e: 0, w: 0 }, nesting: { e: 0, w: 0 }, "pedantic-checks": { e: 0, w: 0 } };
  const r = run("cargo", CARGO_LINT_ARGS);
  for (const line of (r.stdout ?? "").split("\n")) {
    if (!line.trim()) continue;
    let msg;
    try {
      msg = JSON.parse(line);
    } catch {
      continue;
    }
    const code = msg?.message?.code?.code ?? "";
    if (!code.startsWith("clippy::")) continue;
    const isErr = msg.message.level === "error";
    const key = code === "clippy::cognitive_complexity"
      ? "cognitive_complexity"
      : code === "clippy::too_many_lines"
        ? "function_length"
        : code === "clippy::excessive_nesting"
          ? "nesting"
          : code === "clippy::too_many_arguments" || code === "clippy::fn_params_excessive_bools" || code === "clippy::type_complexity"
            ? "pedantic-checks"
            : "lint";
    if (isErr) buckets[key].e++;
    else buckets[key].w++;
  }
  rmSync(path.join(REPORTS, "jscpd.json"), { force: true });
  const j = run("jscpd", [".", "--config", "jscpd.json"]);
  let clones = 0, warns = 0, fails = 0;
  try {
    const stats = JSON.parse(readFileSync(path.join(REPORTS, "jscpd-report.json"), "utf8")).statistics.total;
    clones = stats.clones ?? 0;
    const dup = stats.percentage ?? 0;
    if (dup > 90) fails = 1;
    else if (dup > 80) warns = 1;
  } catch {
    if ((j.status ?? 1) !== 0) fails = 1;
  }
  if (clones > 0) fails = 1; // exact duplicates fail
  write("lint", {
    lint: section(buckets.lint.e ? "failed" : "passed", buckets.lint.e, buckets.lint.w),
    cognitive_complexity: section(buckets.cognitive_complexity.e ? "failed" : "passed", buckets.cognitive_complexity.e, buckets.cognitive_complexity.w),
    function_length: section(buckets.function_length.e ? "failed" : "passed", buckets.function_length.e, buckets.function_length.w),
    nesting: section(buckets.nesting.e ? "failed" : "passed", buckets.nesting.e, buckets.nesting.w),
    "pedantic-checks": section(buckets["pedantic-checks"].e ? "failed" : "passed", buckets["pedantic-checks"].e, buckets["pedantic-checks"].w),
    duplication: section(fails ? "failed" : "passed", fails, warns),
  });
}

function format() {
  const r = run("cargo", ["fmt", "--all", "--check"]);
  write("format", { format: section(r.status === 0 ? "passed" : "failed", (r.stdout ?? "").match(/^Diff in/gm)?.length ?? 0, 0) });
}

function typecheck() {
  const r = run("cargo", ["check", "--workspace", "--exclude", "windows-shell"]);
  write("typecheck", { typecheck: section(r.status === 0 ? "passed" : "failed", 0, 0) });
}

function test() {
  const r = run("cargo", ["test", "--workspace", "--exclude", "windows-shell"]);
  const out = (r.stdout ?? "") + (r.stderr ?? "");
  const fails = [...out.matchAll(/(\d+) failed/g)].reduce((a, m) => a + Number(m[1]), 0);
  if (r.status !== 0) {
    // The report only counts failures; the log keeps why they failed.
    const lines = out.split("\n").filter((line) => /^error|FAILED|panicked|failures:/.test(line));
    console.error(lines.slice(0, 40).join("\n"));
  }
  write("test", { test: section(r.status === 0 ? "passed" : "failed", fails, 0) });
}

const kinds = kind === "all" ? ["lint", "format", "typecheck", "test"] : [kind];
for (const k of kinds) ({ lint, format, typecheck, test })[k]();
