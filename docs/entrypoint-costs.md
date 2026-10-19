# Stream entry point CPU budget

`contracts/stream/entrypoint-cost-baseline.json` records the instruction cost of
one successful call to each of the 36 public stream functions. The fixtures in
`contracts/stream/src/test/entrypoint_costs.rs` use the release
`wasm32v1-none` artifact, with setup calls outside the recorded invocation.
The batch fixtures use one stream. The measurements are local Soroban SDK
estimates, not transaction fees or a replacement for network simulation.

The manually dispatched `entrypoint-costs.yml` workflow runs
`python3 script/validate_gas.py`. It builds the release WASM, runs the
fixtures, and fails if any measured instruction count exceeds its baseline by
more than **10%**. A missing or extra ABI function, fixture, or baseline entry
also fails. The per-function table is uploaded as the `stream-entrypoint-costs`
artifact, including on failure.

To review an intentional cost change, run:

```sh
python3 script/validate_gas.py --record-baseline
```

Review the baseline diff and report before committing it. Re-record only when
the behavior or pinned Soroban SDK/toolchain changes intentionally. The baseline
file is listed in `.github/CODEOWNERS`, so a change to a recorded figure cannot
merge without maintainer review.

## CI repair calibration

The baseline was remeasured with Rust 1.97.1 and Soroban SDK 27.0.6 after
restoring creation authorization and persisting cliff/reference metadata.
Stream reads now load an optional metadata entry, writes maintain it, and
the added code also affects WASM invocation overhead. These behavior fixes
have a measurable cost; the **10% regression tolerance is unchanged**.
The added fixtures cover `upgradeable`, `reclaim_dust`, and
`create_stream_via_factory` (stream WASM with a native factory policy fixture).
They complete the inventory without disabling any resource checks.

Selected changes from the previously committed baseline:

| Entry point | Previous instructions | Repaired instructions | Change |
| --- | ---: | ---: | ---: |
| `create_stream` | 1,108,881 | 1,336,590 | +20.54% |
| `create_stream_with_curve` | 1,050,567 | 1,374,259 | +30.81% |
| `batch_cancel` | 1,071,120 | 1,494,315 | +39.51% |
| `get_stream` | 566,639 | 664,240 | +17.22% |
| `stream_count` | 504,084 | 554,643 | +10.03% |

The full diff is in the baseline JSON. These local, single-call fixtures do not
claim a worst-case transaction cost; max-sized batches are separately checked
against the SDK network limits by `test::resource_limits`.
