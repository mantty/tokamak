#!/usr/bin/env node
// Runs the checks and writes a JSON report for each check kind to reports/.
// Usage: node scripts/collect-report.mts [lint|format|typecheck|test|all]
// Exits 1 when a section fails; the log names each finding.
import { spawnSync, type SpawnSyncReturns } from "node:child_process";
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import path from "node:path";

const PROJECT = "tokamak";
const LANGUAGE = "rust";
const REPORTS = path.resolve("reports");
const JSCPD_REPORT = path.join(REPORTS, "jscpd-report.json");

// The host builds the lint report covers: the workspace, and the runtime with
// its native modules.
/** A check section's result. */
export interface Section {
  status: "passed" | "failed";
  errors: number;
  warnings: number;
}

/** The report of one check kind, as reports/<project>-<kind>.json holds it. */
export interface Report {
  schema_version: 1;
  project: string;
  language: string;
  kind: string;
  sections: Record<string, Section>;
}

/** The parts of a compiler diagnostic in cargo's JSON output that the lint report reads. */
interface Diagnostic {
  level: string;
  code: { code: string } | null;
  spans: unknown[];
  rendered: string;
}

/** A block jscpd found twice, as its JSON report records it. */
interface Duplicate {
  firstFile: { name: string; start: number; end: number };
  secondFile: { name: string; start: number; end: number };
}

const CLIPPY_TARGETS = [
  ["--workspace", "--exclude", "windows-shell", "--all-targets"],
  ["-p", "tokamak", "--all-targets", "--features", "native"],
];

// The lint sections with their own clippy lints; every other finding counts
// in `lint`.
const LINT_SECTIONS = new Map<string | undefined, string>([
  ["clippy::cognitive_complexity", "cognitive_complexity"],
  ["clippy::too_many_lines", "function_length"],
  ["clippy::excessive_nesting", "nesting"],
  ["clippy::fn_params_excessive_bools", "pedantic-checks"],
  ["clippy::too_many_arguments", "pedantic-checks"],
  ["clippy::type_complexity", "pedantic-checks"],
]);

function run(command: string, args: string[]): SpawnSyncReturns<string> {
  const result = spawnSync(command, args, { encoding: "utf8", maxBuffer: 256 * 1024 * 1024 });
  if (result.error) throw result.error;
  return result;
}

function section(failed: boolean, errors = 0, warnings = 0): Section {
  return { status: failed ? "failed" : "passed", errors, warnings };
}

function write(kind: string, sections: Record<string, Section>): void {
  const report: Report = { schema_version: 1, project: PROJECT, language: LANGUAGE, kind, sections };
  writeFileSync(path.join(REPORTS, `${PROJECT}-${kind}.json`), JSON.stringify(report, null, 2));
  const failed = Object.values(sections).some((result) => result.status === "failed");
  console.error(`report ${kind}: ${failed ? "FAILED" : "passed"}`);
  if (failed) process.exitCode = 1;
}

/** The last lines of a command's output, for a log that explains a failure. */
function tail(result: SpawnSyncReturns<string>): string {
  const lines = `${result.stdout}${result.stderr}`.split("\n").filter((line) => line.trim() !== "");
  return lines.slice(-40).join("\n");
}

/** The compiler diagnostic a line of cargo's JSON output carries, if any. */
function diagnostic(line: string): Diagnostic | null {
  try {
    const message = JSON.parse(line);
    return message.reason === "compiler-message" ? message.message : null;
  } catch {
    return null;
  }
}

function lint() {
  const counts = Object.fromEntries(
    ["lint", ...new Set(LINT_SECTIONS.values())].map((name) => [name, { errors: 0, warnings: 0 }]),
  );
  for (const targets of CLIPPY_TARGETS) {
    const result = run("cargo", ["clippy", ...targets, "--message-format=json", "--", "-D", "warnings"]);
    let errors = 0;
    for (const message of result.stdout.split("\n").map(diagnostic)) {
      // Summaries such as "aborting due to 2 previous errors" point at no code.
      if (!message?.spans.length) continue;
      const count = counts[LINT_SECTIONS.get(message.code?.code) ?? "lint"];
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
  let duplicates: Duplicate[];
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

const checks = new Map([["lint", lint], ["format", format], ["typecheck", typecheck], ["test", test]]);
const kind = process.argv[2] ?? "all";
mkdirSync(REPORTS, { recursive: true });
for (const name of kind === "all" ? checks.keys() : [kind]) {
  const check = checks.get(name);
  if (!check) throw new Error("usage: node scripts/collect-report.mts [lint|format|typecheck|test|all]");
  check();
}
