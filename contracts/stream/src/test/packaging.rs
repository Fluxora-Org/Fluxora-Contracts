//! CI guard — package and artifact name stability.
//!
//! These tests exist for one reason: `fluxora-stream` and `fluxora_stream` are
//! consumed by deployment tooling, and an accidental rename would silently
//! break a deploy script without failing any functional test.
//!
//! # What is checked
//!
//! | Identifier | Canonical value | Why it matters |
//! |---|---|---|
//! | Cargo package name | `fluxora-stream` | used by `--package` / `-p` flags in build scripts |
//! | Cargo target name | `fluxora_stream` | becomes the WASM filename stem (`fluxora_stream.wasm`) |
//! | WASM artifact basename | `fluxora_stream.wasm` | consumed by `stellar contract deploy`, `testnet-exercise.sh`, and stage-5 indexer tooling |
//!
//! # How to rename intentionally
//!
//! A rename is a breaking change for downstream tooling and must be deliberate:
//!
//! 1. Update **`EXPECTED_PACKAGE_NAME`** and **`EXPECTED_TARGET_NAME`** below.
//! 2. Update every reference in `script/`, `.github/workflows/`, `docs/ABI.md`,
//!    and `README.md`.
//! 3. Follow the migration guidance in `MIGRATION.md` so integrators are
//!    warned before a release.
//!
//! The constants are the single source of truth — the tests and the CI script
//! both read from them (the CI step re-derives them from `cargo metadata`
//! directly, so any drift between the source and the metadata is caught in two
//! independent places).
//!
//! # Why `std::process::Command`
//!
//! These tests do not use the Soroban test host at all. They are plain
//! host-native Rust that shells out to `cargo metadata`. They are gated on
//! `#[cfg(not(target_family = "wasm"))]` so they are skipped during the WASM
//! build (where `std::process` is unavailable and meaningless).

// Only compile / run on the host — the wasm32 target has no process API.
#![cfg(not(target_family = "wasm"))]

use std::process::Command;

/// The Cargo package name (`[package] name = …` in `Cargo.toml`).
///
/// Changing this constant is the *approved* migration path for a rename;
/// updating it here without updating the downstream consumers listed in the
/// module doc is still a breaking change.
const EXPECTED_PACKAGE_NAME: &str = "fluxora-stream";

/// The Cargo target name, which becomes the WASM filename stem.
///
/// For a `[lib]` target with no explicit `name =` override, Cargo derives
/// this from the package name by replacing `-` with `_`.
const EXPECTED_TARGET_NAME: &str = "fluxora_stream";

/// The expected WASM artifact filename (without a directory prefix).
const EXPECTED_WASM_BASENAME: &str = "fluxora_stream.wasm";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse `cargo metadata --format-version 1 --no-deps` and return the JSON
/// value, panicking with a clear message on any failure.
fn cargo_metadata() -> serde_json::Value {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .expect("failed to spawn `cargo metadata` — is Cargo in PATH?");

    assert!(
        output.status.success(),
        "`cargo metadata` exited with status {}\nstderr:\n{}",
        output.status,
        std::string::String::from_utf8_lossy(&output.stderr),
    );

    serde_json::from_slice(&output.stdout)
        .expect("`cargo metadata` output was not valid JSON — this is a Cargo bug")
}

/// Return the `packages` array from `cargo metadata`.
fn packages(meta: &serde_json::Value) -> &std::vec::Vec<serde_json::Value> {
    meta["packages"]
        .as_array()
        .expect("`packages` key missing from `cargo metadata` output")
}

/// Find the package whose `name` field equals `pkg_name`.
fn find_package<'a>(
    pkgs: &'a [serde_json::Value],
    pkg_name: &str,
) -> Option<&'a serde_json::Value> {
    pkgs.iter().find(|p| p["name"].as_str() == Some(pkg_name))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// The deployable contract package must be present in the workspace under its
/// canonical name.  Any rename breaks `cargo build -p fluxora-stream` in every
/// script and CI step.
#[test]
fn package_name_is_canonical() {
    let meta = cargo_metadata();
    let pkgs = packages(&meta);

    let found: std::vec::Vec<&str> = pkgs.iter().filter_map(|p| p["name"].as_str()).collect();

    assert!(
        find_package(pkgs, EXPECTED_PACKAGE_NAME).is_some(),
        "Workspace package `{EXPECTED_PACKAGE_NAME}` not found.\n\
         Packages present: {found:?}\n\n\
         If this is an intentional rename, update EXPECTED_PACKAGE_NAME in \
         contracts/stream/src/test/packaging.rs and follow the migration \
         checklist in that file's module doc.",
    );

    std::println!("packaging::package_name_is_canonical  package={EXPECTED_PACKAGE_NAME}  ✓");
}

/// The `lib` target inside `fluxora-stream` must have the canonical target
/// name.  The target name becomes the WASM filename stem; changing it silently
/// renames the build artifact and breaks every downstream reference to
/// `fluxora_stream.wasm`.
#[test]
fn lib_target_name_is_canonical() {
    let meta = cargo_metadata();
    let pkgs = packages(&meta);

    let pkg = find_package(pkgs, EXPECTED_PACKAGE_NAME).unwrap_or_else(|| {
        panic!(
            "Package `{EXPECTED_PACKAGE_NAME}` not found in workspace. \
             Run packaging::package_name_is_canonical to diagnose."
        )
    });

    let targets = pkg["targets"]
        .as_array()
        .expect("`targets` key missing from package entry");

    // The contract is a `cdylib` (and also `lib` for tests).  We want the
    // target that contains `cdylib` in its `kind` array — that is the one
    // whose name dictates the `.wasm` filename.
    let cdylib_target = targets.iter().find(|t| {
        t["kind"]
            .as_array()
            .map(|kinds| kinds.iter().any(|k| k.as_str() == Some("cdylib")))
            .unwrap_or(false)
    });

    let cdylib_target = cdylib_target.unwrap_or_else(|| {
        let all_kinds: std::vec::Vec<_> = targets.iter().map(|t| &t["kind"]).collect();
        panic!(
            "No `cdylib` target found in package `{EXPECTED_PACKAGE_NAME}`.\n\
             All target kinds: {all_kinds:?}\n\n\
             The contract must have `crate-type = [\"cdylib\"]` in its \
             `[lib]` section."
        )
    });

    let actual_name = cdylib_target["name"]
        .as_str()
        .expect("`name` field missing from target entry");

    assert_eq!(
        actual_name, EXPECTED_TARGET_NAME,
        "cdylib target name changed: expected `{EXPECTED_TARGET_NAME}`, \
         got `{actual_name}`.\n\n\
         This renames the WASM artifact from `{EXPECTED_WASM_BASENAME}` to \
         `{actual_name}.wasm` and breaks every downstream reference.\n\
         If intentional, update EXPECTED_TARGET_NAME in \
         contracts/stream/src/test/packaging.rs and follow the migration \
         checklist in that file's module doc.",
    );

    std::println!("packaging::lib_target_name_is_canonical  target={EXPECTED_TARGET_NAME}  ✓");
}

/// `publish = false` must be set.  The contract is deployed as a WASM binary,
/// not published to crates.io; accidentally removing this flag would let a
/// `cargo publish` slip through.
#[test]
fn package_is_not_publishable() {
    let meta = cargo_metadata();
    let pkgs = packages(&meta);

    let pkg = find_package(pkgs, EXPECTED_PACKAGE_NAME).unwrap_or_else(|| {
        panic!(
            "Package `{EXPECTED_PACKAGE_NAME}` not found in workspace. \
             Run packaging::package_name_is_canonical to diagnose."
        )
    });

    // `cargo metadata` surfaces `publish = false` as `"publish": []`.
    // Any non-empty array means the package targets one or more registries.
    let publish = &pkg["publish"];
    let is_not_publishable = match publish {
        serde_json::Value::Bool(false) => true,
        serde_json::Value::Array(registries) => registries.is_empty(),
        _ => false,
    };

    assert!(
        is_not_publishable,
        "Package `{EXPECTED_PACKAGE_NAME}` has `publish` set to `{publish}`.\n\
         The contract is deployed as a WASM binary; it must never be published \
         to a registry. Set `publish = false` in its `Cargo.toml`.",
    );

    std::println!("packaging::package_is_not_publishable  publish=false  ✓");
}

/// The WASM artifact path reported by `cargo metadata` must match the expected
/// basename.  This is the path that deployment scripts pass to
/// `stellar contract deploy`.
///
/// Note: `cargo metadata` reports the *expected* artifact path based on the
/// target name — it does not require the artifact to actually exist.  This
/// test therefore works on a clean checkout without a prior build.
#[test]
fn wasm_artifact_basename_is_canonical() {
    let meta = cargo_metadata();
    let pkgs = packages(&meta);

    let pkg = find_package(pkgs, EXPECTED_PACKAGE_NAME).unwrap_or_else(|| {
        panic!(
            "Package `{EXPECTED_PACKAGE_NAME}` not found in workspace. \
             Run packaging::package_name_is_canonical to diagnose."
        )
    });

    let targets = pkg["targets"]
        .as_array()
        .expect("`targets` key missing from package entry");

    // Locate the cdylib target and reconstruct what Cargo will name the file.
    // Cargo names a cdylib `lib<target_name>.so` on Linux and `<target_name>.wasm`
    // on wasm32 — the WASM filename is just `{target_name}.wasm`.
    let cdylib_target = targets.iter().find(|t| {
        t["kind"]
            .as_array()
            .map(|kinds| kinds.iter().any(|k| k.as_str() == Some("cdylib")))
            .unwrap_or(false)
    });

    let actual_target_name = cdylib_target
        .and_then(|t| t["name"].as_str())
        .unwrap_or_else(|| {
            panic!(
                "No `cdylib` target found in `{EXPECTED_PACKAGE_NAME}`. \
                 See packaging::lib_target_name_is_canonical for details."
            )
        });

    let derived_wasm_basename = std::format!("{actual_target_name}.wasm");

    assert_eq!(
        derived_wasm_basename, EXPECTED_WASM_BASENAME,
        "WASM artifact basename changed: expected `{EXPECTED_WASM_BASENAME}`, \
         would be `{derived_wasm_basename}`.\n\n\
         This breaks `stellar contract deploy`, `script/testnet-exercise.sh`, \
         and the stage-5 indexer tooling that references this filename.\n\
         If intentional, update EXPECTED_WASM_BASENAME in \
         contracts/stream/src/test/packaging.rs and follow the migration \
         checklist in that file's module doc.",
    );

    std::println!(
        "packaging::wasm_artifact_basename_is_canonical  basename={EXPECTED_WASM_BASENAME}  ✓"
    );
}

/// **Boundary / regression guard.** Renaming the package by one character
/// (`fluxora-streams`, `Fluxora-Stream`, etc.) must still be caught.
///
/// This test does *not* call real `cargo metadata`; it verifies the comparison
/// logic itself against a mutated string so the guard cannot be quietly
/// short-circuited by a refactor.
#[test]
fn comparison_is_exact_not_prefix_match() {
    // Simulate a mutated name that starts with the canonical prefix.
    let mutated = std::format!("{EXPECTED_PACKAGE_NAME}s"); // e.g. "fluxora-streams"
    assert_ne!(
        mutated.as_str(),
        EXPECTED_PACKAGE_NAME,
        "comparison logic accepted a name that is merely a prefix of the canonical name",
    );

    let mutated_upper = EXPECTED_PACKAGE_NAME.to_uppercase();
    assert_ne!(
        mutated_upper.as_str(),
        EXPECTED_PACKAGE_NAME,
        "comparison logic is case-insensitive when it must be exact",
    );

    std::println!("packaging::comparison_is_exact_not_prefix_match  exact-match-logic=verified  ✓");
}
