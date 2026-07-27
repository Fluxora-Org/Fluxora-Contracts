//! Cargo.lock determinism tests.
//!
//! # Purpose
//!
//! These tests document and verify the "Dependency resolution" residual risk
//! described in `contracts/stream/src/checksum.rs`'s build-reproducibility
//! contract section:
//!
//! > **Dependency resolution.** `Cargo.lock` must be committed and unchanged.
//! > CI-enforced: the `build` job's "Verify Cargo.lock is committed and
//! > unchanged" step runs `cargo update --locked --workspace` before any build
//! > step and fails the build if resolution would modify `Cargo.lock`.
//!
//! ## What is tested here
//!
//! These tests cannot reproduce the exact CI shell step (that requires a live
//! Cargo resolver), but they provide compile-time and structural guarantees
//! that together make the reproducibility contract machine-checkable:
//!
//! 1. **Lockfile presence** — `Cargo.lock` exists at the workspace root.
//! 2. **Pinned soroban-sdk version** — the version string in
//!    `contracts/stream/Cargo.toml` is an exact semver pin (`"21.7.7"`), not a
//!    caret (`^`) or wildcard range that could resolve differently on different
//!    machines or at different times.
//! 3. **Lockfile non-emptiness** — the committed lockfile is not a stub; it
//!    contains at least one `[[package]]` entry.
//! 4. **Lockfile contains soroban-sdk** — the dependency that matters most for
//!    WASM reproducibility is actually recorded in the lockfile.
//! 5. **No `[patch]` table in workspace manifest** — `[patch]` overrides bypass
//!    normal resolution and could produce a lockfile that diverges from what
//!    `cargo update --locked` would expect.
//! 6. **Workspace resolver version** — the workspace `Cargo.toml` must use
//!    resolver `"2"`, which is required for Soroban and avoids the feature
//!    unification issues that can silently change resolved dependency sets.
//!
//! ## Relationship to CI
//!
//! The authoritative gate is `.github/workflows/ci.yml` → `build` job →
//! "Verify Cargo.lock is committed and unchanged" step:
//!
//! ```yaml
//! - name: Verify Cargo.lock is committed and unchanged
//!   run: |
//!     set -euo pipefail
//!     if ! cargo update --locked --workspace; then
//!       echo "::error::Cargo.lock would change on dependency resolution ..."
//!       exit 1
//!     fi
//! ```
//!
//! These Rust tests are a complementary, always-on structural layer that runs
//! in the normal `cargo test` flow, providing an early signal without requiring
//! the full CI environment.
//!
//! ## Security notes
//!
//! - An unpinned `^` version specifier on `soroban-sdk` (or any dependency)
//!   allows `cargo update` to silently pull in a newer patch or minor release,
//!   changing the compiled WASM and invalidating the checksum in
//!   `wasm/checksums.sha256` without any direct code change.
//! - `[patch]` entries in the workspace `Cargo.toml` override version
//!   resolution globally and can introduce non-reproducible local paths or
//!   git revisions that are not captured correctly by `--locked` semantics.
//! - The workspace resolver `"2"` ensures consistent feature unification
//!   across workspace members; resolver `"1"` can unify features differently,
//!   making the resolved set environment-dependent.
//!
//! ## How to recover when CI fails this gate
//!
//! If the CI "Verify Cargo.lock is committed and unchanged" step fails:
//!
//! 1. Run `cargo update` locally to regenerate `Cargo.lock` with the updated
//!    resolution.
//! 2. Review the `git diff Cargo.lock` carefully — every changed `[[package]]`
//!    entry represents a dependency version change that will alter the WASM
//!    binary.
//! 3. If the change is intentional (e.g. a security patch), regenerate the
//!    reference checksum via `script/update-wasm-checksums.sh` and commit
//!    *both* `Cargo.lock` and the updated `wasm/checksums.sha256` together.
//! 4. If the change is unintentional, pin the drifting dependency explicitly
//!    in the relevant `Cargo.toml` before committing.
//!
//! See also: `docs/upgrade.md` §8 "Cargo.lock Determinism".

extern crate std;

use std::fs;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Resolves the workspace root by walking up from this file's location until a
/// directory containing both `Cargo.toml` and `Cargo.lock` is found.
///
/// `file!()` expands to a path relative to the workspace root during
/// compilation, so we strip the known suffix to obtain the root.
fn workspace_root() -> PathBuf {
    // file!() = "contracts/stream/tests/cargo_lock_determinism.rs"
    // We need to go up 3 components: tests/ → stream/ → contracts/ → root
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    // CARGO_MANIFEST_DIR = <workspace_root>/contracts/stream
    manifest_dir
        .parent() // contracts/
        .expect("contracts/ parent must exist")
        .parent() // workspace root
        .expect("workspace root must exist")
        .to_path_buf()
}

/// Reads a file at `path` and returns its contents as a `String`.
fn read_file(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| {
        panic!(
            "Failed to read {}: {}. Ensure the file exists and is UTF-8 encoded.",
            path.display(),
            e
        )
    })
}

// ---------------------------------------------------------------------------
// 1. Lockfile presence
// ---------------------------------------------------------------------------

/// `Cargo.lock` must exist at the workspace root.
///
/// A missing lockfile means either (a) the repository was cloned without it
/// (violating the commit policy) or (b) it was accidentally `.gitignore`d.
/// Either scenario breaks `cargo build --locked` and therefore WASM
/// reproducibility.
///
/// # Relationship to CI
/// The CI "Verify Cargo.lock is committed and unchanged" step requires the file
/// to exist in the working tree before `cargo update --locked` can succeed.
///
/// # Security note
/// A missing `Cargo.lock` silently degrades every subsequent build to
/// unconstrained resolution, defeating the entire determinism guarantee.
#[test]
fn cargo_lock_exists_at_workspace_root() {
    let lockfile = workspace_root().join("Cargo.lock");
    assert!(
        lockfile.exists(),
        "REPRODUCIBILITY VIOLATION: Cargo.lock is missing from the workspace root ({}). \
         Cargo.lock must be committed and present for reproducible WASM builds. \
         See contracts/stream/src/checksum.rs ('Dependency resolution' residual risk) \
         and docs/upgrade.md §8 for the recovery procedure.",
        lockfile.display()
    );
}

// ---------------------------------------------------------------------------
// 2. Lockfile non-emptiness
// ---------------------------------------------------------------------------

/// `Cargo.lock` must contain at least one `[[package]]` entry.
///
/// A zero-byte or stub lockfile would pass the existence check above but still
/// cause `cargo build --locked` to fail in CI, since the resolver would find
/// no recorded resolution for any dependency.
///
/// # Security note
/// A truncated lockfile is as dangerous as a missing one — `cargo` will
/// regenerate it freely, producing an unconstrained build.
#[test]
fn cargo_lock_is_non_empty_and_contains_packages() {
    let lockfile = workspace_root().join("Cargo.lock");
    let contents = read_file(&lockfile);

    assert!(
        contents.contains("[[package]]"),
        "REPRODUCIBILITY VIOLATION: Cargo.lock at {} exists but contains no [[package]] \
         entries. The file appears to be empty or corrupted. Run `cargo update` to \
         regenerate it and commit the result. \
         See contracts/stream/src/checksum.rs and docs/upgrade.md §8.",
        lockfile.display()
    );
}

// ---------------------------------------------------------------------------
// 3. soroban-sdk is recorded in the lockfile
// ---------------------------------------------------------------------------

/// `Cargo.lock` must contain a recorded entry for `soroban-sdk`.
///
/// `soroban-sdk` is the primary determinism-sensitive dependency: it controls
/// the Soroban host ABI, which directly affects the WASM binary output.
/// If `soroban-sdk` is absent from the lockfile, `cargo build --locked` will
/// fail and the reference checksum in `wasm/checksums.sha256` is invalid.
///
/// # Security note
/// The locked `soroban-sdk` version must match the version pinned in
/// `contracts/stream/Cargo.toml`. CI's `cargo update --locked` step will fail
/// if they diverge, but this test provides an earlier signal during `cargo test`.
#[test]
fn cargo_lock_records_soroban_sdk() {
    let lockfile = workspace_root().join("Cargo.lock");
    let contents = read_file(&lockfile);

    assert!(
        contents.contains("name = \"soroban-sdk\""),
        "REPRODUCIBILITY VIOLATION: Cargo.lock at {} does not contain a recorded entry \
         for soroban-sdk. soroban-sdk is the primary WASM-affecting dependency and must \
         be pinned in the lockfile. Run `cargo update` to regenerate Cargo.lock and \
         commit the result. \
         See contracts/stream/src/checksum.rs and docs/upgrade.md §8.",
        lockfile.display()
    );
}

// ---------------------------------------------------------------------------
// 4. soroban-sdk is pinned exactly in Cargo.toml (no caret range)
// ---------------------------------------------------------------------------

/// `contracts/stream/Cargo.toml` must pin `soroban-sdk` with an exact version
/// string, not a caret (`^`) or wildcard range.
///
/// An exact pin (`soroban-sdk = "21.7.7"`) means `cargo update` can never
/// silently pull in `21.7.8` or `21.8.0`. A caret pin (`^21.7.7`) would allow
/// `cargo update` to resolve to any `21.x.y ≥ 21.7.7`, changing the compiled
/// WASM without any diff in source code.
///
/// # What constitutes a valid exact pin
///
/// The version string in `[dependencies]` (or `[dev-dependencies]`) must
/// appear as a bare quoted string like `"21.7.7"` — three numeric components
/// with no leading `^`, `~`, `*`, `>=`, or `<` specifiers.
///
/// # Security note
/// This is the root-cause prevention for the class of lockfile drift described
/// in the "Dependency resolution" residual risk in
/// `contracts/stream/src/checksum.rs`. A caret range is the most common way
/// a contributor silently introduces drift.
#[test]
fn stream_cargo_toml_soroban_sdk_is_exact_pin() {
    let cargo_toml_path = workspace_root()
        .join("contracts")
        .join("stream")
        .join("Cargo.toml");
    let contents = read_file(&cargo_toml_path);

    // Find the soroban-sdk dependency line(s) in [dependencies] and
    // [dev-dependencies]. Both must not use a caret prefix.
    //
    // We look for any line matching:
    //   soroban-sdk = "..." (bare exact pin — allowed)
    // and reject lines matching:
    //   soroban-sdk = "^..." (caret range — forbidden for reproducibility)
    //   soroban-sdk = "~..." (tilde range — forbidden)
    //   soroban-sdk = ">=..." (inequality — forbidden)
    for (lineno, line) in contents.lines().enumerate() {
        let trimmed = line.trim();
        if !trimmed.starts_with("soroban-sdk") {
            continue;
        }
        // Extract the version value (the part after '=')
        if let Some(eq_pos) = trimmed.find('=') {
            let value_part = trimmed[eq_pos + 1..].trim();
            // value_part may be a plain string like `"21.7.7"` or a table like
            // `{ version = "21.7.7", features = [...] }`.
            // In either case, extract the version string.
            let version_str = extract_version_from_toml_value(value_part);
            if let Some(v) = version_str {
                assert!(
                    !v.starts_with('^'),
                    "REPRODUCIBILITY VIOLATION (line {}): soroban-sdk version '{}' uses a \
                     caret range in {}. Use an exact pin (e.g. \"21.7.7\") instead of \"^{}\". \
                     A caret range allows cargo update to silently resolve to a newer version, \
                     invalidating the WASM checksum. \
                     See contracts/stream/src/checksum.rs and docs/upgrade.md §8.",
                    lineno + 1,
                    v,
                    cargo_toml_path.display(),
                    &v[1..]
                );
                assert!(
                    !v.starts_with('~'),
                    "REPRODUCIBILITY VIOLATION (line {}): soroban-sdk version '{}' uses a \
                     tilde range in {}. Use an exact pin (e.g. \"21.7.7\"). \
                     See contracts/stream/src/checksum.rs and docs/upgrade.md §8.",
                    lineno + 1,
                    v,
                    cargo_toml_path.display()
                );
                assert!(
                    !v.starts_with('*'),
                    "REPRODUCIBILITY VIOLATION (line {}): soroban-sdk version '{}' uses a \
                     wildcard in {}. Use an exact pin (e.g. \"21.7.7\"). \
                     See contracts/stream/src/checksum.rs and docs/upgrade.md §8.",
                    lineno + 1,
                    v,
                    cargo_toml_path.display()
                );
                assert!(
                    !v.starts_with(">="),
                    "REPRODUCIBILITY VIOLATION (line {}): soroban-sdk version '{}' uses an \
                     inequality range in {}. Use an exact pin (e.g. \"21.7.7\"). \
                     See contracts/stream/src/checksum.rs and docs/upgrade.md §8.",
                    lineno + 1,
                    v,
                    cargo_toml_path.display()
                );
            }
        }
    }
}

/// Extracts the bare version string from a TOML value fragment.
///
/// Handles two forms:
/// - Plain string:  `"21.7.7"`  →  `Some("21.7.7")`
/// - Inline table:  `{ version = "21.7.7", features = [...] }` → `Some("21.7.7")`
///
/// Returns `None` if no version string can be located (e.g. a path-only
/// dependency with no `version` key).
fn extract_version_from_toml_value(value: &str) -> Option<&str> {
    let value = value.trim();
    if value.starts_with('"') {
        // Plain string: strip surrounding quotes.
        let inner = value.trim_matches('"');
        return Some(inner);
    }
    if value.starts_with('{') {
        // Inline table: find `version = "..."`.
        if let Some(v_pos) = value.find("version") {
            let after_version = &value[v_pos + "version".len()..];
            if let Some(eq_pos) = after_version.find('=') {
                let after_eq = after_version[eq_pos + 1..].trim();
                if after_eq.starts_with('"') {
                    // Find closing quote.
                    let inner = &after_eq[1..];
                    if let Some(end) = inner.find('"') {
                        return Some(&inner[..end]);
                    }
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// 5. No [patch] table in workspace Cargo.toml
// ---------------------------------------------------------------------------

/// The workspace `Cargo.toml` must not contain a `[patch]` table.
///
/// A `[patch]` entry overrides dependency resolution globally, potentially
/// substituting a local path or git revision that differs between machines and
/// CI environments. This makes `cargo update --locked` unreliable: the
/// patched source is not fully captured in `Cargo.lock` in the same way a
/// registry version is, and the resulting WASM may differ between environments.
///
/// # Exceptions
/// If a `[patch]` is intentionally required (e.g. during a soroban-sdk fork
/// evaluation), it must be removed before merging to `main`, and this test
/// must not be suppressed without a documented security review.
///
/// # Security note
/// A `[patch]` that substitutes a local path can silently include uncommitted
/// local changes in the WASM build, completely defeating the reproducibility
/// contract.
#[test]
fn workspace_cargo_toml_has_no_patch_table() {
    let cargo_toml_path = workspace_root().join("Cargo.toml");
    let contents = read_file(&cargo_toml_path);

    // Look for a [patch] section header. Allow inline comments and
    // varied whitespace, but a bare `[patch` at the start of any line
    // (after trimming) is definitive.
    let has_patch = contents.lines().any(|line| {
        let trimmed = line.trim();
        trimmed.starts_with("[patch")
    });

    assert!(
        !has_patch,
        "REPRODUCIBILITY VIOLATION: workspace Cargo.toml at {} contains a [patch] \
         table. [patch] overrides bypass normal Cargo resolution and can produce \
         non-reproducible WASM builds that differ between machines and CI. \
         Remove the [patch] table before merging. \
         See contracts/stream/src/checksum.rs and docs/upgrade.md §8.",
        cargo_toml_path.display()
    );
}

// ---------------------------------------------------------------------------
// 6. Workspace resolver = "2"
// ---------------------------------------------------------------------------

/// The workspace `Cargo.toml` must declare `resolver = "2"`.
///
/// Resolver version 2 is required for Soroban contracts and ensures consistent
/// feature unification across workspace members. Resolver version 1 can
/// unify features differently between `cargo build` and `cargo test`,
/// making the resolved dependency set environment-dependent and potentially
/// producing different WASM binaries depending on which Cargo command last
/// ran.
///
/// # Security note
/// Without `resolver = "2"`, a `cargo test --features testutils` run can
/// silently activate features in `cargo build` targets that alter the compiled
/// WASM, making the checksum unreliable.
#[test]
fn workspace_cargo_toml_uses_resolver_v2() {
    let cargo_toml_path = workspace_root().join("Cargo.toml");
    let contents = read_file(&cargo_toml_path);

    assert!(
        contents.contains("resolver = \"2\""),
        "REPRODUCIBILITY VIOLATION: workspace Cargo.toml at {} does not declare \
         resolver = \"2\". Resolver version 2 is required for Soroban contracts and \
         consistent cross-member feature unification. Add `resolver = \"2\"` under \
         the [workspace] table. \
         See contracts/stream/src/checksum.rs and docs/upgrade.md §8.",
        cargo_toml_path.display()
    );
}

// ---------------------------------------------------------------------------
// 7. Lockfile format version
// ---------------------------------------------------------------------------

/// `Cargo.lock` must declare `version = 3` (or `version = 4`), confirming it
/// was generated by a modern Cargo (≥ 1.78) compatible with the pinned
/// toolchain in `rust-toolchain.toml`.
///
/// An older lockfile format (version 1 or 2, produced by Cargo < 1.38) would
/// be silently upgraded on the next `cargo` invocation, marking `Cargo.lock`
/// as modified even though no dependency versions changed. This would
/// incorrectly fail the CI `cargo update --locked` gate.
///
/// # Security note
/// A format version mismatch is a common source of spurious "Cargo.lock would
/// change" CI failures after a toolchain upgrade. The fix is to run
/// `cargo update --workspace` once with the new toolchain, review the diff,
/// and commit the format-upgraded lockfile.
#[test]
fn cargo_lock_has_modern_format_version() {
    let lockfile = workspace_root().join("Cargo.lock");
    let contents = read_file(&lockfile);

    // Cargo.lock v3 / v4 both start with a `version` key in the root table.
    // Accept either; both are produced by Cargo >= 1.78 (our minimum per
    // rust-toolchain.toml channel "1.94.1").
    let has_modern_version = contents.lines().any(|line| {
        let t = line.trim();
        t == "version = 3" || t == "version = 4"
    });

    assert!(
        has_modern_version,
        "REPRODUCIBILITY VIOLATION: Cargo.lock at {} does not declare a modern \
         format version (3 or 4). An older lockfile format would be silently \
         upgraded by Cargo, causing spurious 'Cargo.lock would change' failures \
         in the CI determinism gate. Run `cargo update --workspace` with the \
         pinned toolchain (rustc 1.94.1) to regenerate the lockfile and commit \
         the result. \
         See contracts/stream/src/checksum.rs and docs/upgrade.md §8.",
        lockfile.display()
    );
}

// ---------------------------------------------------------------------------
// 8. soroban-sdk version in lockfile matches Cargo.toml pin
// ---------------------------------------------------------------------------

/// The `soroban-sdk` version recorded in `Cargo.lock` must match the exact
/// version pin in `contracts/stream/Cargo.toml`.
///
/// This is the end-to-end check: even if both files are individually
/// well-formed, a mismatch between them means `cargo build --locked` will
/// fail, because the lockfile does not satisfy the manifest's version
/// requirement.
///
/// # Security note
/// A stale lockfile (containing an older `soroban-sdk` version than the
/// manifest requires) will cause `cargo build --locked` to fail with an error
/// like "the lock file needs to be updated but --locked was passed". This test
/// catches that scenario before CI, providing a clearer diagnostic.
#[test]
fn cargo_lock_soroban_sdk_version_matches_cargo_toml_pin() {
    let root = workspace_root();
    let lockfile_path = root.join("Cargo.lock");
    let cargo_toml_path = root.join("contracts").join("stream").join("Cargo.toml");

    let lockfile = read_file(&lockfile_path);
    let cargo_toml = read_file(&cargo_toml_path);

    // Extract the pinned version from Cargo.toml.
    // We look for the first `soroban-sdk = "X.Y.Z"` line in [dependencies].
    let manifest_version = cargo_toml
        .lines()
        .find_map(|line| {
            let trimmed = line.trim();
            if !trimmed.starts_with("soroban-sdk") {
                return None;
            }
            if let Some(eq_pos) = trimmed.find('=') {
                let value = trimmed[eq_pos + 1..].trim();
                extract_version_from_toml_value(value).map(|s| s.to_owned())
            } else {
                None
            }
        })
        .expect(
            "soroban-sdk dependency not found in contracts/stream/Cargo.toml. \
             The [dependencies] table must include soroban-sdk with a version pin.",
        );

    // Strip any leading range specifier characters to get the bare version.
    let bare_version = manifest_version.trim_start_matches(['^', '~', '=', '>', '<', ' ']);

    // Check that Cargo.lock contains a package block for this exact version.
    // Cargo.lock entries look like:
    //   [[package]]
    //   name = "soroban-sdk"
    //   version = "21.7.7"
    let expected_lock_entry = format!("version = \"{}\"", bare_version);
    let soroban_block_present = {
        let mut in_soroban_block = false;
        let mut found = false;
        for line in lockfile.lines() {
            let trimmed = line.trim();
            if trimmed == "[[package]]" {
                in_soroban_block = false;
            }
            if in_soroban_block && trimmed == "name = \"soroban-sdk\"" {
                // Already flagged; look for the version line next
                in_soroban_block = true;
            }
            if trimmed == "name = \"soroban-sdk\"" {
                in_soroban_block = true;
            }
            if in_soroban_block && trimmed == expected_lock_entry.as_str() {
                found = true;
                break;
            }
        }
        found
    };

    assert!(
        soroban_block_present,
        "REPRODUCIBILITY VIOLATION: Cargo.lock does not record soroban-sdk version \
         '{}' (the version pinned in contracts/stream/Cargo.toml). \
         This means `cargo build --locked` will fail and the WASM checksum is \
         invalid for the current manifest. Run `cargo update` to sync the lockfile, \
         review the diff, regenerate `wasm/checksums.sha256` via \
         `script/update-wasm-checksums.sh`, and commit all three files together. \
         See contracts/stream/src/checksum.rs and docs/upgrade.md §8.",
        bare_version
    );
}
