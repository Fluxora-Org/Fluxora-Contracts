# Contract Coverage

Every crate in the workspace is measured and gated against its own committed
coverage floor. The floor for a crate lives in that crate's directory as
`coverage-floor.txt`, holds its measured line coverage as a percentage, and is
enforced by `script/check_coverage_floors.py`.

| Crate | Path | Committed floor |
|-------|------|-----------------|
| `fluxora-stream` | `contracts/stream` | 96.4% |
| `fluxora_factory` | `contracts/factory` | 80% |
| `fluxora-archival-probe` | `contracts/archival-probe` | 90% |
| `fluxora-provenance` | `tools/provenance` | 70% |

The crate list is not written down in the scripts or the workflow. It is read
from `cargo metadata`, which is the same source for measurement and for the
gate, so the two cannot drift apart.

## What fails the build

The `Code Coverage` job fails when any of these hold, for any crate in the
workspace:

- **A crate drops below its floor.** The error names the crate and both figures.
- **A crate has no committed floor.** Adding a crate without a
  `coverage-floor.txt` fails the job. A new crate cannot arrive untested
  and unnoticed.
- **A crate has no coverage report.** If the measurement step skips a crate,
  the gate reports a missing report rather than passing on the crates that did
  run.
- **A floor or report is unreadable.** A malformed floor is an error, not a
  zero, so a typo cannot be read as a passing or a failing baseline by accident.

CI writes a per-crate table, and a per-module breakdown for each crate, to the
job summary, and exposes one `coverage_<crate>=<percentage>` step output per
measured crate.

## Changing a floor

The floor must only be raised after measuring the new baseline. Lowering one is
not a valid way to resolve a coverage failure: it converts a detected regression
into an accepted baseline. Floor files are maintainer-reviewed in
`.github/CODEOWNERS` for that reason.

The `fluxora-archival-probe` and `fluxora-provenance` floors were seeded below
the first measured baseline, because the tooling that would measure them was
not available in the environment the change was authored in. They should be
raised to the measured value from the first CI run, which prints it per crate in
the job summary.

To re-baseline deliberately:

1. Add or restore the tests you intend to keep.
2. Run `script/measure-coverage.sh` to get the real figure.
3. Commit the new number in that crate's `coverage-floor.txt`.

## Reproducing the reports locally on Linux

```bash
cargo install cargo-tarpaulin --version 0.31 --locked
script/measure-coverage.sh coverage
python3 script/check_coverage_floors.py --reports-dir coverage
```

`measure-coverage.sh` walks the workspace and writes one report per crate to
`coverage/<crate-name>/cobertura.xml`, with an HTML report alongside it. The
gate then compares each crate against its own floor. The `coverage/` directory
is generated output and is not committed.

Useful environment variables: `COVERAGE_TIMEOUT` (per-test timeout, default
300s, raised above tarpaulin's 60s default because instrumented proptest suites
exceed it) and `CARGO_TARPAULIN_VERSION`.

`--features testutils` is applied only to crates that declare the feature, so a
crate without it is still measured correctly.
