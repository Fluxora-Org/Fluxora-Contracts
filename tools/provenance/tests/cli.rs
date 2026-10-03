//! End-to-end tests for the `fluxora-provenance` binary.
//!
//! The library-level behaviour is covered by the unit tests in `src/lib.rs`.
//! What is only observable through the process boundary is asserted here:
//! argument parsing, the documented exit-code contract (0 success, 1 gate
//! failure, 2 usage error), the documented `generate`/`verify` messages, and
//! the generate -> tamper -> verify flow a release engineer actually runs.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Absolute path to the binary under test, provided by cargo for integration
/// tests. Spawning it is the only way to exercise `src/main.rs`.
const BIN: &str = env!("CARGO_BIN_EXE_fluxora-provenance");

const WORKSPACE_TOML: &str = r#"
[workspace]
resolver = "2"
members = ["contracts/*"]

[profile.release]
opt-level = "z"
lto = true
"#;

const LOCK_TOML: &str = "[[package]]\nname = \"soroban-sdk\"\nversion = \"27.0.5\"\n";

const TOOLCHAIN_TOML: &str = "[toolchain]\nchannel = \"1.97.1\"\n";

const WASM_BYTES: &[u8] = b"\x00asm\x01\x00\x00\x00fluxora-sample-contract-bytes";

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .expect("git must be available to run provenance tests");
    assert!(status.success(), "git {args:?} failed");
}

/// A throwaway workspace root with a committed baseline and a release dir, so
/// `generate` can discover the root and read a stable revision.
fn scratch() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().join("workspace");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("Cargo.toml"), WORKSPACE_TOML).unwrap();
    fs::write(root.join("Cargo.lock"), LOCK_TOML).unwrap();
    fs::write(root.join("rust-toolchain.toml"), TOOLCHAIN_TOML).unwrap();
    git(&root, &["init", "-q"]);
    git(&root, &["config", "user.email", "test@example.com"]);
    git(&root, &["config", "user.name", "Test"]);
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-qm", "init"]);

    let release = root.join("target").join("wasm32v1-none").join("release");
    fs::create_dir_all(&release).unwrap();
    (tmp, root, release)
}

fn run(args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .output()
        .expect("spawn fluxora-provenance")
}

fn code(out: &Output) -> i32 {
    out.status
        .code()
        .expect("the tool always exits with a code")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn help_prints_usage_on_stdout_and_exits_zero() {
    for args in [&["help"][..], &["--help"][..], &["-h"][..]] {
        let out = run(args);
        assert_eq!(code(&out), 0, "args {args:?}");
        let text = stdout(&out);
        assert!(text.contains("Usage:"), "args {args:?}");
        assert!(
            text.contains("fluxora-provenance generate"),
            "args {args:?}"
        );
        assert!(text.contains("verify"), "args {args:?}");
        assert!(stderr(&out).is_empty(), "args {args:?}");
    }
}

#[test]
fn a_missing_command_is_a_usage_error() {
    let out = run(&[]);
    assert_eq!(code(&out), 2);
    assert!(stderr(&out).contains("Usage:"));
    assert!(stdout(&out).is_empty());
}

#[test]
fn an_unknown_command_is_a_usage_error() {
    let out = run(&["frobnicate"]);
    assert_eq!(code(&out), 2);
    assert!(stderr(&out).contains("unknown command 'frobnicate'"));
}

#[test]
fn bad_options_are_usage_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("release");
    let dir = dir.to_str().unwrap();

    let unknown = run(&["generate", dir, "--bogus"]);
    assert_eq!(code(&unknown), 2);
    assert!(stderr(&unknown).contains("unknown option '--bogus'"));

    let no_value = run(&["generate", dir, "--target"]);
    assert_eq!(code(&no_value), 2);
    assert!(stderr(&no_value).contains("option '--target' requires a value"));

    let missing_dir = run(&["generate", "--target", "wasm32v1-none"]);
    assert_eq!(code(&missing_dir), 2);
    assert!(stderr(&missing_dir).contains("missing <release-dir>"));

    let extra_positional = run(&["generate", dir, "extra"]);
    assert_eq!(code(&extra_positional), 2);
    assert!(stderr(&extra_positional).contains("unexpected argument 'extra'"));
}

#[test]
fn generate_then_verify_round_trips_through_the_binary() {
    let (_tmp, _root, release) = scratch();
    fs::write(release.join("fluxora_stream.wasm"), WASM_BYTES).unwrap();
    let dir = release.to_str().unwrap();

    let generated = run(&["generate", dir]);
    assert_eq!(code(&generated), 0, "stderr: {}", stderr(&generated));
    assert!(stdout(&generated).contains("wrote https://slsa.dev/provenance/v1.0 for 1 artifact(s)"));
    assert!(release.join("provenance.json").is_file());
    assert!(release.join("SHASUMS").is_file());

    let verified = run(&["verify", dir]);
    assert_eq!(code(&verified), 0, "stderr: {}", stderr(&verified));
    assert!(stdout(&verified).contains("verified 1 artifact(s) against the manifest"));
}

#[test]
fn verify_detects_a_tampered_artifact_and_exits_one() {
    let (_tmp, _root, release) = scratch();
    fs::write(release.join("fluxora_stream.wasm"), WASM_BYTES).unwrap();
    let dir = release.to_str().unwrap();
    assert_eq!(code(&run(&["generate", dir])), 0);

    let mut tampered = WASM_BYTES.to_vec();
    tampered[10] ^= 0xff;
    fs::write(release.join("fluxora_stream.wasm"), &tampered).unwrap();

    let out = run(&["verify", dir]);
    assert_eq!(code(&out), 1);
    let err = stderr(&out);
    assert!(err.contains("FAILED"));
    assert!(err.contains("sha256 mismatch for fluxora_stream.wasm"));
    // Both digests are printed, so the failure is actionable from the log.
    assert!(err.contains("!= on-disk"));
}

#[test]
fn generate_on_an_empty_release_dir_exits_one() {
    let (_tmp, _root, release) = scratch();
    let out = run(&["generate", release.to_str().unwrap()]);
    assert_eq!(code(&out), 1);
    assert!(stderr(&out).contains("no *.wasm artifacts found in"));
    assert!(!release.join("provenance.json").exists());
}

#[test]
fn verify_on_a_missing_release_dir_exits_one() {
    let tmp = tempfile::tempdir().unwrap();
    let missing = tmp.path().join("no-such-release");
    let out = run(&["verify", missing.to_str().unwrap()]);
    assert_eq!(code(&out), 1);
    assert!(stderr(&out).contains("release dir does not exist"));
}

#[test]
fn flags_override_discovery_and_the_default_paths() {
    let (_tmp, root, _release) = scratch();
    // A release dir outside any workspace: discovery cannot resolve a root for
    // it, so these runs also prove `--workspace-root` is honored.
    let detached = root.parent().unwrap().join("elsewhere").join("release");
    fs::create_dir_all(&detached).unwrap();
    fs::write(detached.join("fluxora_stream.wasm"), WASM_BYTES).unwrap();

    let dir = detached.to_str().unwrap();
    let ws = root.to_str().unwrap();
    let custom = root.parent().unwrap().join("release-provenance.json");
    let manifest = custom.to_str().unwrap();

    let generated = run(&[
        "generate",
        dir,
        "--workspace-root",
        ws,
        "--manifest",
        manifest,
        "--target",
        "wasm32-unknown-unknown",
    ]);
    assert_eq!(code(&generated), 0, "stderr: {}", stderr(&generated));
    assert!(custom.is_file());
    // The manifest override is honored; SHASUMS still lands by the artifacts.
    assert!(!detached.join("provenance.json").exists());
    assert!(detached.join("SHASUMS").is_file());

    // `--key=value` is accepted as well as `--key value`.
    let inline_manifest = format!("--manifest={manifest}");
    let inline_target = "--target=wasm32-unknown-unknown";
    let inline = run(&[
        "verify",
        dir,
        "--workspace-root",
        ws,
        &inline_manifest,
        inline_target,
    ]);
    assert_eq!(code(&inline), 0, "stderr: {}", stderr(&inline));

    // Dropping `--target` falls back to the default triple and must fail, since
    // the recorded target no longer matches.
    let drifted = run(&[
        "verify",
        dir,
        "--workspace-root",
        ws,
        "--manifest",
        manifest,
    ]);
    assert_eq!(code(&drifted), 1);
    assert!(stderr(&drifted).contains("target drifted since provenance was generated"));

    // Without `--manifest`, verify looks next to the artifacts and finds nothing.
    let missing_manifest = run(&["verify", dir, "--workspace-root", ws, inline_target]);
    assert_eq!(code(&missing_manifest), 1);
    assert!(stderr(&missing_manifest).contains("provenance manifest not found"));
}
