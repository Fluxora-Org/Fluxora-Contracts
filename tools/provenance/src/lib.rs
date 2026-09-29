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

    /// The three files `collect_metadata` reads, so a scratch dir behaves like
    /// a real workspace root.
    fn write_workspace_files(root: &Path) {
        fs::write(root.join("Cargo.toml"), FAKE_WORKSPACE).unwrap();
        fs::write(root.join("Cargo.lock"), FAKE_LOCK).unwrap();
        fs::write(root.join("rust-toolchain.toml"), FAKE_TOOLCHAIN).unwrap();
    }

    /// `git init` plus a committed baseline, so `git rev-parse HEAD` yields a
    /// stable revision.
    fn init_repo(root: &Path) {
        git(root, &["init", "-q"]);
        git(root, &["config", "user.email", "test@example.com"]);
        git(root, &["config", "user.name", "Test"]);
        git(root, &["add", "-A"]);
        git(root, &["commit", "-qm", "init"]);
    }

    fn scratch_repo() -> Scratch {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().to_path_buf();
        write_workspace_files(&root);
        init_repo(&root);
        let release = root.join("target").join("wasm32v1-none").join("release");
        fs::create_dir_all(&release).unwrap();
        Scratch {
            _tmp: tmp,
            root,
            release,
        }
    }

    /// A workspace root plus a release dir that lives *outside* it, so a test
    /// can tell the explicit `workspace_root` argument apart from discovery.
    fn detached_release() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().join("workspace");
        fs::create_dir_all(&root).unwrap();
        write_workspace_files(&root);
        init_repo(&root);
        let release = tmp.path().join("out").join("release");
        fs::create_dir_all(&release).unwrap();
        (tmp, root, release)
    }

    /// Re-read a manifest from disk, as `verify` does.
    fn read_manifest(path: &Path) -> Manifest {
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
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
        // `find_workspace_root` canonicalizes the path it is given, so the
        // expectation has to be canonical too: on macOS `tempdir()` hands out
        // `/var/...`, which canonicalizes to `/private/var/...`, and comparing
        // against the raw temp path fails there (it passes on Linux, where no
        // such symlink exists).
        assert_eq!(
            find_workspace_root(&s.release).unwrap(),
            s.root.canonicalize().unwrap()
        );
    }

    #[test]
    fn workspace_root_discovery_walks_up_and_ignores_non_workspace_manifests() {
        let s = scratch_repo();
        // A member crate whose own manifest has no `[workspace]` table must not
        // terminate the walk.
        let nested = s.root.join("contracts").join("stream");
        fs::create_dir_all(&nested).unwrap();
        fs::write(
            nested.join("Cargo.toml"),
            "[package]\nname = \"fluxora-stream\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();

        let expected = s.root.canonicalize().unwrap();
        assert_eq!(find_workspace_root(&nested).unwrap(), expected);
        assert_eq!(find_workspace_root(&s.release).unwrap(), expected);
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

    // -----------------------------------------------------------------------
    // Digest and date primitives
    // -----------------------------------------------------------------------

    /// Everything else in this suite compares a digest produced by
    /// `sha256_hex` against another one produced by `sha256_hex`, so without a
    /// known-answer test a switch to a different algorithm would stay green
    /// while silently breaking `sha256sum -c SHASUMS` and the recorded digests.
    #[test]
    fn sha256_hex_matches_published_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(sha256_hex(b"abc").len(), 64);
    }

    #[test]
    fn civil_from_days_handles_epoch_leap_days_and_pre_epoch_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        assert_eq!(civil_from_days(-719_468), (0, 3, 1));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        assert_eq!(civil_from_days(11_017), (2000, 3, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
    }

    #[test]
    fn now_rfc3339_is_a_fixed_width_utc_timestamp_for_the_current_second() {
        let before = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let stamp = now_rfc3339();
        let after = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        assert_eq!(stamp.len(), "1970-01-01T00:00:00Z".len());
        assert_eq!(&stamp[4..5], "-");
        assert_eq!(&stamp[10..11], "T");
        assert!(stamp.ends_with('Z'));
        assert!(
            (before..=after).any(|secs| format_unix(secs) == stamp),
            "{stamp} is not a timestamp between {before} and {after}"
        );
    }

    // -----------------------------------------------------------------------
    // Error contract — every failure mode names what failed and where
    // -----------------------------------------------------------------------

    #[test]
    fn error_display_names_the_failure_and_the_paths_involved() {
        assert_eq!(
            Error::ReleaseDirMissing(PathBuf::from("/out/release")).to_string(),
            "release dir does not exist: /out/release"
        );
        assert_eq!(
            Error::NotADirectory(PathBuf::from("/out/SHASUMS")).to_string(),
            "release path is not a directory: /out/SHASUMS"
        );
        assert_eq!(
            Error::NoWasmArtifacts(PathBuf::from("/out/release")).to_string(),
            "no *.wasm artifacts found in /out/release"
        );
        assert_eq!(
            Error::WorkspaceRootNotFound(PathBuf::from("/out/release")).to_string(),
            "could not find a workspace Cargo.toml (with [workspace]) walking up from /out/release"
        );
        assert_eq!(
            Error::SdkVersionNotFound.to_string(),
            "soroban-sdk not found in Cargo.lock"
        );
        assert_eq!(
            Error::ManifestMissing(PathBuf::from("/out/release/provenance.json")).to_string(),
            "provenance manifest not found: /out/release/provenance.json"
        );
        assert_eq!(
            Error::MissingArtifact {
                name: "fluxora_stream.wasm".to_string(),
            }
            .to_string(),
            "artifact listed in the manifest is missing from the release dir: fluxora_stream.wasm"
        );
        assert_eq!(
            Error::UnlistedArtifact {
                name: "fluxora_sneaky.wasm".to_string(),
            }
            .to_string(),
            "artifact present in the release dir is not listed in the manifest: fluxora_sneaky.wasm \
             (regenerate provenance so every contract is covered)"
        );
        assert_eq!(
            Error::Io {
                path: PathBuf::from("/out/release"),
                source: std::io::Error::new(std::io::ErrorKind::NotFound, "no such file"),
            }
            .to_string(),
            "/out/release: no such file"
        );
        assert_eq!(
            Error::Toml("expected `=`".to_string()).to_string(),
            "TOML parse error: expected `=`"
        );

        // Both digests are reported, so a mismatch can be investigated straight
        // from the log without re-running the tool.
        let mismatch = Error::HashMismatch {
            name: "fluxora_stream.wasm".to_string(),
            expected: "aa".repeat(32),
            actual: "bb".repeat(32),
        }
        .to_string();
        assert!(mismatch.contains("sha256 mismatch for fluxora_stream.wasm"));
        assert!(mismatch.contains(&"aa".repeat(32)));
        assert!(mismatch.contains(&"bb".repeat(32)));

        // ... and a drift reports the field plus the recorded and current value.
        assert_eq!(
            Error::MetadataMismatch {
                field: "soroban_sdk",
                recorded: "27.0.5".to_string(),
                actual: "27.0.6".to_string(),
            }
            .to_string(),
            "soroban_sdk drifted since provenance was generated: manifest 27.0.5 != current 27.0.6"
        );
    }

    // -----------------------------------------------------------------------
    // list_wasm_files boundaries
    // -----------------------------------------------------------------------

    #[test]
    fn list_wasm_files_sorts_and_ignores_everything_that_is_not_a_wasm_file() {
        let s = scratch_repo();
        write_wasm(&s.release, "zeta.wasm", b"z");
        write_wasm(&s.release, "alpha.wasm", b"a");
        fs::write(s.release.join("NOTES.md"), b"notes").unwrap();
        fs::write(s.release.join("fluxora_stream.wasm.bak"), b"backup").unwrap();
        // Nested entries and directory-shaped entries are not artifacts: only
        // files directly inside the release dir are hashed.
        fs::create_dir_all(s.release.join("nested")).unwrap();
        write_wasm(&s.release.join("nested"), "deep.wasm", b"d");
        fs::create_dir_all(s.release.join("dir.wasm")).unwrap();

        let names: Vec<String> = list_wasm_files(&s.release)
            .unwrap()
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["alpha.wasm", "zeta.wasm"]);
    }

    #[test]
    fn list_wasm_files_reports_an_unreadable_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("no-such-dir");
        match list_wasm_files(&missing) {
            Err(Error::Io { path, .. }) => assert_eq!(path, missing),
            other => panic!("expected Io, got {other:?}"),
        }
    }

    #[test]
    fn generate_rejects_a_release_dir_with_no_wasm_files_even_when_it_holds_other_files() {
        let s = scratch_repo();
        fs::write(s.release.join(SHASUMS_FILENAME), b"stale digests").unwrap();
        fs::write(s.release.join("fluxora_stream.wasm.bak"), b"backup").unwrap();

        match generate(&s.release, None, None, DEFAULT_TARGET) {
            Err(Error::NoWasmArtifacts(p)) => assert_eq!(p, s.release),
            other => panic!("expected NoWasmArtifacts, got {other:?}"),
        }
    }

    #[test]
    fn generate_and_verify_reject_a_release_path_that_is_a_file() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("fluxora_stream.wasm");
        fs::write(&file, sample_wasm()).unwrap();

        match generate(&file, None, None, DEFAULT_TARGET) {
            Err(Error::NotADirectory(p)) => assert_eq!(p, file),
            other => panic!("expected NotADirectory, got {other:?}"),
        }
        match verify(&file, None, None, DEFAULT_TARGET) {
            Err(Error::NotADirectory(p)) => assert_eq!(p, file),
            other => panic!("expected NotADirectory, got {other:?}"),
        }
    }

    #[test]
    fn generate_and_verify_reject_a_missing_release_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("no-such-release");

        match generate(&missing, None, None, DEFAULT_TARGET) {
            Err(Error::ReleaseDirMissing(p)) => assert_eq!(p, missing),
            other => panic!("expected ReleaseDirMissing, got {other:?}"),
        }
        match verify(&missing, None, None, DEFAULT_TARGET) {
            Err(Error::ReleaseDirMissing(p)) => assert_eq!(p, missing),
            other => panic!("expected ReleaseDirMissing, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // generate / verify argument overrides
    // -----------------------------------------------------------------------

    #[test]
    fn generate_and_verify_honor_an_explicit_manifest_path() {
        let s = scratch_repo();
        write_wasm(&s.release, "fluxora_stream.wasm", &sample_wasm());
        let manifest_path = s.root.join("provenance").join("release.json");
        fs::create_dir_all(manifest_path.parent().unwrap()).unwrap();

        generate(&s.release, Some(&manifest_path), None, DEFAULT_TARGET).unwrap();

        assert!(manifest_path.is_file());
        assert!(!s.release.join(MANIFEST_FILENAME).exists());
        // SHASUMS is always written next to the artifacts, whatever the
        // manifest override is.
        assert!(s.release.join(SHASUMS_FILENAME).is_file());

        assert_eq!(
            verify(&s.release, Some(&manifest_path), None, DEFAULT_TARGET).unwrap(),
            1
        );
        // Without the override, verify looks at <release-dir>/provenance.json.
        match verify(&s.release, None, None, DEFAULT_TARGET) {
            Err(Error::ManifestMissing(p)) => {
                assert_eq!(p, s.release.join(MANIFEST_FILENAME));
            }
            other => panic!("expected ManifestMissing, got {other:?}"),
        }
    }

    #[test]
    fn an_explicit_workspace_root_is_used_for_a_detached_release_dir() {
        let (_tmp, root, release) = detached_release();
        write_wasm(&release, "fluxora_stream.wasm", &sample_wasm());

        // Discovery alone cannot find a workspace above this release dir ...
        match find_workspace_root(&release) {
            Err(Error::WorkspaceRootNotFound(p)) => assert_eq!(p, release),
            other => panic!("expected WorkspaceRootNotFound, got {other:?}"),
        }
        // ... and `generate` without the override fails the same way ...
        assert!(matches!(
            generate(&release, None, None, DEFAULT_TARGET),
            Err(Error::WorkspaceRootNotFound(_))
        ));

        // ... while the explicit root records the checkout it was told about.
        let manifest = generate(&release, None, Some(&root), DEFAULT_TARGET).unwrap();
        assert_eq!(manifest.build.git_revision, head_revision(&root));
        assert_eq!(
            verify(&release, None, Some(&root), DEFAULT_TARGET).unwrap(),
            1
        );
    }

    // -----------------------------------------------------------------------
    // Build-input discovery failures
    // -----------------------------------------------------------------------

    #[test]
    fn collect_metadata_reports_a_lock_without_a_soroban_sdk_entry() {
        let s = scratch_repo();
        write_wasm(&s.release, "fluxora_stream.wasm", &sample_wasm());
        fs::write(
            s.root.join("Cargo.lock"),
            "[[package]]\nname = \"serde\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();

        assert!(matches!(
            generate(&s.release, None, None, DEFAULT_TARGET),
            Err(Error::SdkVersionNotFound)
        ));

        // A lock with no `[[package]]` array at all is the same failure.
        fs::write(s.root.join("Cargo.lock"), "[metadata]\n").unwrap();
        assert!(matches!(
            generate(&s.release, None, None, DEFAULT_TARGET),
            Err(Error::SdkVersionNotFound)
        ));
    }

    #[test]
    fn collect_metadata_reports_an_unparseable_workspace_manifest() {
        let s = scratch_repo();
        write_wasm(&s.release, "fluxora_stream.wasm", &sample_wasm());
        fs::write(s.root.join("Cargo.toml"), "[[[").unwrap();

        // The root is passed explicitly: an unparseable `Cargo.toml` is skipped
        // by discovery (it cannot be shown to declare `[workspace]`), so the
        // parse failure only surfaces when the root is known.
        assert!(matches!(
            generate(&s.release, None, Some(&s.root), DEFAULT_TARGET),
            Err(Error::Toml(_))
        ));
        assert!(matches!(
            find_workspace_root(&s.release),
            Err(Error::WorkspaceRootNotFound(_))
        ));
    }

    // -----------------------------------------------------------------------
    // Environment drift — every recorded build input is checked
    // -----------------------------------------------------------------------

    #[test]
    fn an_absent_toolchain_pin_is_recorded_as_none_and_still_verifies() {
        let s = scratch_repo();
        write_wasm(&s.release, "fluxora_stream.wasm", &sample_wasm());
        fs::remove_file(s.root.join("rust-toolchain.toml")).unwrap();

        let manifest = generate(&s.release, None, None, DEFAULT_TARGET).unwrap();
        assert!(manifest.build.toolchain_channel.is_none());
        // Both sides render the absent pin as `<none>`, so verify stays green.
        assert_eq!(verify(&s.release, None, None, DEFAULT_TARGET).unwrap(), 1);
    }

    #[test]
    fn verify_rejects_a_drifted_toolchain_pin() {
        let s = scratch_repo();
        write_wasm(&s.release, "fluxora_stream.wasm", &sample_wasm());
        generate(&s.release, None, None, DEFAULT_TARGET).unwrap();

        fs::write(
            s.root.join("rust-toolchain.toml"),
            "[toolchain]\nchannel = \"1.98.0\"\n",
        )
        .unwrap();

        match verify(&s.release, None, None, DEFAULT_TARGET) {
            Err(Error::MetadataMismatch {
                field,
                recorded,
                actual,
            }) => {
                assert_eq!(field, "toolchain_channel");
                assert_eq!(recorded, "1.97.1");
                assert_eq!(actual, "1.98.0");
            }
            other => panic!("expected toolchain_channel MetadataMismatch, got {other:?}"),
        }
    }

    #[test]
    fn verify_rejects_a_removed_toolchain_pin() {
        let s = scratch_repo();
        write_wasm(&s.release, "fluxora_stream.wasm", &sample_wasm());
        generate(&s.release, None, None, DEFAULT_TARGET).unwrap();

        fs::remove_file(s.root.join("rust-toolchain.toml")).unwrap();

        match verify(&s.release, None, None, DEFAULT_TARGET) {
            Err(Error::MetadataMismatch {
                field,
                recorded,
                actual,
            }) => {
                assert_eq!(field, "toolchain_channel");
                assert_eq!(recorded, "1.97.1");
                assert_eq!(actual, "<none>");
            }
            other => panic!("expected toolchain_channel MetadataMismatch, got {other:?}"),
        }
    }

    #[test]
    fn verify_rejects_a_drifted_target() {
        let s = scratch_repo();
        write_wasm(&s.release, "fluxora_stream.wasm", &sample_wasm());
        generate(&s.release, None, None, DEFAULT_TARGET).unwrap();

        match verify(&s.release, None, None, "wasm32-unknown-unknown") {
            Err(Error::MetadataMismatch {
                field,
                recorded,
                actual,
            }) => {
                assert_eq!(field, "target");
                assert_eq!(recorded, DEFAULT_TARGET);
                assert_eq!(actual, "wasm32-unknown-unknown");
            }
            other => panic!("expected target MetadataMismatch, got {other:?}"),
        }
    }

    #[test]
    fn verify_rejects_a_drifted_soroban_sdk_version() {
        let s = scratch_repo();
        write_wasm(&s.release, "fluxora_stream.wasm", &sample_wasm());
        generate(&s.release, None, None, DEFAULT_TARGET).unwrap();

        fs::write(
            s.root.join("Cargo.lock"),
            "[[package]]\nname = \"soroban-sdk\"\nversion = \"27.0.6\"\n",
        )
        .unwrap();

        match verify(&s.release, None, None, DEFAULT_TARGET) {
            Err(Error::MetadataMismatch {
                field,
                recorded,
                actual,
            }) => {
                assert_eq!(field, "soroban_sdk");
                assert_eq!(recorded, "27.0.5");
                assert_eq!(actual, "27.0.6");
            }
            other => panic!("expected soroban_sdk MetadataMismatch, got {other:?}"),
        }
    }

    /// `rustc` and `cargo` come from the toolchain that is actually installed,
    /// so they are pinned by rewriting the recorded manifest the way a
    /// different builder's manifest would look.
    #[test]
    fn verify_rejects_a_drifted_compiler_identity() {
        let s = scratch_repo();
        write_wasm(&s.release, "fluxora_stream.wasm", &sample_wasm());
        generate(&s.release, None, None, DEFAULT_TARGET).unwrap();

        let path = s.release.join(MANIFEST_FILENAME);
        for field in ["rustc", "cargo"] {
            // Regenerate each round so only the field under test has drifted.
            generate(&s.release, None, None, DEFAULT_TARGET).unwrap();
            let recorded = "0.0.0 (built elsewhere)".to_string();
            let mut manifest = read_manifest(&path);
            if field == "rustc" {
                manifest.build.rustc = recorded.clone();
            } else {
                manifest.build.cargo = recorded.clone();
            }
            fs::write(&path, serde_json::to_string_pretty(&manifest).unwrap()).unwrap();

            match verify(&s.release, None, None, DEFAULT_TARGET) {
                Err(Error::MetadataMismatch {
                    field: drifted,
                    recorded: reported,
                    actual,
                }) => {
                    assert_eq!(drifted, field);
                    assert_eq!(reported, recorded);
                    assert_ne!(actual, recorded);
                }
                other => panic!("expected {field} MetadataMismatch, got {other:?}"),
            }
        }
    }

    // -----------------------------------------------------------------------
    // Idempotence, recovery and the files the tool owns
    // -----------------------------------------------------------------------

    #[test]
    fn generate_overwrites_the_previous_manifest_so_a_rebuild_can_be_reverified() {
        let s = scratch_repo();
        write_wasm(&s.release, "fluxora_stream.wasm", &sample_wasm());
        generate(&s.release, None, None, DEFAULT_TARGET).unwrap();

        // Tamper -> the gate must fail ...
        write_wasm(&s.release, "fluxora_stream.wasm", b"\x00asm rebuilt bytes");
        assert!(matches!(
            verify(&s.release, None, None, DEFAULT_TARGET),
            Err(Error::HashMismatch { .. })
        ));

        // ... and regenerating after the rebuild is the recovery path.
        let manifest = generate(&s.release, None, None, DEFAULT_TARGET).unwrap();
        assert_eq!(
            manifest.subject[0].sha256,
            sha256_hex(b"\x00asm rebuilt bytes")
        );
        assert_eq!(verify(&s.release, None, None, DEFAULT_TARGET).unwrap(), 1);

        // A newly added artifact is caught, then covered by regenerating.
        write_wasm(&s.release, "fluxora_archival_probe.wasm", b"\x00asm probe");
        assert!(matches!(
            verify(&s.release, None, None, DEFAULT_TARGET),
            Err(Error::UnlistedArtifact { .. })
        ));
        generate(&s.release, None, None, DEFAULT_TARGET).unwrap();
        assert_eq!(verify(&s.release, None, None, DEFAULT_TARGET).unwrap(), 2);
    }

    #[test]
    fn generate_and_verify_leave_no_temp_files_behind() {
        let s = scratch_repo();
        write_wasm(&s.release, "fluxora_stream.wasm", &sample_wasm());
        generate(&s.release, None, None, DEFAULT_TARGET).unwrap();

        // The atomic write renames its temp file into place, so the release dir
        // holds exactly the artifact, the manifest and SHASUMS.
        let mut names: Vec<String> = fs::read_dir(&s.release)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                SHASUMS_FILENAME.to_string(),
                "fluxora_stream.wasm".to_string(),
                MANIFEST_FILENAME.to_string(),
            ]
        );

        // `verify` never writes anything: both files must be byte-identical
        // afterwards.
        let shasums = fs::read_to_string(s.release.join(SHASUMS_FILENAME)).unwrap();
        let manifest = fs::read_to_string(s.release.join(MANIFEST_FILENAME)).unwrap();
        assert_eq!(verify(&s.release, None, None, DEFAULT_TARGET).unwrap(), 1);
        assert_eq!(
            fs::read_to_string(s.release.join(SHASUMS_FILENAME)).unwrap(),
            shasums
        );
        assert_eq!(
            fs::read_to_string(s.release.join(MANIFEST_FILENAME)).unwrap(),
            manifest
        );
    }

    #[test]
    fn shasums_lists_every_artifact_once_in_sorted_order() {
        let s = scratch_repo();
        write_wasm(&s.release, "fluxora_stream.wasm", &sample_wasm());
        write_wasm(
            &s.release,
            "fluxora_archival_probe.wasm",
            b"\x00asm probe bytes",
        );
        generate(&s.release, None, None, DEFAULT_TARGET).unwrap();

        let shasums = fs::read_to_string(s.release.join(SHASUMS_FILENAME)).unwrap();
        assert_eq!(
            shasums,
            format!(
                "{probe}  fluxora_archival_probe.wasm\n{stream}  fluxora_stream.wasm\n",
                probe = sha256_hex(b"\x00asm probe bytes"),
                stream = sha256_hex(&sample_wasm()),
            )
        );
        // Two lines in `sha256sum` format, so `sha256sum -c SHASUMS` consumes it.
        assert_eq!(shasums.lines().count(), 2);
        for line in shasums.lines() {
            let (hash, name) = line.split_once("  ").expect("sha256sum format");
            assert_eq!(hash.len(), 64);
            assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
            assert!(name.ends_with(".wasm"));
        }
    }

    #[test]
    fn verify_ignores_files_that_are_not_wasm_artifacts() {
        let s = scratch_repo();
        write_wasm(&s.release, "fluxora_stream.wasm", &sample_wasm());
        generate(&s.release, None, None, DEFAULT_TARGET).unwrap();

        fs::write(s.release.join("README.txt"), b"notes").unwrap();
        fs::create_dir_all(s.release.join("nested")).unwrap();
        write_wasm(&s.release.join("nested"), "ignored.wasm", b"nested");

        assert_eq!(verify(&s.release, None, None, DEFAULT_TARGET).unwrap(), 1);
    }
}
