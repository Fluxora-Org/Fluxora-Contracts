#!/usr/bin/env bash
# Fail if captured build/run output is tracked in git.
# Used by CI (.github/workflows/no-committed-logs.yml) and for local checks.
set -euo pipefail

mapfile -t tracked < <(git ls-files -- \
  'batch_test_output.txt' \
  'script/testnet-exercise.log' \
  '*.log' \
  || true)

if [ "${#tracked[@]}" -eq 0 ] || [ -z "${tracked[0]:-}" ]; then
  echo "OK: no captured run/build logs are tracked."
  exit 0
fi

for f in "${tracked[@]}"; do
  [ -z "$f" ] && continue
  echo "ERROR: tracked capture log: $f" >&2
  echo "Remove it, keep it gitignored, and publish via CI artifacts instead." >&2
done
exit 1
