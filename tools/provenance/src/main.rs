use fluxora_provenance::{generate, verify, DEFAULT_TARGET};
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "\
fluxora-provenance — wasm provenance for Fluxora contract releases

Usage:
  fluxora-provenance generate <release-dir> [--manifest <path>] [--workspace-root <dir>] [--target <triple>]
  fluxora-provenance verify   <release-dir> [--manifest <path>] [--workspace-root <dir>] [--target <triple>]
  fluxora-provenance help

Commands:
  generate   Hash every *.wasm in <release-dir> and write provenance.json and
             SHASUMS next to the artifacts.
  verify     Re-hash the artifacts and compare against provenance.json; exit
             non-zero on any mismatch. This is the release gate.

Options:
  --manifest <path>       Manifest path (default: <release-dir>/provenance.json).
  --workspace-root <dir>  Workspace root to read toolchain, sdk, profile and git
                          from (default: discovered by walking up from <release-dir>).
  --target <triple>       Wasm target triple recorded and checked
                          (default: wasm32v1-none, per rust-toolchain.toml).

Exit codes:
  0  success
  1  generation or verification failed
  2  usage error
";

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let Some((cmd, rest)) = argv.split_first() else {
        eprint!("{USAGE}");
        return ExitCode::from(2);
    };
    match cmd.as_str() {
        "-h" | "--help" | "help" => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        "generate" => dispatch(rest, true),
        "verify" => dispatch(rest, false),
        other => {
            eprintln!("fluxora-provenance: unknown command '{other}'\n");
            eprint!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn dispatch(rest: &[String], is_generate: bool) -> ExitCode {
    let mut release_dir: Option<PathBuf> = None;
    let mut manifest: Option<PathBuf> = None;
    let mut workspace_root: Option<PathBuf> = None;
    let mut target = DEFAULT_TARGET.to_string();

    let mut i = 0;
    while i < rest.len() {
        let arg = &rest[i];
        if arg == "-h" || arg == "--help" {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        let (key, inline) = match arg.split_once('=') {
            Some((k, v)) => (k.to_string(), Some(v.to_string())),
            None => (arg.clone(), None),
        };
        if matches!(key.as_str(), "--manifest" | "--workspace-root" | "--target") {
            let value = match inline {
                Some(v) => v,
                None => {
                    i += 1;
                    match rest.get(i) {
                        Some(v) => v.clone(),
                        None => {
                            eprintln!("fluxora-provenance: option '{key}' requires a value\n");
                            eprint!("{USAGE}");
                            return ExitCode::from(2);
                        }
                    }
                }
            };
            match key.as_str() {
                "--manifest" => manifest = Some(PathBuf::from(value)),
                "--workspace-root" => workspace_root = Some(PathBuf::from(value)),
                "--target" => target = value,
                _ => unreachable!(),
            }
        } else if key.starts_with('-') {
            eprintln!("fluxora-provenance: unknown option '{key}'\n");
            eprint!("{USAGE}");
            return ExitCode::from(2);
        } else if release_dir.is_some() {
            eprintln!("fluxora-provenance: unexpected argument '{key}'\n");
            eprint!("{USAGE}");
            return ExitCode::from(2);
        } else {
            release_dir = Some(PathBuf::from(key));
        }
        i += 1;
    }

    let Some(dir) = release_dir else {
        eprintln!("fluxora-provenance: missing <release-dir>\n");
        eprint!("{USAGE}");
        return ExitCode::from(2);
    };

    let result = if is_generate {
        generate(
            &dir,
            manifest.as_deref(),
            workspace_root.as_deref(),
            &target,
        )
        .map(|m| {
            let short = m.build.git_revision.chars().take(8).collect::<String>();
            format!(
                "wrote {schema} for {n} artifact(s), target {target}, git {short}",
                schema = m.schema,
                n = m.subject.len(),
                target = m.build.target,
                short = short,
            )
        })
    } else {
        verify(
            &dir,
            manifest.as_deref(),
            workspace_root.as_deref(),
            &target,
        )
        .map(|n| format!("verified {n} artifact(s) against the manifest"))
    };

    match result {
        Ok(msg) => {
            println!("fluxora-provenance: {msg}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("fluxora-provenance: FAILED: {e}");
            ExitCode::FAILURE
        }
    }
}
