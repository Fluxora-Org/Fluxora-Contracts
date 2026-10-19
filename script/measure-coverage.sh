#!/usr/bin/env bash
# Measure line coverage for every crate in the workspace, one report per crate.
#
# Issue #1873. Coverage used to be measured for fluxora-stream alone, with the
# factory bolted on by a second hand-maintained tarpaulin invocation in
# .github/workflows/ci.yml. A crate added after that had no measurement at all,
# so it could arrive with no tests and still look green in every other gate.
#
# The crate list is not written here. It is read from `cargo metadata` via
# `script/check_coverage_floors.py --list-crates`, which is the same list the
# gate enforces floors against. Measurement and gate therefore cannot drift:
# there is one definition of "a crate in this workspace", and it is cargo's.
#
# Output layout (what the gate expects):
#
#   coverage/<crate-name>/cobertura.xml
#   coverage/<crate-name>/cobertura.html
#
# Usage:
#   script/measure-coverage.sh [output-dir]
#
# Environment:
#   COVERAGE_OUTPUT_DIR   same as the positional argument; defaults to ./coverage
#   COVERAGE_TIMEOUT      per-test timeout in seconds; defaults to 300
#   CARGO_TARPAULIN_VERSION  tarpaulin version to install if it is missing
#
# A crate whose tarpaulin run fails aborts the whole script: a partial
# measurement would leave the gate reading a stale or absent report, which is
# exactly the silent-pass failure mode this script exists to remove.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

OUTPUT_DIR="${1:-${COVERAGE_OUTPUT_DIR:-coverage}}"
TIMEOUT="${COVERAGE_TIMEOUT:-300}"
TARPAULIN_VERSION="${CARGO_TARPAULIN_VERSION:-0.31}"

if ! cargo tarpaulin --version >/dev/null 2>&1; then
  echo "cargo-tarpaulin is not installed; installing ${TARPAULIN_VERSION}." >&2
  cargo install cargo-tarpaulin --version "${TARPAULIN_VERSION}" --locked
fi

# Read the crate list once, before the loop. A workspace that cannot be
# resolved is a hard stop: measuring nothing and reporting success is worse
# than failing.
crate_list="$(python3 script/check_coverage_floors.py --list-crates)"
if [ -z "${crate_list}" ]; then
  echo "error: cargo metadata reported no workspace crates to measure." >&2
  exit 1
fi

crate_count=0
while IFS=$'\t' read -r name directory feature; do
  [ -n "${name}" ] || continue
  crate_count=$((crate_count + 1))

  # `--features testutils` is only valid for crates that declare it. A crate
  # without the feature is measured with the flag omitted, so adding a new
  # crate does not require editing this script.
  feature_args=()
  if [ -n "${feature}" ]; then
    feature_args=(--features "${feature}")
  fi

  echo "::group::Measuring coverage for ${name} (${directory})"

  # --ignore-tests keeps the test harness itself out of the denominator, so the
  # floor tracks production code rather than how many tests were written.
  # --skip-clean preserves the build cache between crates; without it each
  # crate would rebuild the whole dependency graph from scratch.
  # --timeout is raised from tarpaulin's 60s default: instrumented proptest
  # suites (test_withdrawable_props) exceed it and abort the run with
  # "Timed out waiting for test response" (seen on PR coverage runs).
  rm -rf "${OUTPUT_DIR:?}/${name}"
  cargo tarpaulin \
    "${feature_args[@]}" \
    --out Xml \
    --out Html \
    --output-dir "${OUTPUT_DIR}/${name}" \
    --ignore-tests \
    --skip-clean \
    --timeout "${TIMEOUT}" \
    -p "${name}"

  if [ ! -s "${OUTPUT_DIR}/${name}/cobertura.xml" ]; then
    echo "error: ${name} produced no cobertura.xml under ${OUTPUT_DIR}/${name}." >&2
    echo "::endgroup::"
    exit 1
  fi

  echo "::endgroup::"
done <<< "${crate_list}"

echo "Measured coverage for ${crate_count} workspace crate(s) into ${OUTPUT_DIR}/."
