# Fluxora release runbook

One document for the whole release: what runs, in what order, who is allowed to
push to mainnet, and how to back out. Every script and every
`.github/workflows/ci.yml` input is covered below.

If a step here does not match the code, the code wins — fix this document in the
same PR.

---

## 1. Authorisation — who may trigger a mainnet deploy

There are exactly two gates. Both must pass; neither is optional.

| Gate | What it is | Who controls it |
|---|---|---|
| **Trigger permission** | GitHub only lets a user with **Write** access (triage/write/maintain/admin) dispatch `workflow_dispatch` on `Fluxora-Org/Fluxora-Contracts`. | Repository collaborators |
| **Environment approval** | The `deploy-mainnet` job binds to the GitHub environment `mainnet` (`environment: name: mainnet` in `ci.yml`). GitHub holds the job until the approval rules configured on that environment are satisfied. | Environment required reviewers |

**Rule:** a mainnet deploy may be triggered only by a designated release
maintainer — someone Write-or-above **and** listed as a required reviewer on the
`mainnet` environment — and it must be approved by a *different* designated
reviewer. Configure the environment with "Prevent self-review" enabled so the
person who dispatched the run cannot approve their own run.

Additional constraints that are already enforced in the repository:

* `deploy-mainnet` only runs on `workflow_dispatch` with
  `deploy_target == 'mainnet'`. It can never run from a push or a PR.
* `deploy-mainnet` hard-fails (exit 1, not a warning) when the
  `STELLAR_MAINNET_SECRET_KEY` secret is absent — a missing production secret
  stops the deploy rather than silently skipping it.
* `deploy-mainnet` depends on `lint`, so a red lint/test run blocks the deploy.
* Any change to `.github/workflows/**` must be reviewed by
  `@Johnsource-hub/maintainers` (`.github/CODEOWNERS`) before merge. Nobody can
  widen the deploy gate by pushing an unreviewed workflow edit.
* A **local** mainnet write (`script/release-dry-run.sh --confirm-write
  --network mainnet`) requires the deployer secret on the operator's machine.
  Treat `STELLAR_MAINNET_SECRET_KEY` as a production credential: it lives only
  in GitHub Actions secrets, never in the repo, a shell history, or a log.

**Testnet** is intentionally weaker: `deploy-testnet` runs automatically on
every push to `main` (after `lint`), or on a dispatch with
`deploy_target == 'testnet'`, using the `testnet` environment and
`STELLAR_TESTNET_SECRET_KEY`. It warns and skips when the secret is missing.

---

## 2. Script inventory — the role of each script

### Release-critical

| Script | Role | Mutates network? |
|---|---|---|
| `script/release.sh` | **The only producer of release artifacts.** Builds exactly `-p fluxora-stream` for `wasm32v1-none` (never `--workspace`), then asserts `fluxora_stream.wasm` exists *and* that `fluxora_archival_probe.wasm` is absent. Output: `target/wasm32v1-none/release/fluxora_stream.wasm`. Exits non-zero if the probe would ship. | No (local build) |
| `script/release-dry-run.sh` | Three-phase pre-flight before any deploy/upgrade: (1) offline validation — network id, contract-id/token strkey shape, artifact presence, size ≤ 128 KiB / 131 072 bytes, SHA-256, admin strkey, deployer identity; (2) read-only RPC validation — `getLatestLedger` health plus `stellar contract read` when a contract id is given; (3) write guard — **without `--confirm-write` it exits 0 having changed nothing**, with `--confirm-write` it runs `stellar contract deploy`. Flags: `--network`, `--rpc-url`, `--wasm`, `--contract-id`, `--source`, `--token`, `--admin`, `--expected-checksum`, `--confirm-write`, `--local-only`, `-h`. Exit codes: `0` success, `1` validation/execution failure, `2` bad arguments. | Only with `--confirm-write` |
| `script/test-release-dry-run.sh` | Regression suite *for* `release-dry-run.sh`. Builds mock/empty/oversized artifacts and asserts exit codes and messages across happy path, network-id rules, strkey boundaries, checksum mismatch, size limit, init inputs, the write guard, and retry idempotency. Exits non-zero if any case fails. | No |
| `script/provenance.sh` | Release-integrity gate. Subcommands: `build` (wasm build + generate + verify), `generate [dir]`, `verify [dir]`, `test`. Writes `provenance.json` (SLSA-shaped: git revision, rust toolchain, soroban-sdk version, target triple, profile) plus `SHASUMS` in `sha256sum` format next to the artifacts; `verify` re-hashes and fails the release on any drift. Release dir defaults to `target/<FLUXORA_WASM_TARGET>/release`. | No |
| `script/local-sandbox-proof.sh` | End-to-end smoke test with **zero** network credentials: starts a `stellar/quickstart` Docker container in standalone mode, builds and deploys `fluxora_stream.wasm`, creates a stream, queries `get_stream`/`withdrawable_of`/`vested_of`/`refundable_of`/`stream_count`/`stream_exists`, then tears the container down. Requires `docker` and `stellar`. | No (throwaway container) |

### Release-adjacent (validation and post-deploy)

| Script | Role | Mutates network? |
|---|---|---|
| `script/verify-wasm-checksum.sh [--no-build]` | Reproducibility check: rebuilds (unless `--no-build`) and compares SHA-256 of `fluxora_stream.wasm` / `.optimized.wasm` against their `.sha256` files. CI calls it with `--no-build`. | No |
| `script/check-stream-wasm-size.sh [budget]` | Enforces `BASELINE_BYTES`/`MAX_BYTES` from `contracts/stream/wasm-size-budget.env` against the built artifact. | No |
| `script/testnet-exercise.sh [CONTRACT_ID]` | Post-deploy credibility check: calls every public entrypoint against the live testnet deployment and asserts on-chain results, writing `script/testnet-exercise.log`. Needs `stellar` ≥ 27 and funded `fluxora-alice` / `fluxora-bob` / `fluxora-deployer`. | Yes (testnet writes) |
| `script/archival-canary.sh [--restore]` | Live archival/restore round trip using the throwaway probe contract. Without `--restore` it only reports status and exits 0 — safe any time. | Yes (testnet, probe only) |
| `script/verify_rust_version.py` | Asserts the running toolchain matches the `rust-toolchain.toml` pin. Run in every Rust CI job. | No |
| `script/update_soroban_version.py` | Bumps/validates the pinned soroban-sdk version and `soroban_version.txt`. | No |
| `script/check-discriminant-collisions.py` | Cross-file audit that no two error variants share a discriminant. Run in the `fuzz` job. | No |

Two rules the scripts encode and the runbook inherits:

* **Never** build release artifacts with `cargo build --workspace --release`;
  that is exactly how the archival probe wasm lands in the output directory.
  `script/release.sh` exists to make that impossible.
* **Never** deploy `fluxora_archival_probe.wasm` to testnet or mainnet. It is a
  throwaway that proves the archival round trip (see
  `docs/KNOWN-LIMITATIONS.md` §1), not product surface.

---

## 3. Workflow inputs, triggers, and environments

`.github/workflows/ci.yml` defines **one** input and **two** environments.

**Triggers:** `push` to `main`/`develop`, `pull_request`, nightly
`schedule` (`0 3 * * *`), and `workflow_dispatch`.

**Input (workflow_dispatch only):**

| Input | Type | Required | Default | Options | Effect |
|---|---|---|---|---|---|
| `deploy_target` | choice | yes | `testnet` | `testnet`, `mainnet` | Selects which deploy job is allowed to run. `testnet` → `deploy-testnet`; `mainnet` → `deploy-mainnet`. Any other event never runs a deploy job. |

**Jobs relevant to a release:**

| Job | Runs when | Environment | Secrets |
|---|---|---|---|
| `docs-alignment-check` | tests/scripts changed | — | — |
| `lint` | every trigger | — | — (builds the WASM and uploads `fluxora_stream-wasm-hash`) |
| `fuzz`, `coverage`, `packaging`, `fuzz-feature-matrix` | per their own `if`/`needs` | — | — |
| `deploy-testnet` | `push` to `main` **or** dispatch with `deploy_target = testnet`; `needs: lint` | `testnet` | `STELLAR_TESTNET_SECRET_KEY` (missing → warning + skip) |
| `deploy-mainnet` | dispatch with `deploy_target = mainnet` only; `needs: lint` | `mainnet` (url `https://stellar.expert`) | `STELLAR_MAINNET_SECRET_KEY` (missing → error + fail) |

Both deploy jobs download the artifact `fluxora_stream-wasm`, install the pinned
Stellar CLI (`STELLAR_CLI_VERSION`), load the deployer identity from the
environment secret, and run `stellar contract deploy` preferring
`fluxora_stream.optimized.wasm` when present.

---

## 4. The release sequence, end to end

Perform these in order. Stop at the first failing step.

### A. Pre-flight on a clean checkout of `main`

```bash
git fetch origin && git checkout main && git pull
python3 script/verify_rust_version.py        # toolchain pin matches
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
python3 script/check-discriminant-collisions.py
python3 script/validate_gas.py
python3 script/validate-doc-alignment.py
bash script/test-release-dry-run.sh          # dry-run regression suite
```

Deep randomized sweep (the nightly budget; run it before a release):

```bash
FLUXORA_FUZZ_SEEDS=200 FLUXORA_FUZZ_STEPS=300 PROPTEST_CASES=5000 cargo test --release
```

### B. Produce and gate the artifact

```bash
script/release.sh                 # ONLY path to fluxora_stream.wasm
script/provenance.sh build        # build + generate provenance.json/SHASUMS + verify
```

`provenance.sh verify` is the gate — a mismatch means the bytes, the toolchain,
the SDK version, the target or the profile drifted. Do not proceed; regenerate
after fixing the drift.

Confirm the artifact is the product contract and fits the budget:

```bash
ls -l target/wasm32v1-none/release/*.wasm     # expect fluxora_stream.wasm only
bash script/check-stream-wasm-size.sh
sha256sum -c target/wasm32v1-none/release/SHASUMS
```

### C. Local end-to-end proof (no credentials)

```bash
script/local-sandbox-proof.sh      # deploy + init + read + teardown in Docker
```

### D. Pre-flight against the target network (still read-only)

```bash
script/release-dry-run.sh --network testnet   --contract-id <C...> --wasm target/wasm32v1-none/release/fluxora_stream.wasm
script/release-dry-run.sh --network mainnet   --contract-id <C...> --wasm target/wasm32v1-none/release/fluxora_stream.wasm \
                          --expected-checksum <sha256 from SHASUMS>
```

Both must print `DRY-RUN SUCCESSFUL — NO NETWORK MUTATION PERFORMED` and exit 0.
Nothing has been written yet.

### E. Deploy to testnet and exercise it

Preferred (CI): merge to `main`. The push triggers the pipeline; when `lint`
goes green, `deploy-testnet` runs in the `testnet` environment.

Manual alternative:

```bash
script/release-dry-run.sh --network testnet --contract-id <C...> --wasm target/wasm32v1-none/release/fluxora_stream.wasm --confirm-write
```

Then prove the deployment:

```bash
script/testnet-exercise.sh <C...>            # every entrypoint vs live testnet
script/archival-canary.sh                    # status only; --restore for round trip
```

### F. Deploy to mainnet

1. Confirm the artifact checksum you recorded in step B still matches.
2. Actions → **CI/CD Pipeline** → **Run workflow** → set
   `deploy_target = mainnet`. Only a Write-or-above user can do this.
3. The run pauses at the `mainnet` environment. A **second** designated
   reviewer — not the triggerer — approves it. No approval, no deployment.
4. `deploy-mainnet` verifies the WASM artifact exists, installs the pinned CLI,
   loads `STELLAR_MAINNET_SECRET_KEY` (hard-fails if absent), and runs
   `stellar contract deploy --network mainnet`.
5. Record the new contract id, the artifact SHA-256, the workflow run URL and
   the commit SHA in the release notes, next to `provenance.json`/`SHASUMS`.

### G. After the deploy

* Re-run `script/release-dry-run.sh --network mainnet --contract-id <new C...>`
  with **no** `--confirm-write` to confirm the deployed code is readable and its
  checksum is the one you released.
* Publish the contract id and the `SHASUMS` digest so integrators can pin bytes.
* Keep `provenance.json` and `SHASUMS` with the release tag — they are the
  rollback reference.

---

## 5. Rollback

**There is no in-place rollback.** The core contract is immutable by design: no
admin key, no upgrade path (`docs/ABI.md` § "What frozen means",
`contracts/stream/src/lib.rs`). Once wasm is deployed, those bytes are fixed for
the life of that contract id.

That gives two distinct situations, and the runbook handles each separately.

### 5.1 A bad *deployment* (wrong artifact, wrong network, accidental dispatch)

A deploy always mints a **new** contract id; it never mutates an existing
contract. So the previous deployment is still live and untouched.

1. Do **not** publish the new contract id. Leave it unused.
2. Keep integrators on the last published, provenance-verified contract id.
3. Revoke/rotate nothing on-chain (there is no admin) — instead treat the stray
   id as abandoned and note it in the release log so nobody redeploy-picks it.
4. If the dispatch itself was unauthorised, audit who could trigger it and
   tighten the `mainnet` environment reviewers; workflow changes go through
   `@Johnsource-hub/maintainers` per CODEOWNERS.

### 5.2 A bad *artifact* that is already published to integrators

Roll back by redeploying the last known-good artifact as a new contract id and
repointing integrators — state cannot be moved between contract ids.

```bash
git checkout <last-good-tag>
script/release.sh                                   # rebuild the known-good product only
sha256sum -c target/wasm32v1-none/release/SHASUMS   # must match that tag's recorded digests
script/provenance.sh verify
script/release-dry-run.sh --network mainnet --wasm target/wasm32v1-none/release/fluxora_stream.wasm \
                          --expected-checksum <good sha256> --confirm-write   # after environment approval
```

Then, in order: publish the new contract id, mark the bad id as withdrawn in the
release notes and `docs/ABI.md` errata, and re-run
`script/testnet-exercise.sh` (and the testnet deploy first, per section E) so the
replacement is proven before integrators move.

### 5.3 Testnet

Testnet is disposable. Re-run the deploy from a known-good commit: push that
commit to `main` (automatic `deploy-testnet`) or dispatch with
`deploy_target = testnet`, then re-run `script/testnet-exercise.sh` against the
new id.

### 5.4 What "rolled back" means for state

Streams created on the withdrawn contract id stay there; there is no migration
entrypoint. Coordinate with stream creators before abandoning an id, and prefer
rolling back **before** real value is deposited.

---

## 6. Known gaps in the current pipeline

Recorded so the runbook stays truthful; fix them in their own PRs.

* **Deploy jobs reference an artifact nothing uploads.** Both `deploy-testnet`
  and `deploy-mainnet` download `fluxora_stream-wasm`, but `ci.yml` only
  uploads `fluxora_stream-wasm-hash`, `snapshot-validation-failure`, and
  `coverage-report`. Until an uploader is added, both deploy jobs fail at the
  download step — so a mainnet deploy currently has to go through the manual
  path in §4 F (`script/release-dry-run.sh --confirm-write --network mainnet`)
  with environment approval arranged out of band.
* **`script/test-release-dry-run.sh` is not wired into CI.** Run it locally in
  step A; it is not a merge gate yet.
* **`docs/error.md` / `docs/streaming.md` do not exist**, so the relevant
  branches of `script/validate-doc-alignment.py` self-skip rather than check.
* **`deploy-mainnet` environment approval must actually be configured.** The
  workflow binds to `environment: mainnet`; if no required reviewers are set on
  that environment, GitHub starts the job without an approval pause and any
  Write-access user who can dispatch can effectively self-approve.

---

## 7. Acceptance

| Acceptance criterion | Where it is satisfied |
|---|---|
| One document describes the release sequence end to end | §4 A→G |
| States who may trigger a mainnet deploy and what approval is required | §1 (Write access **and** `mainnet` environment approval by a second designated reviewer) |
| Each script's role is explained | §2 (all `script/*.sh` and release-relevant Python scripts) |
| Rollback is described | §5 |
| A release can be performed by following the document alone | §4 with the §6 gaps called out |
| Every workflow input covered | §3 (`deploy_target`: `testnet` \| `mainnet`) |
