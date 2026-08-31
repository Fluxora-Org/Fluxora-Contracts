# PR Summary: Triage Ignored Tests (#1518)

Closes #1518.

## Classification Table

| Test File | Tests | Classification | Reason |
| --- | --- | --- | --- |
| `contracts/stream/src/test_token_edge_cases.rs` | All 11 tests | Obsolete | The entire file and the behaviour it covered was genuinely removed in the v1 rewrite (commit `84ff481`). See `docs/MIGRATION.md` for the full deletion audit. |
| `contracts/stream/src/test_withdrawable_props.rs` | All 5 tests | Obsolete | The entire file and the behaviour it covered was genuinely removed in the v1 rewrite (commit `84ff481`). See `docs/MIGRATION.md` for the full deletion audit. |

## Verification
- `cargo test --workspace` passes because these files and their ignored tests have already been deleted.
- The repo-wide ignored count has already dropped by 16 as a result of the v1 rewrite.
- No assertion was weakened.
- All deletions cite the removed behaviour (v1 rewrite, protocol 27).
- No test in scope retains the generic "Needs dedicated triage" message.
