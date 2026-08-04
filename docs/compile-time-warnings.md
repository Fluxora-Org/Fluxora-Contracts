Compile-time warning and cleanup policy
This document defines the warning policy for the stream contract and the regression surface for compile-time cleanup. It applies to contracts/stream/src/lib.rs and the storage primitives it consumes from contracts/stream/src/storage.rs.

Current behavior
The stream contract has one canonical definition for each public contract type and each storage primitive:

Contract ABI types (Stream, ContractError, event payloads, and DataKey) are defined at the crate root in lib.rs.
Storage, TTL, index, pooled-stream, and duplicate-ID helpers are defined in storage.rs and made available through pub use storage::*.
lib.rs does not keep shadow copies of those storage helpers. Calls therefore continue to use the same key, TTL, index, and liability implementations as before the cleanup.
Native library compilation is warning-free, and CI runs cargo clippy -p fluxora_stream --lib -- -D warnings as a hard gate.
The only crate-level Clippy exception is clippy::too_many_arguments. Soroban's #[contractimpl] macro generates client methods that mirror the released contract ABI. Reducing those generated argument lists would require changing public entrypoint signatures, so the exception is retained with an explicit reason. It does not suppress rustc warnings, unused code/import warnings, or any other Clippy lint.

Cleanup behavior
Compile-time cleanup is structural only:

Remove duplicate or orphan declarations instead of hiding their diagnostics.
Keep the canonical implementation in storage.rs; do not add a second helper with the same name to lib.rs.
Remove unused imports, parameters, and same-type casts rather than adding broad allow(warnings), allow(unused), or allow(dead_code) attributes to lib.rs.
Treat a new warning in the production stream library as a CI failure.
If a lint must be suppressed for ABI compatibility, scope it to the narrowest practical surface and include a reason.
The policy test is tests/test_compile_time_warnings.py. It locks down the single-owner storage rule, unique compatibility-sensitive declarations, the narrow lint exception, and the hard CI gate.

Storage and state
This cleanup does not migrate or rewrite ledger state.

DataKey order remains append-only and its frozen discriminants remain unchanged.
The live Stream field order remains the released order, including compatibility fields such as is_pooled, metadata, irrevocable, witness, delegation fields, and decommissioned.
TTL values, bump conditions, recipient/sender index updates, reentrancy locking, paused-stream accounting, and total-liability accounting continue to come from storage.rs.
Existing entries are read by the same load_stream, get_config, and index helpers. No new key or conversion is introduced.
A future cleanup that moves a storage helper is safe only when its function body and call semantics remain equivalent. Reordering DataKey variants or Stream fields is not compile-time cleanup; it is a storage compatibility change and must follow the upgrade policy.

Gas and WASM size
Removing duplicate source does not add a host call, storage read/write, event, authorization check, or token transfer to any entrypoint. Runtime gas behavior is therefore expected to remain unchanged. The compiled WASM may become smaller because orphan contract-type metadata and duplicate helper code are no longer emitted.

Gas and size remain protected by the existing controls:

contracts/stream/tests/gas_regression.rs and script/validate_gas.py cover measured operation costs.
script/check-wasm-size.sh enforces the contract WASM budget and reports headroom.
MAX_STREAM_ENTRY_BYTES and its XDR-size regression test continue to bound persistent entry size.
Any measured gas increase is outside the expected surface of this cleanup and must be investigated rather than accepted as a warning-policy side effect.

Upgrade and compatibility
This is a non-breaking internal cleanup:

CONTRACT_VERSION remains 9.
Public entrypoint names, parameters, return values, authorization, and error behavior are unchanged.
ContractError wire values, DataKey discriminants, event topics/payloads, and stored Stream layout are unchanged.
The too_many_arguments exception is retained specifically to avoid an ABI-breaking signature refactor.
No migration or upgrade() call is required solely for this source cleanup.
The ordinary release and upgrade rules still apply if a later warning fix changes any public or stored type. In that case, update docs/ABI_STABILITY.md, docs/storage.md, and docs/upgrade.md, then run the compatibility suites.

Expected regression surface
Surface	Expected result	Regression signal
Native production library	Warning-free compile and Clippy	cargo clippy -p fluxora_stream --lib -- -D warnings fails
Soroban-generated client	Stable method signatures	New ABI/spec diff or client compile failure
Storage and TTL	Same keys, values, and bump behavior	Storage compatibility/invariant test failure
Gas	No added runtime work	Gas baseline exceeds its tolerance
WASM	Same behavior; size may decrease	WASM build or size-budget failure
Upgrade	No version bump or migration	Version/discriminant compatibility test failure
Test/testutils builds	Production rules remain visible; test-only helpers stay feature-gated	Test compilation or feature-gate failure
Maintainer commands
Bash

cargo check -p fluxora_stream --lib
cargo clippy -p fluxora_stream --lib -- -D warnings
cargo build --workspace
cargo build --release -p fluxora_stream --target wasm32-unknown-unknown
pytest tests/test_compile_time_warnings.py -q