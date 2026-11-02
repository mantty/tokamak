#!/usr/bin/env node
// Validates that every required report exists and contains its required sections.
// Only structural validation; the underlying tools enforce their own rules.
import { existsSync, readFileSync } from "node:fs";
import path from "node:path";
import type { Report } from "./collect-report.mts";

/** checks.config.json: the sections each project's reports must contain, by check kind. */
interface ChecksConfig {
  projects: Record<string, { required_sections: Record<string, string[]> }>;
}

const config: ChecksConfig = JSON.parse(readFileSync("checks.config.json", "utf8"));
const project = process.argv[2] ?? Object.keys(config.projects)[0];
const required = config.projects[project].required_sections;
let bad = false;
for (const [kind, sections] of Object.entries(required)) {
  const file = path.join("reports", `${project}-${kind}.json`);
  if (!existsSync(file)) {
    console.error(`missing report: ${file}`);
    bad = true;
    continue;
  }
  // The report may lack any part, which is what this script checks.
  const report: Partial<Report> = JSON.parse(readFileSync(file, "utf8"));
  if (report.schema_version !== 1) {
    console.error(`${file}: schema_version must be 1`);
    bad = true;
  }
  if (report.kind !== kind) {
    console.error(`${file}: kind must be ${kind}`);
    bad = true;
  }
  for (const s of sections) {
    const sec = report.sections?.[s];
    if (!sec || !["passed", "failed"].includes(sec.status)) {
      console.error(`${file}: missing section "${s}" with status passed|failed`);
      bad = true;
    }
  }
}
for (const kind of ["lint", "format", "typecheck", "test"]) {
  const file = path.join("reports", `${project}-${kind}.json`);
  if (!required[kind] && existsSync(file)) {
    console.error(`${file}: no required_sections entry for kind "${kind}" in checks.config.json`);
    bad = true;
  }
}
process.exit(bad ? 1 : 0);
