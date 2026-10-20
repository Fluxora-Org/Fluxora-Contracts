#!/usr/bin/env node
/**
 * lint-full-with-baseline.mjs
 *
 * Full-repository lint used in CI. Runs `cargo fmt --check` plus
 * `cargo clippy --workspace --all-targets --all-features -D warnings`, then
 * diffs any violations against `scripts/lint-baseline.json`.
 *
 * ## Baseline contract
 *
 * The baseline records a set of *fingerprints* for known, pre-existing lint
 * violations. A run is GREEN when:
 *
 *   current_violations ⊆ baseline                 (no NEW violations)
 *
 * A run is RED when the set difference
 *
 *   current_violations ∖ baseline                 (anything NEW)
 *
 * is non-empty. In that case, each new violation is printed with the exact
 * `cargo clippy` diagnostic so the PR author can fix it, and the script
 * exits non-zero.
 *
 * The baseline only ever grows when a maintainer explicitly runs
 *   `pnpm lint:baseline:update`
 * and commits the updated `lint-baseline.json`. Any shrinkage (a violation
 * was fixed and no longer appears) is silently allowed — that's the happy
 * path. This guarantees the backlog *cannot grow* without an explicit
 * commit, while making incremental fixes free.
 *
 * ## Fingerprinting
 *
 * Each clippy diagnostic is reduced to a stable, line-agnostic fingerprint
 * of the form:
 *   <package>::<file>:<linter>::<error_code>
 *
 * Using the error-code (not the message) keeps the baseline stable when a
 * diagnostic's wording changes upstream. Line numbers are deliberately NOT
 * included — otherwise an unrelated edit a few lines above would regenerate
 * the entire baseline for no reason.
 *
 * Format errors have no error code; they fingerprint as:
 *   <file>::fmt::needs-format
 *
 * Usage:
 *   pnpm lint:full                  # CI mode: check, exit non-zero on NEW
 *   pnpm lint:baseline:update       # re-baseline: save ALL current violations
 */

import { spawnSync } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { resolve, relative } from "node:path";

const REPO_ROOT = resolve(import.meta.dirname, "..");
const BASELINE_PATH = resolve(REPO_ROOT, "scripts", "lint-baseline.json");
const UPDATE_BASELINE = process.argv.includes("--update-baseline");

function run(cmd, args, opts = {}) {
  const result = spawnSync(cmd, args, {
    cwd: REPO_ROOT,
    encoding: "utf-8",
    maxBuffer: 16 * 1024 * 1024,
    env: { ...process.env, CARGO_TERM_COLOR: "never" },
    ...opts,
  });
  if (result.error || result.signal || result.status === null) {
    failTool(
      `${cmd} ${args[0]}`,
      result,
      result.error?.message || `terminated by ${result.signal || "an unknown signal"}`
    );
  }
  return result;
}

function failTool(label, result, detail) {
  console.error(`[lint:full] ERROR — ${label}: ${detail}`);
  if (result.stderr) process.stderr.write(result.stderr);
  // Incomplete runs must never pass or replace the committed baseline.
  process.exit(2);
}

// ---- Baseline file helpers ------------------------------------------------

function loadBaseline() {
  if (!existsSync(BASELINE_PATH)) return { fingerprints: [], updated_at: null };
  try {
    const raw = JSON.parse(readFileSync(BASELINE_PATH, "utf-8"));
    if (!Array.isArray(raw.fingerprints)) raw.fingerprints = [];
    return raw;
  } catch (e) {
    console.error(
      `[lint:full] WARNING: lint-baseline.json unparseable (${e.message}); treating as empty.`
    );
    return { fingerprints: [], updated_at: null };
  }
}

function saveBaseline(fingerprints, metadata) {
  const sorted = Array.from(new Set(fingerprints)).sort();
  const payload = {
    updated_at: new Date().toISOString(),
    ...metadata,
    fingerprint_count: sorted.length,
    fingerprints: sorted,
  };
  writeFileSync(
    BASELINE_PATH,
    JSON.stringify(payload, null, 2) + "\n",
    "utf-8"
  );
  console.log(
    `[lint:full] Baseline saved: ${sorted.length} fingerprint(s) → ` +
      relative(REPO_ROOT, BASELINE_PATH)
  );
}

// ---- cargo fmt capture -----------------------------------------------------
//
// `cargo fmt --check` exits 1 on mismatch and writes file-by-file diffs to
// stdout. We parse the "Diff in ..." lines to fingerprint each file that
// needs formatting.

function parseFmtViolations(stdout) {
  const fps = [];
  const re = /^Diff in (.+):(\d+):\r?$/gm;
  let m;
  while ((m = re.exec(stdout)) !== null) {
    const file = relative(REPO_ROOT, m[1].trim()).replace(/\\/g, "/");
    fps.push({
      fingerprint: `${file}::fmt::needs-format`,
      source: "fmt",
      file,
      detail: "needs `cargo fmt`",
    });
  }
  return fps;
}

// ---- clippy diagnostic capture --------------------------------------------
//
// Clippy's human-readable headers often omit the lint code entirely. Cargo's
// JSON stream preserves it (including names such as clippy::needless_return)
// and includes build-finished so incomplete output cannot look like a pass.

function parseClippyDiagnostics(stdout) {
  const fps = [];
  let finished;
  for (const line of stdout.split(/\r?\n/).filter((line) => line.trim())) {
    const entry = JSON.parse(line);
    if (entry.reason === "build-finished") {
      finished = entry.success;
    }
    if (entry.reason !== "compiler-message") continue;
    const diagnostic = entry.message;
    if (!["error", "warning"].includes(diagnostic.level)) continue;
    const span = diagnostic.spans.find((span) => span.is_primary);
    const code = diagnostic.code?.code;
    if (!code || !span?.file_name) {
      throw new Error(diagnostic.rendered || diagnostic.message);
    }
    fps.push({
      source: "clippy",
      severity: diagnostic.level,
      code,
      file: relative(REPO_ROOT, resolve(REPO_ROOT, span.file_name)).replace(/\\/g, "/"),
      detail: diagnostic.rendered || diagnostic.message,
    });
  }
  if (typeof finished !== "boolean") throw new Error("missing build-finished diagnostic");

  const violations = fps.map((d) => {
    // Package = first two path segments for workspace packages, else "root".
    let pkg = "workspace-root";
    if (d.file.startsWith("contracts/stream/")) pkg = "fluxora-stream";
    else if (d.file.startsWith("contracts/factory/")) pkg = "fluxora-factory";
    else if (d.file.startsWith("contracts/archival-probe/"))
      pkg = "archival-probe";
    else if (d.file.startsWith("tools/provenance/")) pkg = "provenance";

    const fingerprint = `${pkg}::${d.file}::${d.source}::${d.code || "no-code"}`;
    return { ...d, fingerprint };
  });
  return { violations, finished };
}

// ---- Main ------------------------------------------------------------------

function main() {
  console.log(
    "[lint:full] " +
      (UPDATE_BASELINE
        ? "BASELINE-UPDATE MODE — saving all current violations."
        : "CI MODE — failing only on NEW violations vs. baseline.")
  );

  const baseline = loadBaseline();
  const baselineSet = new Set(baseline.fingerprints);
  console.log(
    `[lint:full] Baseline loaded: ${baselineSet.size} known fingerprint(s).`
  );

  // 1. cargo fmt
  console.log("\n[lint:full] Step 1/2: cargo fmt --all --check");
  const fmt = run("cargo", ["fmt", "--all", "--", "--check"]);
  const fmtViolations = parseFmtViolations(fmt.stdout);
  if (
    fmt.status !== 0 &&
    (fmt.status !== 1 || fmtViolations.length === 0 || /^error\b/m.test(fmt.stderr))
  ) {
    failTool("cargo fmt", fmt, `exited ${fmt.status} without a complete formatting result`);
  }

  // 2. cargo clippy — workspace-wide, all targets, all features, hard fail
  console.log(
    "[lint:full] Step 2/2: cargo clippy --workspace --all-targets --all-features --message-format=json -- -D warnings"
  );
  const clippy = run("cargo", [
    "clippy",
    "--workspace",
    "--all-targets",
    "--all-features",
    "--message-format=json",
    "--",
    "-D",
    "warnings",
  ]);
  let clippyResult;
  try {
    clippyResult = parseClippyDiagnostics(clippy.stdout);
  } catch (error) {
    failTool("cargo clippy", clippy, error.message);
  }
  const clippyViolations = clippyResult.violations;
  if (
    clippyResult.finished !== (clippy.status === 0) ||
    (clippy.status !== 0 && !clippyViolations.some((v) => v.severity === "error"))
  ) {
    failTool("cargo clippy", clippy, `exited ${clippy.status} without a complete diagnostic result`);
  }

  // Aggregate
  const allViolations = [...fmtViolations, ...clippyViolations];
  const allFps = Array.from(new Set(allViolations.map((v) => v.fingerprint)));

  if (UPDATE_BASELINE) {
    saveBaseline(allFps, {
      mode: "manual-update",
      fmt_count: fmtViolations.length,
      clippy_count: clippyViolations.length,
    });
    if (allFps.length === 0) {
      console.log("[lint:full] ✓ No violations detected; baseline is empty/clean.");
    } else {
      console.log(
        `[lint:full] ⚠ Baseline recorded ${allFps.length} violation(s). ` +
          `Review the diff before committing.`
      );
    }
    process.exit(0);
  }

  // CI mode: compare against baseline.
  const newOnes = allFps.filter((fp) => !baselineSet.has(fp));
  const fixedOnes = Array.from(baselineSet).filter(
    (fp) => !allFps.includes(fp)
  );

  console.log(
    `\n[lint:full] Summary: ${allFps.length} current, ` +
      `${baselineSet.size} baseline, ` +
      `${newOnes.length} NEW, ` +
      `${fixedOnes.length} fixed.`
  );

  if (fixedOnes.length > 0) {
    console.log(
      `\n[lint:full] 🎉 ${fixedOnes.length} baseline violation(s) fixed — great!`
    );
    for (const fp of fixedOnes) console.log(`              - ${fp}`);
    console.log(
      "              (run `pnpm lint:baseline:update` to shrink the baseline.)"
    );
  }

  if (newOnes.length === 0) {
    console.log("\n[lint:full] ✓ PASS — no new lint violations introduced.");
    if (allFps.length === 0) {
      console.log("[lint:full]   Repository is perfectly clean.");
    } else {
      console.log(
        `[lint:full]   All ${allFps.length} current violation(s) are in the baseline.`
      );
    }
    process.exit(0);
  }

  // Print every NEW violation with its human-readable detail.
  console.error(
    `\n[lint:full] ✗ FAIL — ${newOnes.length} NEW lint violation(s) detected ` +
      `(not in ${relative(REPO_ROOT, BASELINE_PATH)}):`
  );
  for (const fp of newOnes) {
    const v = allViolations.find((x) => x.fingerprint === fp);
    console.error(`\n  === NEW: ${fp} ===`);
    if (v) {
      if (v.file) console.error(`  file    : ${v.file}`);
      if (v.code) console.error(`  code    : ${v.code}`);
      if (v.detail) console.error(`  detail  : ${v.detail}`);
    }
  }

  console.error(
    "\n  Suggested fixes:\n" +
      "    • Address each violation listed above.\n" +
      "    • If any are intentional false-positives, apply `#[allow(...)]` locally\n" +
      "      and verify the baseline fingerprint no longer matches.\n" +
      "    • If you need to *grow* the baseline (rare — a new allowlisted legacy\n" +
      "      pattern), ask a maintainer to run `pnpm lint:baseline:update`\n" +
      "      and commit the updated scripts/lint-baseline.json alongside your PR."
  );
  process.exit(1);
}

main();
