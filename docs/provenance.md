# Wasm provenance — release integrity

**Status: implemented for issue #1546.**

The workspace produces deployable wasm cdylibs, but nothing used to tie the
bytes to the exact inputs of the build. This document records the design
decisions made for that gap, the manifest schema, and the failure contract of
the tooling.

---

## The problem

A `fluxora_stream.wasm` artifact is only meaningful if you can answer: *which
source revision was this built from, with which toolchain, SDK, target and
release flags, and are these bytes actually the ones that build produced?*
Without that, a released contract cannot be reproduced, audited, or
cross-checked against a deployment.

## Design decisions

### 1. Format: a self-contained JSON manifest plus `SHASUMS`

**Decision:** a machine-readable `provenance.json` per build, following
[SLSA v1.0](https://slsa.dev/provenance/v1.0) conventions — a `subject` list
of artifact digests plus a build definition — without adopting in-toto
attestation envelopes or an external signing/attestation service.

Why this shape and not something heavier:

- **Self-contained and offline.** The manifest is produced and verified by a
  local tool with no network dependency, so it works identically on a
  developer laptop, in CI, and in an air-gapped release process. An in-toto
  signed attestation would require key management and a verification
  infrastructure that this repo does not have and the issue does not ask for.
- **SLSA-conventional, not SLSA-certified.** The schema uses SLSA's
  vocabulary (subject digests, build definition, build type) so anyone familiar
  with the standard can read it, but we do not claim SLSA attestation
  compliance. The threat model here is *accidental or unnoticed drift*, not a
  malicious build system.
- **`SHASUMS` is the low-friction consumer path.** Alongside the manifest, a
  `SHASUMS` file in `sha256sum` format is written, so anyone can confirm the
  artifacts with one stock command: `sha256sum -c SHASUMS`. Consumers who want
  the full provenance read `provenance.json`; consumers who just want to pin
  bytes use `SHASUMS`.

### 2. Verification is mandatory before deployment

**Decision:** yes. `verify` is a hard gate: it re-hashes every artifact,
checks that the manifest covers every wasm in the release dir and nothing
else, and re-reads the environment to confirm the recorded build inputs have
not drifted. Any mismatch is a non-zero exit, which blocks release.

It is wired into CI (generate then verify in the same job, so the job fails on
a mismatch) and is the final step of `script/provenance.sh build`. A release
must not ship a wasm that has not been verified against its manifest.

### 3. Target: `wasm32v1-none`

The issue's verification snippet uses `wasm32-unknown-unknown`, but this repo
pins `wasm32v1-none` in `rust-toolchain.toml` — `wasm32-unknown-unknown`
"still builds but emits unsupported features" on protocol 23+ (see
`rust-toolchain.toml`). The provenance tool records whatever target it is
told, defaulting to `wasm32v1-none`, and `verify` fails if the recorded target
does not match. The PR that implements this therefore verifies with:

```bash
cargo build --workspace --target wasm32v1-none --release
sha256sum -c target/wasm32v1-none/release/SHASUMS
```

---

## The manifest

Written next to the artifacts as `provenance.json` (pretty-printed, atomic
write). Example:

```json
{
  "schema": "https://slsa.dev/provenance/v1.0",
  "subject": [
    {
      "name": "fluxora_archival_probe.wasm",
      "sha256": "d3507515f3a071d6d8318cf2101e05fbdf0bf5abe9d3ad85c1f5ed56aca49aa2"
    },
    {
      "name": "fluxora_stream.wasm",
      "sha256": "1e597a4f7296a6344dfb561aff85f2e1adcd258434da26ce475847488d61d766"
    }
  ],
  "build": {
    "build_type": "https://github.com/Fluxora-Org/Fluxora-Contracts/provenance/v1",
    "target": "wasm32v1-none",
    "profile": {
      "codegen-units": 1,
      "debug": 0,
      "debug-assertions": false,
      "lto": true,
      "opt-level": "z",
      "overflow-checks": true,
      "panic": "abort",
      "strip": "symbols"
    },
    "git_revision": "07fd236bc4048097a69dc6765a9cc685ee556007",
    "git_ref": "main",
    "git_dirty": false,
    "toolchain_channel": "1.97.1",
    "rustc": "rustc 1.97.1 (8bab26f4f 2026-07-14)",
    "cargo": "cargo 1.97.1 (c980f4866 2026-06-30)",
    "soroban_sdk": "27.0.5",
    "host": "x86_64-unknown-linux-gnu",
    "started_on": "2026-08-26T19:33:43Z"
  }
}
```

| field | source | role |
|---|---|---|
| `schema` | constant | SLSA provenance v1.0 URL the shape follows |
| `subject[].name` / `.sha256` | release dir scan | every `*.wasm` present, hex SHA-256 |
| `build.build_type` | constant | this repo's provenance schema URL |
| `build.target` | CLI `--target` | wasm target triple |
| `build.profile` | workspace `Cargo.toml` `[profile.release]` | release flags, verbatim |
| `build.git_revision` | `git rev-parse HEAD` | the exact source commit |
| `build.git_ref` | `git branch --show-current` | branch, if any (context only) |
| `build.git_dirty` | `git status --porcelain` | whether the tree had uncommitted changes at build time |
| `build.toolchain_channel` | `rust-toolchain.toml` | pinned toolchain channel |
| `build.rustc` / `build.cargo` | `rustc --version` / `cargo --version` | exact compiler identity |
| `build.soroban_sdk` | workspace `Cargo.lock` | SDK major/minor/patch |
| `build.host` | `rustc -vV` | builder host triple (context only) |
| `build.started_on` | UTC clock | ISO-8601 build time (informational) |

`SHASUMS` is the same digests in `sha256sum` format:

```
d3507515f3a071d6d8318cf2101e05fbdf0bf5abe9d3ad85c1f5ed56aca49aa2  fluxora_archival_probe.wasm
1e597a4f7296a6344dfb561aff85f2e1adcd258434da26ce475847488d61d766  fluxora_stream.wasm
```

## What verification checks

`verify` fails (non-zero exit, specific message) when any of the following
holds:

1. **Integrity** — a recorded artifact is missing from the release dir, or its
   bytes hash to something other than the recorded digest.
2. **Completeness** — a `*.wasm` in the release dir is not listed in the
   manifest. "Provenance for every contract" is enforced both directions, so a
   newly added contract can never slip out unprovenanced.
3. **Environment consistency** — the recorded `target`, `git_revision`,
   `rustc`, `cargo`, `soroban_sdk`, `toolchain_channel` or `profile` no longer
   match the current checkout and toolchain. This catches a manifest that was
   generated from a different commit or with different build inputs than the
   one being released.

Warnings only (never blocking): `git_ref` drift (the revision stays pinned),
and a recorded dirty working tree. `host` and `started_on` are informational.

## Usage

```bash
script/provenance.sh build                      # wasm build + generate + verify — the release gate
script/provenance.sh generate [release-dir]     # write provenance.json + SHASUMS
script/provenance.sh verify   [release-dir]     # re-check; exit non-zero on any mismatch
script/provenance.sh test                       # regression suite for the tool itself
```

The tool is a standalone Rust crate at `tools/provenance` — deliberately not a
workspace member, because it targets the host while the workspace's product
crates target `wasm32v1-none`. Invoke it directly as
`fluxora-provenance generate|verify <release-dir> [--target <triple>]`
if needed; see `fluxora-provenance help` for the full CLI.

## Failure, boundary, retry, and authorization behavior

Explicit contract, so the gate is predictable:

- **Failure** — every failure mode is a typed error with a specific message
  and a non-zero exit: missing release dir, dir is not a directory, no `*.wasm`
  found, workspace root not found, unparseable manifest, missing/unlisted
  artifact, hash mismatch (both hashes reported), metadata drift (field and
  both values reported), git/toolchain invocation failure. `verify` never
  writes anything.
- **Boundaries** — an empty release dir is an error (`no *.wasm artifacts`),
  not a silently-empty manifest; a release gate that covers nothing is worse
  than one that fails. Artifacts are sorted by name so the manifest is
  deterministic. Non-wasm files in the release dir are ignored.
- **Retry** — all commands are idempotent. After fixing the cause (rebuild,
  regenerate, or correct the drift) simply re-run. `generate` overwrites the
  previous manifest via a temp-file rename, so a crash never leaves a
  half-written manifest that a later gate could mistake for valid.
- **Authorization** — provenance is a build-time integrity gate, not a
  network service, so there is no runtime authorization surface. The
  enforcement point is process: CI runs `generate` then `verify` in the same
  job and fails on mismatch, and `script/provenance.sh build` ends with
  `verify`. Anyone who can run the release pipeline can bypass it only by
  deliberately not running it — which is why the gate lives in CI, not in
  documentation.

## Reproducibility

The manifest pins every input a rebuild needs: source commit, toolchain
channel and exact `rustc`/`cargo` builds, SDK version, target, and the full
`[profile.release]` table. That is what makes a *reproducible* rebuild
checkable — the same inputs should yield the same bytes. The CI gate verifies
the integrity of the actual artifacts and the consistency of the recorded
inputs; it does not perform a cross-machine rebuild-and-diff, which cargo does
not guarantee byte-for-byte. A `verify` pass therefore means "these bytes are
what this manifest describes", and `sha256sum -c SHASUMS` is the way a
consumer independently confirms the bytes they hold.
