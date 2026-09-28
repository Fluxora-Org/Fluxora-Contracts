# Stream entry point CPU budget

`contracts/stream/entrypoint-cost-baseline.json` records the instruction cost of
one successful call to each of the 24 public stream functions. The fixtures in
`contracts/stream/src/test/entrypoint_costs.rs` use the release
`wasm32v1-none` artifact, with setup calls outside the recorded invocation.
The batch fixtures use one stream. The measurements are local Soroban SDK
estimates, not transaction fees or a replacement for network simulation.

CI runs `python3 script/validate_gas.py`. It builds the release WASM, runs the
fixtures, and fails if any measured instruction count exceeds its baseline by
more than **10%**. A missing or extra ABI function, fixture, or baseline entry
also fails. The per-function table is uploaded as the `stream-entrypoint-costs`
artifact, including on failure.

To review an intentional cost change, run:

```sh
python3 script/validate_gas.py --record-baseline
```

Review the baseline diff and report before committing it. Re-record only when
the behavior or pinned Soroban SDK/toolchain changes intentionally.
