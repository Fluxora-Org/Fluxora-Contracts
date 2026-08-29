//! Release metadata guard.
//!
//! The production artifact is a narrow thing: `fluxora-stream` built with the
//! workspace `release` profile for `wasm32v1-none`, without test helpers,
//! logging, or debug assertions. These tests keep that contract explicit.

use std::{format, fs, path::Path, string::String};

const WORKSPACE_MANIFEST: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.toml");
const STREAM_MANIFEST: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
const TOOLCHAIN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../rust-toolchain.toml");
const CI_WORKFLOW: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../.github/workflows/ci.yml"
);
const STREAM_SRC: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src");

const RELEASE_COMMAND: &str = "cargo build --target wasm32v1-none --release";

fn read(path: &str) -> String {
    fs::read_to_string(path).unwrap_or_else(|err| panic!("failed to read {path}: {err}"))
}

fn section<'a>(contents: &'a str, header: &str) -> &'a str {
    let start = contents
        .find(header)
        .unwrap_or_else(|| panic!("missing TOML section {header}"));
    let tail = &contents[start + header.len()..];
    let end = tail.find("\n[").unwrap_or(tail.len());
    &tail[..end]
}

fn assert_setting(section: &str, key: &str, value: &str) {
    let expected = format!("{key} = {value}");
    assert!(
        section.lines().any(|line| line.trim() == expected),
        "missing release invariant `{expected}` in:\n{section}",
    );
}

#[test]
fn workspace_release_profile_is_the_production_profile() {
    let manifest = read(WORKSPACE_MANIFEST);
    let release = section(&manifest, "[profile.release]");

    assert_setting(release, "opt-level", "\"z\"");
    assert_setting(release, "overflow-checks", "true");
    assert_setting(release, "debug", "0");
    assert_setting(release, "strip", "\"symbols\"");
    assert_setting(release, "debug-assertions", "false");
    assert_setting(release, "panic", "\"abort\"");
    assert_setting(release, "codegen-units", "1");
    assert_setting(release, "lto", "true");

    let with_logs = section(&manifest, "[profile.release-with-logs]");
    assert_setting(with_logs, "inherits", "\"release\"");
    assert_setting(with_logs, "debug-assertions", "true");
}

#[test]
fn production_wasm_target_is_wasm32v1_none() {
    let toolchain = read(TOOLCHAIN);

    assert!(
        toolchain.contains("targets = [\"wasm32v1-none\"]"),
        "rust-toolchain.toml must install exactly the production Soroban target",
    );
    assert!(
        !toolchain.contains("targets = [\"wasm32-unknown-unknown\"]"),
        "wasm32-unknown-unknown must not become the production target",
    );

    let ci = read(CI_WORKFLOW);
    assert!(
        ci.contains(RELEASE_COMMAND),
        "CI must build the authoritative release artifact with `{RELEASE_COMMAND}`",
    );
}

#[test]
fn production_build_does_not_enable_test_features() {
    let manifest = read(STREAM_MANIFEST);
    let features = section(&manifest, "[features]");
    let dependencies = section(&manifest, "[dependencies]");
    let dev_dependencies = section(&manifest, "[dev-dependencies]");

    assert!(
        !features
            .lines()
            .any(|line| line.trim().starts_with("default")),
        "default features must stay empty so production builds cannot inherit test helpers",
    );
    assert!(
        features.contains("testutils = [\"soroban-sdk/testutils\"]"),
        "testutils must remain an explicit opt-in feature",
    );
    assert!(
        !dependencies.contains("testutils"),
        "production dependencies must not enable soroban-sdk/testutils",
    );
    assert!(
        dev_dependencies.contains("features = [\"testutils\"]"),
        "testutils belongs only in dev-dependencies",
    );
}

#[test]
fn wasm_compile_guards_reject_bad_artifact_flags() {
    let lib = read(&format!("{STREAM_SRC}/lib.rs"));

    assert!(
        lib.contains(r#"#[cfg(all(target_family = "wasm", not(target_os = "none")))]"#),
        "wasm builds must reject non-Soroban wasm targets",
    );
    assert!(
        lib.contains(r#"#[cfg(all(target_family = "wasm", debug_assertions))]"#),
        "wasm builds must reject debug assertions",
    );
    assert!(
        lib.contains(r#"#[cfg(all(target_family = "wasm", feature = "testutils"))]"#),
        "wasm builds must reject test-only features",
    );
}

#[test]
fn production_contract_source_contains_no_logging_macros() {
    let production_files = [
        "accrual.rs",
        "error.rs",
        "events.rs",
        "lib.rs",
        "storage.rs",
        "types.rs",
    ];
    let forbidden = ["log!(", "println!(", "eprintln!("];

    for file in production_files {
        let path = Path::new(STREAM_SRC).join(file);
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));

        for needle in forbidden {
            assert!(
                !source.contains(needle),
                "production source {} contains forbidden logging macro `{needle}`",
                path.display(),
            );
        }
    }
}
