#!/usr/bin/env node
// Runs the checks and writes a JSON report for each check kind to reports/.
// Usage: node scripts/collect-report.mjs [lint|format|typecheck|test|all]
// Exits 1 when a section fails; the log names each finding.
import { spawnSync } from "node:child_process";
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import path from "node:path";

const PROJECT = "tokamak";
const LANGUAGE = "rust";
const REPORTS = path.resolve("reports");
const JSCPD_REPORT = path.join(REPORTS, "jscpd-report.json");

// The host builds the lint report covers: the workspace, and the runtime with
// its native modules.
const CLIPPY_TARGETS = [
  ["--workspace", "--exclude", "windows-shell", "--all-targets"],
  ["-p", "tokamak", "--all-targets", "--features", "native"],
];

// The lint sections with their own clippy lints; every other finding counts
// in `lint`.
const LINT_SECTIONS = {
  "clippy::cognitive_complexity": "cognitive_complexity",
  "clippy::too_many_lines": "function_length",
  "clippy::excessive_nesting": "nesting",
  "clippy::fn_params_excessive_bools": "pedantic-checks",
  "clippy::too_many_arguments": "pedantic-checks",
  "clippy::type_complexity": "pedantic-checks",
};

function run(command, args) {
  const result = spawnSync(command, args, { encoding: "utf8", maxBuffer: 256 * 1024 * 1024 });
  if (result.error) throw result.error;
  return result;
}

function section(failed, errors = 0, warnings = 0) {
  return { status: failed ? "failed" : "passed", errors, warnings };
}

function write(kind, sections) {
  const report = { schema_version: 1, project: PROJECT, language: LANGUAGE, kind, sections };
  writeFileSync(path.join(REPORTS, `${PROJECT}-${kind}.json`), JSON.stringify(report, null, 2));
  const failed = Object.values(sections).some((result) => result.status === "failed");
  console.error(`report ${kind}: ${failed ? "FAILED" : "passed"}`);
  if (failed) process.exitCode = 1;
}

/** The last lines of a command's output, for a log that explains a failure. */
function tail(result) {
  const lines = `${result.stdout}${result.stderr}`.split("\n").filter((line) => line.trim() !== "");
  return lines.slice(-40).join("\n");
}

/** The compiler diagnostic a line of cargo's JSON output carries, if any. */
function diagnostic(line) {
  try {
    const message = JSON.parse(line);
    return message.reason === "compiler-message" ? message.message : null;
  } catch {
    return null;
  }
}

function lint() {
  const counts = Object.fromEntries(
    ["lint", ...new Set(Object.values(LINT_SECTIONS))].map((name) => [name, { errors: 0, warnings: 0 }]),
  );
  for (const targets of CLIPPY_TARGETS) {
    const result = run("cargo", ["clippy", ...targets, "--message-format=json", "--", "-D", "warnings"]);
    let errors = 0;
    for (const message of result.stdout.split("\n").map(diagnostic)) {
      // Summaries such as "aborting due to 2 previous errors" point at no code.
      if (!message?.spans.length) continue;
      const count = counts[LINT_SECTIONS[message.code?.code] ?? "lint"];
      if (message.level === "error") {
        count.errors++;
        errors++;
      } else {
        count.warnings++;
      }
      console.error(message.rendered.trimEnd());
    }
    if (result.status !== 0 && errors === 0) {
      counts.lint.errors++;
      console.error(`cargo clippy ${targets.join(" ")} exited ${result.status ?? result.signal}:\n${tail(result)}`);
    }
  }
  const sections = Object.fromEntries(
    Object.entries(counts).map(([name, count]) => [name, section(count.errors > 0, count.errors, count.warnings)]),
  );
  write("lint", { ...sections, duplication: duplication() });
}

/** The duplication section: any block jscpd finds twice fails it. */
function duplication() {
  rmSync(JSCPD_REPORT, { force: true });
  const result = run("jscpd", [".", "--config", "jscpd.json"]);
  let duplicates;
  try {
    duplicates = JSON.parse(readFileSync(JSCPD_REPORT, "utf8")).duplicates;
  } catch {
    console.error(`jscpd wrote no report; it exited ${result.status ?? result.signal}:\n${tail(result)}`);
    return section(true, 1);
  }
  for (const { firstFile: first, secondFile: second } of duplicates) {
    console.error(`duplicate: ${first.name}:${first.start}-${first.end} and ${second.name}:${second.start}-${second.end}`);
  }
  return section(duplicates.length > 0, duplicates.length);
}

function format() {
  const result = run("cargo", ["fmt", "--all", "--check"]);
  const failed = result.status !== 0;
  if (failed) console.error(tail(result));
  const files = result.stdout.match(/^Diff in/gm)?.length ?? 0;
  write("format", { format: section(failed, failed ? Math.max(files, 1) : 0) });
}

function typecheck() {
  const result = run("cargo", ["check", "--workspace", "--exclude", "windows-shell"]);
  const failed = result.status !== 0;
  if (failed) console.error(tail(result));
  write("typecheck", { typecheck: section(failed, failed ? 1 : 0) });
}

function test() {
  const result = run("cargo", ["test", "--workspace", "--exclude", "windows-shell"]);
  const failed = result.status !== 0;
  if (failed) console.error(`cargo test exited ${result.status ?? result.signal}:\n${tail(result)}`);
  const failures = [...`${result.stdout}${result.stderr}`.matchAll(/(\d+) failed/g)]
    .reduce((total, match) => total + Number(match[1]), 0);
  write("test", { test: section(failed, failed ? Math.max(failures, 1) : 0) });
}

const checks = { lint, format, typecheck, test };
const kind = process.argv[2] ?? "all";
mkdirSync(REPORTS, { recursive: true });
for (const name of kind === "all" ? Object.keys(checks) : [kind]) checks[name]();
