//! Provenance generation and verification for Fluxora contract wasm artifacts.
//!
//! Every deployable wasm the workspace produces gets a machine-readable
//! manifest tying its bytes to the exact inputs of the build — git revision,
//! Rust toolchain, soroban-sdk version, target triple and release profile
//! flags — plus a SHA-256 digest per artifact, and a `SHASUMS` file in
//! `sha256sum` format for direct consumption.
//!
//! `verify` re-hashes the artifacts and re-reads the environment, and fails
//! the release if anything drifted: a byte changed, an artifact appeared or
//! disappeared, or the recorded build inputs no longer match the current
//! checkout and toolchain. See `docs/provenance.md` for the design decisions
//! and the failure contract.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const SCHEMA: &str = "https://slsa.dev/provenance/v1.0";
pub const BUILD_TYPE: &str = "https://github.com/Fluxora-Org/Fluxora-Contracts/provenance/v1";
pub const DEFAULT_TARGET: &str = "wasm32v1-none";
pub const MANIFEST_FILENAME: &str = "provenance.json";
pub const SHASUMS_FILENAME: &str = "SHASUMS";

// ---------------------------------------------------------------------------
// Manifest schema
// ---------------------------------------------------------------------------

/// One released artifact and its digest. `name` is the wasm file name within
/// the release dir; `sha256` is the hex SHA-256 of its bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subject {
    pub name: String,
    pub sha256: String,
}

/// The `[profile.release]` table of the workspace `Cargo.toml`, preserved
/// verbatim so release flags cannot drift from what the manifest claims.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    #[serde(flatten)]
    pub values: BTreeMap<String, serde_json::Value>,
}

/// Everything that identifies the build the artifacts came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildInfo {
    pub build_type: String,
    pub target: String,
    pub profile: Profile,
    pub git_revision: String,
    pub git_ref: Option<String>,
    pub git_dirty: bool,
    pub toolchain_channel: Option<String>,
    pub rustc: String,
    pub cargo: String,
    pub soroban_sdk: String,
    pub host: String,
    pub started_on: String,
}

/// The provenance manifest. Follows SLSA v1.0 conventions (`subject` digests
/// plus a build definition) without claiming full SLSA attestation compliance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub schema: String,
    pub subject: Vec<Subject>,
    pub build: BuildInfo,
}

// ---------------------------------------------------------------------------
// Errors — every failure mode is explicit and typed
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum Error {
    ReleaseDirMissing(PathBuf),
    NotADirectory(PathBuf),
    NoWasmArtifacts(PathBuf),
    WorkspaceRootNotFound(PathBuf),
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Utf8(std::string::FromUtf8Error),
    Toml(String),
    Json(serde_json::Error),
    Git(String),
    ToolVersion(String, String),
    HostParse(String),
    SdkVersionNotFound,
    ManifestMissing(PathBuf),
    MissingArtifact {
        name: String,
    },
    HashMismatch {
        name: String,
        expected: String,
        actual: String,
    },
    UnlistedArtifact {
        name: String,
    },
    MetadataMismatch {
        field: &'static str,
        recorded: String,
        actual: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::ReleaseDirMissing(p) => {
                write!(f, "release dir does not exist: {}", p.display())
            }
            Error::NotADirectory(p) => write!(f, "release path is not a directory: {}", p.display()),
            Error::NoWasmArtifacts(p) => {
                write!(f, "no *.wasm artifacts found in {}", p.display())
            }
            Error::WorkspaceRootNotFound(p) => write!(
                f,
                "could not find a workspace Cargo.toml (with [workspace]) walking up from {}",
                p.display()
            ),
            Error::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Error::Utf8(e) => write!(f, "command output was not valid UTF-8: {e}"),
            Error::Toml(msg) => write!(f, "TOML parse error: {msg}"),
            Error::Json(e) => write!(f, "JSON error: {e}"),
            Error::Git(msg) => write!(f, "git: {msg}"),
            Error::ToolVersion(tool, msg) => write!(f, "{tool} --version failed: {msg}"),
            Error::HostParse(out) => {
                write!(f, "could not read the host triple from `rustc -vV`: {out:?}")
            }
            Error::SdkVersionNotFound => write!(f, "soroban-sdk not found in Cargo.lock"),
            Error::ManifestMissing(p) => {
                write!(f, "provenance manifest not found: {}", p.display())
            }
            Error::MissingArtifact { name } => write!(
                f,
                "artifact listed in the manifest is missing from the release dir: {name}"
            ),
            Error::HashMismatch {
                name,
                expected,
                actual,
            } => write!(
                f,
                "sha256 mismatch for {name}: manifest {expected} != on-disk {actual} \
                 (rebuild and regenerate, or investigate before releasing)"
            ),
            Error::UnlistedArtifact { name } => write!(
                f,
                "artifact present in the release dir is not listed in the manifest: {name} \
                 (regenerate provenance so every contract is covered)"
            ),
            Error::MetadataMismatch {
                field,
                recorded,
                actual,
            } => write!(
                f,
                "{field} drifted since provenance was generated: manifest {recorded} != current {actual}"
            ),
        }
    }
}

impl std::error::Error for Error {}

// ---------------------------------------------------------------------------
// Small utilities
// ---------------------------------------------------------------------------

pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

fn read_to_string(path: &Path) -> Result<String, Error> {
    std::fs::read_to_string(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn hash_file(path: &Path) -> Result<String, Error> {
    let bytes = std::fs::read(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(sha256_hex(&bytes))
}

/// Write `contents` to a temp file in the same dir, then rename over `path`,
/// so a crash mid-write never leaves a half-written manifest that a release
/// gate could mistake for a valid one.
fn write_atomic(path: &Path, contents: &str) -> Result<(), Error> {
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("out")
        .to_string();
    let tmp = path.with_file_name(format!(".{file_name}.tmp"));
    std::fs::write(&tmp, contents).map_err(|source| Error::Io {
        path: tmp.clone(),
        source,
    })?;
    std::fs::rename(&tmp, path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(())
}

/// Every `*.wasm` file directly inside `dir`, sorted by name so the manifest
/// is deterministic.
pub fn list_wasm_files(dir: &Path) -> Result<Vec<PathBuf>, Error> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(|source| Error::Io {
        path: dir.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| Error::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|e| e == "wasm") {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

fn validate_release_dir(dir: &Path) -> Result<(), Error> {
    if !dir.exists() {
        return Err(Error::ReleaseDirMissing(dir.to_path_buf()));
    }
    if !dir.is_dir() {
        return Err(Error::NotADirectory(dir.to_path_buf()));
    }
    Ok(())
}

fn run_git(args: &[&str], cwd: &Path) -> Result<String, Error> {
    let label = format!("git {}", args.join(" "));
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|e| Error::Git(format!("could not run `{label}`: {e}")))?;
    if !out.status.success() {
        return Err(Error::Git(format!("`{label}` exited with {}", out.status)));
    }
    Ok(String::from_utf8(out.stdout)
        .map_err(Error::Utf8)?
        .trim()
        .to_string())
}

fn tool_version(tool: &str) -> Result<String, Error> {
    let out = Command::new(tool)
        .arg("--version")
        .output()
        .map_err(|e| Error::ToolVersion(tool.to_string(), e.to_string()))?;
    if !out.status.success() {
        return Err(Error::ToolVersion(
            tool.to_string(),
            format!("exit {}", out.status),
        ));
    }
    Ok(String::from_utf8(out.stdout)
        .map_err(Error::Utf8)?
        .trim()
        .to_string())
}

fn host_triple() -> Result<String, Error> {
    let out = Command::new("rustc")
        .args(["-vV"])
        .output()
        .map_err(|e| Error::ToolVersion("rustc -vV".to_string(), e.to_string()))?;
    let stdout = String::from_utf8(out.stdout).map_err(Error::Utf8)?;
    for line in stdout.lines() {
        if let Some(v) = line.strip_prefix("host: ") {
            return Ok(v.trim().to_string());
        }
    }
    Err(Error::HostParse(stdout))
}

// ---------------------------------------------------------------------------
// Build-input discovery (workspace root, profile, sdk, toolchain, git)
// ---------------------------------------------------------------------------

/// Walk up from `from` to the nearest directory whose `Cargo.toml` declares a
/// `[workspace]` table.
pub fn find_workspace_root(from: &Path) -> Result<PathBuf, Error> {
    // Canonicalize so a relative path like `target/wasm32v1-none/release`
    // walks up to an absolute root instead of exhausting to an empty path.
    let mut dir = from.canonicalize().map_err(|source| Error::Io {
        path: from.to_path_buf(),
        source,
    })?;
    loop {
        let manifest = dir.join("Cargo.toml");
        if manifest.is_file() {
            if let Ok(text) = read_to_string(&manifest) {
                if text
                    .parse::<toml::Value>()
                    .is_ok_and(|doc| doc.get("workspace").is_some())
                {
                    return Ok(dir);
                }
            }
        }
        if !dir.pop() {
            return Err(Error::WorkspaceRootNotFound(from.to_path_buf()));
        }
    }
}

fn load_toml(root: &Path, file: &str) -> Result<toml::Value, Error> {
    let text = read_to_string(&root.join(file))?;
    text.parse::<toml::Value>()
        .map_err(|e| Error::Toml(e.to_string()))
}

/// The `[profile.release]` table of the workspace `Cargo.toml`, as JSON so it
/// can be recorded and diffed verbatim.
fn profile_from_workspace(root: &Path) -> Result<Profile, Error> {
    let doc = load_toml(root, "Cargo.toml")?;
    let mut values = BTreeMap::new();
    if let Some(release) = doc
        .get("profile")
        .and_then(|p| p.get("release"))
        .and_then(|r| r.as_table())
    {
        for (key, value) in release {
            values.insert(
                key.clone(),
                serde_json::to_value(value).map_err(Error::Json)?,
            );
        }
    }
    Ok(Profile { values })
}

fn soroban_sdk_version(root: &Path) -> Result<String, Error> {
    let doc = load_toml(root, "Cargo.lock")?;
    let packages = doc
        .get("package")
        .and_then(|p| p.as_array())
        .ok_or(Error::SdkVersionNotFound)?;
    for pkg in packages {
        if pkg.get("name").and_then(|n| n.as_str()) == Some("soroban-sdk") {
            return pkg
                .get("version")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .ok_or(Error::SdkVersionNotFound);
        }
    }
    Err(Error::SdkVersionNotFound)
}

fn toolchain_channel(root: &Path) -> Result<Option<String>, Error> {
    let path = root.join("rust-toolchain.toml");
    if !path.is_file() {
        return Ok(None);
    }
    let doc = load_toml(root, "rust-toolchain.toml")?;
    Ok(doc
        .get("toolchain")
        .and_then(|t| t.get("channel"))
        .and_then(|c| c.as_str())
        .map(str::to_string))
}

fn git_metadata(root: &Path) -> Result<(String, Option<String>, bool), Error> {
    let revision = run_git(&["rev-parse", "HEAD"], root)?;
    // `branch --show-current` is empty (and exits 0) on a detached HEAD, which
    // is the norm in CI checkouts — a ref is context, not identity.
    let reference = run_git(&["branch", "--show-current"], root)
        .ok()
        .filter(|r| !r.is_empty());
    let status = run_git(&["status", "--porcelain"], root)?;
    Ok((revision, reference, !status.is_empty()))
}

#[derive(Debug)]
struct BuildMetadata {
    git_revision: String,
    git_ref: Option<String>,
    git_dirty: bool,
    toolchain_channel: Option<String>,
    rustc: String,
    cargo: String,
    soroban_sdk: String,
    host: String,
    profile: Profile,
}

fn collect_metadata(root: &Path) -> Result<BuildMetadata, Error> {
    let (git_revision, git_ref, git_dirty) = git_metadata(root)?;
    Ok(BuildMetadata {
        git_revision,
        git_ref,
        git_dirty,
        toolchain_channel: toolchain_channel(root)?,
        rustc: tool_version("rustc")?,
        cargo: tool_version("cargo")?,
        soroban_sdk: soroban_sdk_version(root)?,
        host: host_triple()?,
        profile: profile_from_workspace(root)?,
    })
}

// ---------------------------------------------------------------------------
// generate / verify
// ---------------------------------------------------------------------------

fn collect_artifacts(dir: &Path) -> Result<Vec<Subject>, Error> {
    let files = list_wasm_files(dir)?;
    if files.is_empty() {
        return Err(Error::NoWasmArtifacts(dir.to_path_buf()));
    }
    let mut subjects = Vec::with_capacity(files.len());
    for path in files {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        subjects.push(Subject {
            name,
            sha256: hash_file(&path)?,
        });
    }
    Ok(subjects)
}

fn shasums_text(subjects: &[Subject]) -> String {
    let mut out = String::new();
    for s in subjects {
        out.push_str(&format!("{hash}  {name}\n", hash = s.sha256, name = s.name));
    }
    out
}

/// Hash every `*.wasm` in `release_dir` and write `provenance.json` plus
/// `SHASUMS` next to the artifacts. Overwrites any previous manifest, so
/// re-running after a rebuild is the recovery path.
pub fn generate(
    release_dir: &Path,
    manifest_path: Option<&Path>,
    workspace_root: Option<&Path>,
    target: &str,
) -> Result<Manifest, Error> {
    validate_release_dir(release_dir)?;
    let root = match workspace_root {
        Some(r) => r.to_path_buf(),
        None => find_workspace_root(release_dir)?,
    };
    let artifacts = collect_artifacts(release_dir)?;
    let meta = collect_metadata(&root)?;

    if meta.git_dirty {
        eprintln!(
            "fluxora-provenance: warning: working tree is dirty — provenance records \
             uncommitted changes"
        );
    }

    let manifest = Manifest {
        schema: SCHEMA.to_string(),
        subject: artifacts.clone(),
        build: BuildInfo {
            build_type: BUILD_TYPE.to_string(),
            target: target.to_string(),
            profile: meta.profile,
            git_revision: meta.git_revision,
            git_ref: meta.git_ref,
            git_dirty: meta.git_dirty,
            toolchain_channel: meta.toolchain_channel,
            rustc: meta.rustc,
            cargo: meta.cargo,
            soroban_sdk: meta.soroban_sdk,
            host: meta.host,
            started_on: now_rfc3339(),
        },
    };

    let json = serde_json::to_string_pretty(&manifest).map_err(Error::Json)?;
    let default_path = release_dir.join(MANIFEST_FILENAME);
    let path = manifest_path.unwrap_or(&default_path);
    write_atomic(path, &format!("{json}\n"))?;
    write_atomic(
        &release_dir.join(SHASUMS_FILENAME),
        &shasums_text(&artifacts),
    )?;
    Ok(manifest)
}

fn check_field<T: PartialEq + fmt::Display>(
    field: &'static str,
    recorded: &T,
    actual: &T,
) -> Result<(), Error> {
    if recorded == actual {
        Ok(())
    } else {
        Err(Error::MetadataMismatch {
            field,
            recorded: recorded.to_string(),
            actual: actual.to_string(),
        })
    }
}

fn opt_string(value: &Option<String>) -> String {
    value.clone().unwrap_or_else(|| "<none>".to_string())
}

/// The release gate. Returns the number of artifacts verified and fails on:
/// any recorded artifact missing or with a different hash, any wasm in the
/// dir not covered by the manifest, or any recorded build input (target, git
/// revision, toolchain, sdk, profile) that no longer matches the environment.
pub fn verify(
    release_dir: &Path,
    manifest_path: Option<&Path>,
    workspace_root: Option<&Path>,
    target: &str,
) -> Result<usize, Error> {
    validate_release_dir(release_dir)?;
    let default_path = release_dir.join(MANIFEST_FILENAME);
    let manifest_path = manifest_path.unwrap_or(&default_path);
    if !manifest_path.is_file() {
        return Err(Error::ManifestMissing(manifest_path.to_path_buf()));
    }
    let text = read_to_string(manifest_path)?;
    let manifest: Manifest = serde_json::from_str(&text).map_err(Error::Json)?;

    // Integrity — every recorded artifact must exist with the recorded hash.
    let mut current = BTreeMap::new();
    for path in list_wasm_files(release_dir)? {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        current.insert(name, hash_file(&path)?);
    }
    for subject in &manifest.subject {
        let Some(actual) = current.get(&subject.name) else {
            return Err(Error::MissingArtifact {
                name: subject.name.clone(),
            });
        };
        if actual != &subject.sha256 {
            return Err(Error::HashMismatch {
                name: subject.name.clone(),
                expected: subject.sha256.clone(),
                actual: actual.clone(),
            });
        }
    }

    // Completeness — every wasm in the release dir must be covered.
    for name in current.keys() {
        if !manifest.subject.iter().any(|s| &s.name == name) {
            return Err(Error::UnlistedArtifact { name: name.clone() });
        }
    }

    // Environment consistency — the recorded build inputs must not have
    // drifted from the checkout and toolchain that now hold the artifacts.
    let root = match workspace_root {
        Some(r) => r.to_path_buf(),
        None => find_workspace_root(release_dir)?,
    };
    let meta = collect_metadata(&root)?;

    check_field("target", &manifest.build.target, &target.to_string())?;
    check_field(
        "git_revision",
        &manifest.build.git_revision,
        &meta.git_revision,
    )?;
    check_field("rustc", &manifest.build.rustc, &meta.rustc)?;
    check_field("cargo", &manifest.build.cargo, &meta.cargo)?;
    check_field(
        "soroban_sdk",
        &manifest.build.soroban_sdk,
        &meta.soroban_sdk,
    )?;
    check_field(
        "toolchain_channel",
        &opt_string(&manifest.build.toolchain_channel),
        &opt_string(&meta.toolchain_channel),
    )?;
    let recorded_profile = serde_json::to_string(&manifest.build.profile).map_err(Error::Json)?;
    let current_profile = serde_json::to_string(&meta.profile).map_err(Error::Json)?;
    check_field("profile", &recorded_profile, &current_profile)?;

    if let (Some(recorded), Some(current_ref)) = (&manifest.build.git_ref, &meta.git_ref) {
        if recorded != current_ref {
            eprintln!(
                "fluxora-provenance: warning: git ref drifted ({recorded} -> {current_ref}); \
                 the revision is still pinned"
            );
        }
    }
    if manifest.build.git_dirty {
        eprintln!("fluxora-provenance: warning: provenance records a dirty working tree");
    }

    Ok(manifest.subject.len())
}

// ---------------------------------------------------------------------------
// UTC timestamp, without pulling in a date crate
// ---------------------------------------------------------------------------

pub fn now_rfc3339() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_unix(secs)
}

fn format_unix(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let sod = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z",
        hh = sod / 3_600,
        mm = (sod % 3_600) / 60,
        ss = sod % 60,
    )
}

/// Howard Hinnant's `civil_from_days`: days since 1970-01-01 -> (y, m, d).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;

    const FAKE_WORKSPACE: &str = r#"
[workspace]
resolver = "2"
members = ["contracts/*"]

[workspace.package]
version = "1.0.0"
edition = "2021"

[profile.release]
opt-level = "z"
overflow-checks = true
debug = 0
strip = "symbols"
debug-assertions = false
panic = "abort"
codegen-units = 1
lto = true
"#;

    const FAKE_LOCK: &str = r#"
[[package]]
name = "soroban-sdk"
version = "27.0.5"
"#;

    const FAKE_TOOLCHAIN: &str = r#"
[toolchain]
channel = "1.97.1"
targets = ["wasm32v1-none"]
"#;

    struct Scratch {
        _tmp: tempfile::TempDir,
        root: PathBuf,
        release: PathBuf,
    }

    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(dir)
            .status()
            .expect("git must be available to run provenance tests");
        assert!(status.success(), "git {args:?} failed");
    }

    fn scratch_repo() -> Scratch {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().to_path_buf();
        fs::write(root.join("Cargo.toml"), FAKE_WORKSPACE).unwrap();
        fs::write(root.join("Cargo.lock"), FAKE_LOCK).unwrap();
        fs::write(root.join("rust-toolchain.toml"), FAKE_TOOLCHAIN).unwrap();
        git(&root, &["init", "-q"]);
        git(&root, &["config", "user.email", "test@example.com"]);
        git(&root, &["config", "user.name", "Test"]);
        git(&root, &["add", "-A"]);
        git(&root, &["commit", "-qm", "init"]);
        let release = root.join("target").join("wasm32v1-none").join("release");
        fs::create_dir_all(&release).unwrap();
        Scratch {
            _tmp: tmp,
            root,
            release,
        }
    }

    fn write_wasm(release: &Path, name: &str, content: &[u8]) {
        fs::write(release.join(name), content).unwrap();
    }

    fn sample_wasm() -> Vec<u8> {
        b"\x00asm\x01\x00\x00\x00fluxora-sample-contract-bytes".to_vec()
    }

    fn head_revision(root: &Path) -> String {
        let out = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(root)
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }

    fn rustc_version() -> String {
        let out = Command::new("rustc").arg("--version").output().unwrap();
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }

    /// Restores the process cwd on drop, even if the test panics, so a
    /// relative-path test cannot poison the other parallel tests.
    struct CwdGuard(PathBuf);

    impl CwdGuard {
        fn set(dir: &Path) -> CwdGuard {
            let prev = std::env::current_dir().expect("current_dir");
            std::env::set_current_dir(dir).expect("set_current_dir");
            CwdGuard(prev)
        }
    }

    impl Drop for CwdGuard {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.0);
        }
    }

    #[test]
    fn generate_accepts_a_relative_release_dir() {
        let s = scratch_repo();
        write_wasm(&s.release, "fluxora_stream.wasm", &sample_wasm());
        let _guard = CwdGuard::set(&s.root);

        let manifest = generate(
            Path::new("target/wasm32v1-none/release"),
            None,
            None,
            DEFAULT_TARGET,
        )
        .unwrap();
        assert_eq!(manifest.build.git_revision, head_revision(&s.root));
        verify(
            Path::new("target/wasm32v1-none/release"),
            None,
            None,
            DEFAULT_TARGET,
        )
        .unwrap();
    }

    #[test]
    fn generate_writes_manifest_and_shasums() {
        let s = scratch_repo();
        write_wasm(&s.release, "fluxora_stream.wasm", &sample_wasm());

        let manifest = generate(&s.release, None, None, DEFAULT_TARGET).unwrap();

        assert_eq!(manifest.schema, SCHEMA);
        assert_eq!(manifest.build.build_type, BUILD_TYPE);
        assert_eq!(manifest.build.target, DEFAULT_TARGET);
        assert_eq!(manifest.build.git_revision, head_revision(&s.root));
        assert_eq!(manifest.build.soroban_sdk, "27.0.5");
        assert_eq!(manifest.build.toolchain_channel.as_deref(), Some("1.97.1"));
        assert_eq!(
            manifest
                .build
                .profile
                .values
                .get("opt-level")
                .and_then(|v| v.as_str()),
            Some("z")
        );
        assert_eq!(
            manifest.build.profile.values.get("lto"),
            Some(&serde_json::json!(true))
        );
        assert_eq!(manifest.build.rustc, rustc_version());

        assert_eq!(manifest.subject.len(), 1);
        assert_eq!(manifest.subject[0].name, "fluxora_stream.wasm");
        assert_eq!(manifest.subject[0].sha256, sha256_hex(&sample_wasm()));

        let shasums = fs::read_to_string(s.release.join(SHASUMS_FILENAME)).unwrap();
        assert_eq!(
            shasums,
            format!(
                "{hash}  fluxora_stream.wasm\n",
                hash = sha256_hex(&sample_wasm())
            )
        );

        let on_disk: Manifest =
            serde_json::from_str(&fs::read_to_string(s.release.join(MANIFEST_FILENAME)).unwrap())
                .unwrap();
        assert_eq!(on_disk, manifest);
    }

    #[test]
    fn workspace_root_is_discovered_from_the_release_dir() {
        let s = scratch_repo();
        assert_eq!(find_workspace_root(&s.release).unwrap(), s.root);
    }

    #[test]
    fn verify_passes_on_matching_artifacts() {
        let s = scratch_repo();
        write_wasm(&s.release, "fluxora_stream.wasm", &sample_wasm());
        generate(&s.release, None, None, DEFAULT_TARGET).unwrap();
        assert_eq!(verify(&s.release, None, None, DEFAULT_TARGET).unwrap(), 1);
    }

    #[test]
    fn verify_covers_every_contract_in_one_manifest() {
        let s = scratch_repo();
        write_wasm(&s.release, "fluxora_stream.wasm", &sample_wasm());
        write_wasm(
            &s.release,
            "fluxora_archival_probe.wasm",
            b"\x00asm probe bytes",
        );
        let manifest = generate(&s.release, None, None, DEFAULT_TARGET).unwrap();

        let names: Vec<&str> = manifest.subject.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            ["fluxora_archival_probe.wasm", "fluxora_stream.wasm"]
        );
        assert_eq!(verify(&s.release, None, None, DEFAULT_TARGET).unwrap(), 2);
    }

    #[test]
    fn verify_rejects_tampered_artifact() {
        let s = scratch_repo();
        write_wasm(&s.release, "fluxora_stream.wasm", &sample_wasm());
        generate(&s.release, None, None, DEFAULT_TARGET).unwrap();

        let mut tampered = sample_wasm();
        tampered[10] ^= 0xff;
        write_wasm(&s.release, "fluxora_stream.wasm", &tampered);

        match verify(&s.release, None, None, DEFAULT_TARGET) {
            Err(Error::HashMismatch {
                name,
                expected,
                actual,
            }) => {
                assert_eq!(name, "fluxora_stream.wasm");
                assert_eq!(expected, sha256_hex(&sample_wasm()));
                assert_eq!(actual, sha256_hex(&tampered));
            }
            other => panic!("expected HashMismatch, got {other:?}"),
        }
    }

    #[test]
    fn verify_rejects_missing_artifact() {
        let s = scratch_repo();
        write_wasm(&s.release, "fluxora_stream.wasm", &sample_wasm());
        generate(&s.release, None, None, DEFAULT_TARGET).unwrap();
        fs::remove_file(s.release.join("fluxora_stream.wasm")).unwrap();

        match verify(&s.release, None, None, DEFAULT_TARGET) {
            Err(Error::MissingArtifact { name }) => assert_eq!(name, "fluxora_stream.wasm"),
            other => panic!("expected MissingArtifact, got {other:?}"),
        }
    }

    #[test]
    fn verify_rejects_unlisted_artifact() {
        let s = scratch_repo();
        write_wasm(&s.release, "fluxora_stream.wasm", &sample_wasm());
        generate(&s.release, None, None, DEFAULT_TARGET).unwrap();
        write_wasm(&s.release, "fluxora_sneaky.wasm", &sample_wasm());

        match verify(&s.release, None, None, DEFAULT_TARGET) {
            Err(Error::UnlistedArtifact { name }) => assert_eq!(name, "fluxora_sneaky.wasm"),
            other => panic!("expected UnlistedArtifact, got {other:?}"),
        }
    }

    #[test]
    fn generate_rejects_an_empty_release_dir() {
        let s = scratch_repo();
        assert!(matches!(
            generate(&s.release, None, None, DEFAULT_TARGET),
            Err(Error::NoWasmArtifacts(_))
        ));
    }

    #[test]
    fn verify_fails_when_the_manifest_is_missing() {
        let s = scratch_repo();
        write_wasm(&s.release, "fluxora_stream.wasm", &sample_wasm());
        assert!(matches!(
            verify(&s.release, None, None, DEFAULT_TARGET),
            Err(Error::ManifestMissing(_))
        ));
    }

    #[test]
    fn verify_fails_on_an_unparseable_manifest() {
        let s = scratch_repo();
        write_wasm(&s.release, "fluxora_stream.wasm", &sample_wasm());
        fs::write(s.release.join(MANIFEST_FILENAME), "not json").unwrap();
        assert!(matches!(
            verify(&s.release, None, None, DEFAULT_TARGET),
            Err(Error::Json(_))
        ));
    }

    #[test]
    fn verify_rejects_a_drifted_git_revision() {
        let s = scratch_repo();
        write_wasm(&s.release, "fluxora_stream.wasm", &sample_wasm());
        generate(&s.release, None, None, DEFAULT_TARGET).unwrap();

        let path = s.release.join(MANIFEST_FILENAME);
        let mut manifest: Manifest =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        manifest.build.git_revision = "0".repeat(40);
        fs::write(&path, serde_json::to_string_pretty(&manifest).unwrap()).unwrap();

        match verify(&s.release, None, None, DEFAULT_TARGET) {
            Err(Error::MetadataMismatch {
                field,
                recorded,
                actual,
            }) => {
                assert_eq!(field, "git_revision");
                assert_eq!(recorded, "0".repeat(40));
                assert_eq!(actual, head_revision(&s.root));
            }
            other => panic!("expected MetadataMismatch, got {other:?}"),
        }
    }

    #[test]
    fn verify_rejects_release_profile_drift() {
        let s = scratch_repo();
        write_wasm(&s.release, "fluxora_stream.wasm", &sample_wasm());
        generate(&s.release, None, None, DEFAULT_TARGET).unwrap();

        // Simulate a release-flags change between generate and verify.
        let cargo_toml = s.root.join("Cargo.toml");
        let edited = fs::read_to_string(&cargo_toml)
            .unwrap()
            .replace("lto = true", "lto = false");
        fs::write(&cargo_toml, edited).unwrap();

        match verify(&s.release, None, None, DEFAULT_TARGET) {
            Err(Error::MetadataMismatch { field, .. }) => assert_eq!(field, "profile"),
            other => panic!("expected profile MetadataMismatch, got {other:?}"),
        }
    }

    #[test]
    fn generate_records_whether_the_tree_is_dirty() {
        let s = scratch_repo();
        write_wasm(&s.release, "fluxora_stream.wasm", &sample_wasm());
        assert!(
            generate(&s.release, None, None, DEFAULT_TARGET)
                .unwrap()
                .build
                .git_dirty
        );

        git(&s.root, &["add", "-A"]);
        git(&s.root, &["commit", "-qm", "add wasm"]);
        assert!(
            !generate(&s.release, None, None, DEFAULT_TARGET)
                .unwrap()
                .build
                .git_dirty
        );
    }

    #[test]
    fn generate_is_deterministic_except_the_timestamp() {
        let s = scratch_repo();
        write_wasm(&s.release, "fluxora_stream.wasm", &sample_wasm());
        write_wasm(
            &s.release,
            "fluxora_archival_probe.wasm",
            b"\x00asm probe bytes",
        );

        let a = generate(&s.release, None, None, DEFAULT_TARGET).unwrap();
        let b = generate(&s.release, None, None, DEFAULT_TARGET).unwrap();

        assert_eq!(a.subject, b.subject);
        let mut a_ = a.clone();
        let mut b_ = b.clone();
        a_.build.started_on.clear();
        b_.build.started_on.clear();
        assert_eq!(a_, b_);
    }

    #[test]
    fn format_unix_is_rfc3339_utc() {
        assert_eq!(format_unix(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_unix(86_400), "1970-01-02T00:00:00Z");
        assert_eq!(format_unix(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(format_unix(1_893_456_000), "2030-01-01T00:00:00Z");
    }
}
