# Contributing to Fluxora-Contracts

This repo holds `Fluxora-Contracts` — the Rust / Soroban smart contracts. If
you're new here, read this before writing any code.

## 1. Layout: what's live, what isn't

```
contracts/
  stream/            the product. Deployed to testnet, ABI frozen.
  archival-probe/    a throwaway probe, kept as a workspace member so its
                      smoke test runs in CI, but never released or deployed.
  factory/           NOT a workspace member. Has only a tests/ directory,
                      no Cargo.toml, no src/. Does not build.
  governance/         NOT a workspace member. Has src/lib.rs but no
                      Cargo.toml. Does not build.
```

Only two crates are in the workspace (`Cargo.toml` at the repo root):

```toml
[workspace]
members = ["contracts/archival-probe", "contracts/stream"]
```

`factory` and `governance` are **not** buildable or testable as-is — there is
no `Cargo.toml` for either, so `cargo build`, `cargo test`, and `cargo clippy`
never touch them, workspace-wide flags included. Don't assume a red build
there means you broke something; it means the crate was never wired in. If
your issue is about one of those two directories, say so in your PR and don't
expect `cargo test --workspace` to cover it — see `docs/MIGRATION.md` for why
governance was dropped from the product (§6: no admin key, no upgradeability)
and check the issue tracker for whether factory is meant to be restored.

`contracts/stream` is the interface of record: its ABI is **frozen** as of
2026-08-12 (`docs/ABI.md`). Anything not documented there isn't part of the
interface, and a breaking change means a new contract address, not an edit to
the deployed one.

`contracts/archival-probe` exists only to prove the live-network
archival/restore round trip that the unit suite can't (see
`docs/KNOWN-LIMITATIONS.md` §1). It stays in the workspace so its smoke test
runs, but `script/release.sh` builds *only* `fluxora-stream` and fails if a
probe wasm shows up in the output. Never deploy it to mainnet.

## 2. Commands CI runs — reproduce them locally

Four jobs in `.github/workflows/ci.yml` gate every PR: `docs-alignment-check`,
`lint`, `fuzz`, `packaging`, and `fuzz-feature-matrix` (plus `coverage`, which
runs but is allowed to fail — see §5). Run the same commands locally before
pushing:

```bash
# Formatting and lints — hard gate, warnings fail the build
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings

# Build and test the whole workspace
cargo build --workspace
cargo test --workspace --all-features

# Resource-cost regression report (prints, doesn't just assert)
cargo test --release resource_limits -- --nocapture --test-threads=1

# Release artifact — builds only fluxora-stream, never the probe
script/release.sh

# Cargo.lock must not drift from Cargo.toml
cargo update --locked --workspace

# WASM build + provenance
cargo build --release --workspace --target wasm32v1-none
script/provenance.sh build

# Package/artifact name guard (issue #1594) — catches an accidental rename
# of the deployable package or WASM file
cargo test --all-features packaging:: -- --nocapture --test-threads=1

# Feature-matrix check — every supported feature combination must compile
cargo check -p fluxora-stream
cargo check -p fluxora-stream --features testutils
cargo check -p fluxora-stream --no-default-features

# Doc-alignment / entrypoint-drift check (Python)
pip install pytest pytest-cov
pytest tests/ --cov=script/ --cov-fail-under=50 -v --tb=short
python3 script/validate-doc-alignment.py
```

`script/verify_rust_version.py` checks your installed `rustc` against the pin
in `rust-toolchain.toml` (currently `1.97.1` per that file; CI's `lint` job
separately pins `1.94.1` via `dtolnay/rust-toolchain` — match whichever job
you're trying to reproduce). Install the toolchain and target with:

```bash
rustup toolchain install 1.97.1
rustup target add wasm32v1-none --toolchain 1.97.1
rustup component add rustfmt clippy --toolchain 1.97.1
```

**Windows note:** several `cargo test` runs in this workspace create
temporary files under a fixed name and then try to rename or delete them
(model-registry-style GC tests are the classic case, but the stream contract
has its own tempfile-based tests too). If you see `OSError: [WinError 1314]
A required privilege is not held by the client` on Windows, it's usually
because Developer Mode (or "Create symbolic links") isn't enabled for your
account, or antivirus is holding a lock on the temp file mid-test. Run tests
from WSL2 if you hit this repeatedly — it avoids the whole class of failure.

## 3. Test expectations for a change to the stream contract

`contracts/stream/src/test/` is staged to match the build order (see the
module doc at the top of `test/mod.rs`):

- **Stage 1** — data model, create, withdraw, views, plus the property suite
  and the pool invariant.
- **Stage 2** — cliff, cancel, pause/resume, top-up, recipient transfer, and
  every adversarial boundary case.
- **Stage 3** — TTL survival, archival recovery within the test host, resource
  consumption at the batch cap.
- **Stage 4** — the stream-id invariant (unique, monotonic, never reused).

Any new test file must be added to `mod.rs` — `cargo test` only picks up a
module that's declared there. Check first: several files on disk
(`packaging.rs`, `snapshot_tests.rs`, `release_dry_run.rs`,
`read_methods_no_side_effects.rs`, `withdrawal_atomicity.rs`) are **not**
currently declared in `mod.rs`. If you're adding a file, don't assume an
existing undeclared file is a template to copy from without checking whether
it's supposed to run.

What a change to `contracts/stream` is expected to satisfy before review:

1. **The pool invariant holds after every operation you touch.** If your
   change affects accrual, withdraw, cancel, pause, or top-up, run
   `assert_invariants()` (from `test::common::Harness`) after the operation in
   any new test you write — this is the pattern every existing lifecycle test
   follows.
2. **Exact accounting, not "under a documented bound".** What the recipient
   withdraws plus what's refunded to the sender must equal `deposited`
   exactly. `vested` must be monotonic; rounding is always down and tight to
   one stroop; `top_up` must never reduce `vested`.
3. **Adversarial boundaries are covered explicitly**, not just the happy
   path: withdraw at exactly `cliff_time`, withdraw at exactly `end_time`,
   cancel one second after creation, cancel at the instant of creation, cancel
   after full vesting, pause/resume across the cliff, top-up on a cancelled
   stream (must reject), withdraw from a depleted stream (no-op or typed
   error, never a panic).
4. **If you touch the ABI** (any `#[contractimpl] pub fn`, an event's field
   order, an error discriminant, a struct field), update `docs/audit.md` in
   the same PR. CI's `docs-alignment-check` job diffs the entrypoint surface
   in `lib.rs` against that table and fails the build if they've drifted.
   Read `docs/ABI.md` first — it defines what counts as a compatible change
   (a new field at the end of an event, a new error discriminant, a new entry
   point) versus a breaking one (renamed/removed entry point, reordered
   parameters or event topics, renumbered error discriminant).
5. **If you touch `MAX_BATCH_SIZE`, the wasm size, or anything resource
   related**, re-measure — don't adjust the number by feel. See §3.2 of
   `fluxora-build-spec.md` for how the batch cap was derived, and
   `contracts/stream/wasm-size-budget.env` for the size gate (currently
   48,128 bytes baseline, 131,072 max).
6. **If you rename the package or the cdylib target**, update the canonical
   values in *both* `.github/workflows/ci.yml` (the `lint` and `packaging`
   jobs) and `contracts/stream/src/test/packaging.rs`
   (`EXPECTED_PACKAGE_NAME`/`EXPECTED_TARGET_NAME`) in the same PR. There are
   two independent gates for this (issue #1594); missing either one fails CI.

## 4. The nightly fuzz suite, and reproducing a failing seed

Two files drive long randomized operation sequences against the real
contract, re-checking every invariant after **every single operation**:
`contracts/stream/src/test/invariants.rs` and `.../lifecycle_proptest.rs`.
Both use a small deterministic PRNG (xorshift64\*) seeded explicitly, not
`rand` — so a failure is reproducible from its printed seed alone. This has
found two real bugs (a stream stuck `Depleted` with `paused_at` still set
after being paused post-maturity, and `top_up` rounding driving `vested`
backwards) that no hand-written case caught.

A per-PR `cargo test --workspace` run only uses each test's small default
budget. The nightly cron in `ci.yml` (`0 3 * * *`, in the `fuzz` job) raises
it via environment variables:

```bash
# What CI runs nightly — deeper than any local run should default to
FLUXORA_FUZZ_SEEDS=200 FLUXORA_FUZZ_STEPS=300 PROPTEST_CASES=5000 cargo test --release
```

`FLUXORA_FUZZ_SEEDS` / `FLUXORA_FUZZ_STEPS` control `invariants.rs`;
`PROPTEST_CASES` controls `lifecycle_proptest.rs`. Run this before a release,
or after touching `accrual.rs`.

**If the nightly run finds a failing seed:** the assertion message prints the
seed and the step, e.g. `seed 11400714819323198485, step 37: liability
conservation violated`. Reproduce it directly — no need to re-run the whole
sweep:

1. Open `lifecycle_proptest.rs`, find `regression_specific_seeds()`.
2. Add a line calling `run_lifecycle_sequence(<seed>, <steps>)` with the
   exact seed and step count from the failure (there's a commented example
   in that function already).
3. Run just that test: `cargo test regression_specific_seeds -- --nocapture`.
4. This replays the exact operation sequence deterministically — fix the bug,
   then leave the regression test in place so this seed never silently
   breaks again.

The equivalent function in `invariants.rs` is `run_sequence(seed, steps)` if
the failure came from that file instead — check which file's test name is in
the failure output.

## 5. A few things that will surprise you

- **`cargo test --workspace` covers the probe but not factory/governance.**
  See §1 — those two crates aren't in the workspace at all.
- **The `coverage` job is allowed to fail** (`continue-on-error: true`) — it
  hits a `rand_core` version skew via `cargo-tarpaulin` + `testutils`. Don't
  be alarmed if it's red; it isn't a merge blocker the way `lint` and `fuzz`
  are.
- **Some Python-side CI tooling tests currently fail against a fresh clone**
  (`tests/test_check_snapshot_diff.py`, `tests/test_rust_toolchain_pin.py`,
  parts of `tests/test_check_discriminant_collisions.py`) because they
  exercise functions or fixture files (`docs/error.md`, a `rustc` install)
  that aren't present in every environment. If you're touching
  `script/check_snapshot_diff.py`, `script/verify_rust_version.py`, or
  `script/check-discriminant-collisions.py`, run the matching test file
  first and check whether your change is expected to fix a pre-existing
  failure or you've introduced a new one — don't assume today's `main` is
  fully green on these.
- **The test-host's storage runs in recording mode**, so an expired
  persistent entry is silently auto-restored during `cargo test`. This means
  `test::ttl` proves the rent arithmetic but *not* the real-network recovery
  flow — that's what `script/archival-canary.sh` and the archival probe are
  for. Read `docs/KNOWN-LIMITATIONS.md` §1 before claiming TTL is "solved" by
  a green suite.

## Before opening a PR

- [ ] `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets --all-features -- -D warnings` are clean
- [ ] `cargo test --workspace --all-features` passes
- [ ] `script/release.sh` succeeds and produces only `fluxora_stream.wasm`
- [ ] If you touched the ABI, `docs/audit.md` is updated in the same PR
- [ ] If you touched accrual/withdraw/cancel/pause/top-up, new tests call
      `assert_invariants()` and cover the adversarial boundaries in §3
- [ ] Link your PR to the issue it closes (see the Stellar Wave Program note
      on your assigned issue — points are only awarded when the issue is
      marked complete by a maintainer)
