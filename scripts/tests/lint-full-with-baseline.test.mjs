import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import test from "node:test";

const script = new URL("../lint-full-with-baseline.mjs", import.meta.url);
const finished = (success) => JSON.stringify({ reason: "build-finished", success }) + "\n";
const lint = (code = "clippy::needless_return") => JSON.stringify({
  reason: "compiler-message",
  message: {
    level: "error",
    code: code ? { code } : null,
    message: "unneeded return statement",
    rendered: "error: unneeded return statement\n",
    spans: [{ file_name: "contracts/stream/src/lib.rs", is_primary: true }],
  },
}) + "\n";
const fingerprint = "fluxora-stream::contracts/stream/src/lib.rs::clippy::clippy::needless_return";

function run(t, results = {}, fingerprints = [], args = []) {
  const root = mkdtempSync(join(tmpdir(), "fluxora-lint-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  mkdirSync(join(root, "scripts"));
  copyFileSync(script, join(root, "scripts", "lint-full-with-baseline.mjs"));
  const baseline = join(root, "scripts", "lint-baseline.json");
  const original = JSON.stringify({ fingerprints });
  writeFileSync(baseline, original);
  const preload = join(root, "cargo-fixture.mjs");
  writeFileSync(preload, `
    import childProcess from "node:child_process";
    import { syncBuiltinESMExports } from "node:module";
    const results = JSON.parse(process.env.FLUXORA_TEST_RESULTS);
    childProcess.spawnSync = (cmd, args) => {
      if (cmd !== "cargo") throw new Error("unexpected command: " + cmd);
      const result = { ...results[args[0]] };
      if (result.error) result.error = new Error(result.error);
      return result;
    };
    syncBuiltinESMExports();
  `);
  const fixtures = {
    fmt: { status: 0, stdout: "", stderr: "", ...results.fmt },
    clippy: { status: 0, stdout: finished(true), stderr: "", ...results.clippy },
  };
  // rustfmt emits an absolute path; use a placeholder to keep fixtures portable.
  fixtures.fmt.stdout = fixtures.fmt.stdout.replaceAll("<ROOT>", root);
  const result = spawnSync(process.execPath, [
    "--import", pathToFileURL(preload).href,
    join(root, "scripts", "lint-full-with-baseline.mjs"), ...args,
  ], {
    cwd: tmpdir(),
    env: { ...process.env, FLUXORA_TEST_RESULTS: JSON.stringify(fixtures) },
    encoding: "utf8",
  });
  assert.ifError(result.error);
  return { ...result, baseline: readFileSync(baseline, "utf8"), original };
}

test("clean tools pass", (t) => {
  assert.equal(run(t).status, 0);
});

test("a failed fmt invocation is not a clean repository", (t) => {
  const result = run(t, { fmt: { status: 1, stderr: "error: invalid manifest\n" } });
  assert.equal(result.status, 2);
  assert.match(result.stderr, /invalid manifest/);
  assert.doesNotMatch(result.stdout, /PASS/);
});

test("dependency resolution failure is not a clean clippy result", (t) => {
  const result = run(t, { clippy: {
    status: 101, stdout: finished(false), stderr: "error: failed to select a version\n",
  } });
  assert.equal(result.status, 2);
  assert.match(result.stderr, /failed to select a version/);
});

test("spawn errors and signals produce an actionable failure", (t) => {
  for (const failure of [
    { status: null, error: "spawn cargo ENOENT" },
    { status: null, signal: "SIGTERM" },
  ]) {
    const result = run(t, { clippy: failure });
    assert.equal(result.status, 2);
    assert.match(result.stderr, /ENOENT|SIGTERM/);
  }
});

test("JSON clippy diagnostics with namespaced codes fail when new", (t) => {
  const result = run(t, { clippy: { status: 101, stdout: lint() + finished(false) } });
  assert.equal(result.status, 1);
  assert.match(result.stderr, /clippy::needless_return/);
});

test("known clippy diagnostics retain the baseline contract", (t) => {
  const result = run(t, { clippy: { status: 101, stdout: lint() + finished(false) } }, [fingerprint]);
  assert.equal(result.status, 0);
  assert.match(result.stdout, /1 current, 1 baseline, 0 NEW/);
});

test("an uncoded compiler error cannot hide behind a known lint", (t) => {
  const result = run(t, { clippy: {
    status: 101, stdout: lint() + lint(null) + finished(false),
  } }, [fingerprint]);
  assert.equal(result.status, 2);
  assert.match(result.stderr, /unneeded return statement/);
});

test("truncated or invalid diagnostic output fails", (t) => {
  for (const stdout of [lint(), "not JSON\n"]) {
    const result = run(t, { clippy: { status: 101, stdout } }, [fingerprint]);
    assert.equal(result.status, 2);
  }
});

test("format fingerprints ignore the reported line number", (t) => {
  const result = run(t, { fmt: {
    status: 1, stdout: "Diff in <ROOT>/contracts/stream/src/lib.rs:123:\n-old\n+new\n",
  } }, ["contracts/stream/src/lib.rs::fmt::needs-format"]);
  assert.equal(result.status, 0);
  assert.match(result.stdout, /1 current, 1 baseline, 0 NEW/);
});

test("an incomplete run cannot overwrite the baseline", (t) => {
  const result = run(t, { clippy: {
    status: 101, stdout: "", stderr: "error: could not execute process\n",
  } }, [fingerprint], ["--update-baseline"]);
  assert.equal(result.status, 2);
  assert.equal(result.baseline, result.original);
});

test("a complete baseline update records namespaced lint codes", (t) => {
  const result = run(t, { clippy: { status: 101, stdout: lint() + finished(false) } }, [], ["--update-baseline"]);
  assert.equal(result.status, 0);
  assert.deepEqual(JSON.parse(result.baseline).fingerprints, [fingerprint]);
});
