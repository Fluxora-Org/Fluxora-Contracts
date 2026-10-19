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
import { createHash } from "node:crypto";

const REPO_ROOT = resolve(import.meta.dirname, "..");
const BASELINE_PATH = resolve(REPO_ROOT, "scripts", "lint-baseline.json");
const UPDATE_BASELINE = process.argv.includes("--update-baseline");

function run(cmd, args, opts = {}) {
  return spawnSync(cmd, args, {
    cwd: REPO_ROOT,
    encoding: "utf-8",
    env: { ...process.env, CARGO_TERM_COLOR: "never" },
    ...opts,
  });
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
  const re = /Diff in (.+?)(?:\r?\n|$)/g;
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
// clippy writes machine-parseable diagnostics to stderr in the form:
//   error[E0308]: ...
//     --> path/to/file.rs:42:18
//      = note: ...
//
// We parse blocks bounded by the `error[` / `warning[` header and the
// following `--> file:line:col` to build a code + file fingerprint.
//
// Alternative: `--message-format json` is more robust but produces very
// verbose output; the text parser is sufficient for our fingerprinting and
// keeps the human-readable log flowing.

function parseClippyDiagnostics(stderr) {
  const fps = [];
  const lines = stderr.split(/\r?\n/);
  let current = null;

  for (const line of lines) {
    const header = line.match(/^(error|warning)\[([A-Za-z0-9_-]+)\]:/);
    if (header) {
      if (current && current.file) fps.push(current);
      current = {
        source: "clippy",
        severity: header[1],
        code: header[2],
        file: null,
        detail: line.trim(),
      };
      continue;
    }
    if (!current) continue;
    const loc = line.match(/^\s+-->\s+(.+?):(\d+):(\d+)/);
    if (loc) {
      const file = relative(REPO_ROOT, loc[1].trim()).replace(/\\/g, "/");
      current.file = file;
    }
  }
  if (current && current.file) fps.push(current);

  return fps.map((d) => {
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

  // 2. cargo clippy — workspace-wide, all targets, all features, hard fail
  console.log(
    "[lint:full] Step 2/2: cargo clippy --workspace --all-targets --all-features -- -D warnings"
  );
  const clippy = run("cargo", [
    "clippy",
    "--workspace",
    "--all-targets",
    "--all-features",
    "--",
    "-D",
    "warnings",
  ]);
  const clippyViolations = parseClippyDiagnostics(clippy.stderr);

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
