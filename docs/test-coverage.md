# Stream Contract Coverage

The stream contract coverage job records the measured baseline in
`contracts/stream/coverage-floor.txt` and fails when line coverage falls below
that value. The current committed floor is **96.4%** (644 of 668 lines).

The floor must only be raised after measuring the new baseline. Lowering it is
not a valid way to resolve a coverage failure and requires explicit review.

CI writes aggregate and per-module coverage to the job summary and uploads the
full Cobertura XML and HTML reports as the `coverage-report` artifact.

To reproduce the report locally on Linux:

```bash
cargo install cargo-tarpaulin --version 0.31 --locked
cargo tarpaulin \
  --features testutils \
  --out Xml \
  --out Html \
  --output-dir coverage \
  --ignore-tests \
  --skip-clean \
  --timeout 300 \
  -p fluxora_stream
python3 script/check_stream_coverage.py \
  --xml coverage/cobertura.xml \
  --floor contracts/stream/coverage-floor.txt
```