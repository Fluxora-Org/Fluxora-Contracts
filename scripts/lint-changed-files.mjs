#!/usr/bin/env node
/**
 * lint-changed-files.mjs
 *
 * Fast local lint: runs `cargo clippy` only on files changed relative to the
 * current merge-base (origin/main by default, or the ref in $LINT_BASE_REF).
 *
 * This is the **developer-local** fast path. It intentionally does NOT lint
 * untouched files, so pre-existing violations that predate this script do
 * not suddenly appear on a developer's screen when they touch an unrelated
 * file. CI enforces the full-repository lint via `lint-full-with-baseline.mjs`.
 *
 * Usage:
 *   node scripts/lint-changed-files.mjs           # vs. origin/main
 *   LINT_BASE_REF=HEAD~5 node scripts/lint-changed-files.mjs  # vs. last 5 commits
 *   node scripts/lint-changed-files.mjs --staged  # only staged changes
 */

import { execFileSync, spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { resolve, relative } from "node:path";

const REPO_ROOT = resolve(import.meta.dirname, "..");
const LINT_BASE_REF = process.env.LINT_BASE_REF || "origin/main";
const ONLY_STAGED = process.argv.includes("--staged");

function run(cmd, args, opts = {}) {
  const result = spawnSync(cmd, args, {
    cwd: REPO_ROOT,
    encoding: "utf-8",
    env: { ...process.env, CARGO_TERM_COLOR: "always" },
    ...opts,
  });
  return result;
}

function changedRustFiles() {
  // --diff-filter=d excludes deleted files (nothing to lint there).
  const diffArgs = ONLY_STAGED
    ? ["diff", "--cached", "--name-only", "--diff-filter=d", "--"]
    : ["diff", "--name-only", `${LINT_BASE_REF}...HEAD`, "--diff-filter=d", "--"];

  const diff = run("git", diffArgs);
  if (diff.status !== 0) {
    console.error("[lint:changed] git diff failed:");
    console.error(diff.stderr);
    process.exit(2);
  }

  const files = diff.stdout
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter((l) => l && (l.endsWith(".rs") || l.endsWith(".toml")));

  return files;
}

function main() {
  if (!existsSync(resolve(REPO_ROOT, "Cargo.toml"))) {
    console.error("[lint:changed] Cargo.toml not found; run from repo root.");
    process.exit(2);
  }

  // 1. Format check (cheap, run it always even if no Rust files changed — a
  //    Cargo.toml-only change should still pass format gates that touch it).
  console.log("[lint:changed] cargo fmt --check");
  const fmt = run("cargo", ["fmt", "--all", "--", "--check"]);
  process.stdout.write(fmt.stdout);
  process.stderr.write(fmt.stderr);
  if (fmt.status !== 0) {
    console.error("\n[lint:changed] FAIL — cargo fmt reported changes needed.");
    console.error("              Run `cargo fmt --all` locally and commit the fix.");
    process.exit(1);
  }
  console.log("[lint:changed] cargo fmt: clean.");

  // 2. If no changed Rust source files, we skip clippy entirely. This is the
  //    whole point of the changed-files mode: a docs-only PR doesn't pay the
  //    60-second clippy startup cost.
  const rustFiles = changedRustFiles();
  const sourceFiles = rustFiles.filter((f) => f.endsWith(".rs"));

  if (sourceFiles.length === 0) {
    console.log(
      "[lint:changed] No changed .rs files detected; skipping clippy " +
        "(run `pnpm lint:full` for whole-repo lint)."
    );
    process.exit(0);
  }

  console.log(
    `[lint:changed] Clippy on changed source (${sourceFiles.length} file(s)):`
  );
  for (const f of sourceFiles) {
    console.log(`              - ${f}`);
  }

  // 3. Scope clippy to the packages that own the changed files. In a single-
  //    package workspace this is just `fluxora-stream`; a modular contract
  //    added later would be picked up automatically.
  //
  //    Because clippy doesn't accept an arbitrary list of file paths at the
  //    CLI level, we target the package(s) containing them. The granular
  //    changed-file filtering still matters *before* this step so a docs-
  //    only change doesn't spin up clippy at all.
  const packagesToLint = new Set();
  for (const f of rustFiles) {
    if (f.startsWith("contracts/stream/")) packagesToLint.add("fluxora-stream");
    if (f.startsWith("contracts/factory/")) packagesToLint.add("fluxora-factory");
    if (f.startsWith("contracts/archival-probe/"))
      packagesToLint.add("archival-probe");
    if (f.startsWith("tools/provenance/")) packagesToLint.add("provenance");
  }
  // Fallback: unrecognised path — lint the whole workspace to be safe.
  const pkgArgs =
    packagesToLint.size > 0
      ? Array.from(packagesToLint).flatMap((p) => ["-p", p])
      : ["--workspace"];

  console.log(
    `[lint:changed] cargo clippy ${pkgArgs.join(" ")} --all-targets ` +
      `--all-features -- -D warnings`
  );
  const clippy = run("cargo", [
    "clippy",
    ...pkgArgs,
    "--all-targets",
    "--all-features",
    "--",
    "-D",
    "warnings",
  ]);
  process.stdout.write(clippy.stdout);
  process.stderr.write(clippy.stderr);

  if (clippy.status !== 0) {
    console.error(
      `\n[lint:changed] FAIL — clippy reported ${clippy.status ? "errors" : "warnings"}.`
    );
    console.error("              Full-repo lint & baseline check: `pnpm lint:full`");
    process.exit(1);
  }

  console.log("[lint:changed] clippy: clean for changed packages.");
  console.log("[lint:changed] OK (changed-files fast-path).");
}

main();
