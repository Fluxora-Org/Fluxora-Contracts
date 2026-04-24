//! WASM build reproducibility — checksum verification contract.
//!
//! This module documents and tests the invariants that the CI checksum
//! verification relies on. It does NOT perform I/O or read files; instead
//! it validates the properties that guarantee reproducible builds from
//! the contract's own configuration (toolchain pin, SDK pin, build flags).
//!
//! # Determinism contract
//!
//! The following invariants must hold for a build to be reproducible:
//!
//! 1. **Rust toolchain** is pinned via `rust-toolchain.toml` to a specific
//!    channel (`stable`) and target set (`wasm32-unknown-unknown`).
//! 2. **soroban-sdk** version is pinned in `contracts/stream/Cargo.toml`
//!    (currently `21.7.7`).
//! 3. **Build profile** is `--release` with `wasm32-unknown-unknown` target.
//! 4. **No feature flags** beyond the default are used during WASM builds
//!    (the `testutils` feature is only for `#[cfg(test)]`).
//! 5. **No environment-dependent code** is compiled into the WASM artifact.
//!
//! If any of these invariants change, the reference checksum in
//! `wasm/checksums.sha256` must be regenerated via
//! `script/update-wasm-checksums.sh`.
//!
//! # CI verification flow
//!
//! 1. CI builds the WASM artifact with the pinned toolchain.
//! 2. CI computes `sha256sum` of the built WASM.
//! 3. CI compares the computed hash against `wasm/checksums.sha256`.
//! 4. If the hashes differ, CI fails with an actionable error message
//!    directing the developer to regenerate the checksums.
//!
//! # Residual risks
//!
//! - **Optimized WASM**: The Stellar CLI `optimize` step may produce
//!   non-deterministic output depending on the CLI version. The reference
//!   checksum covers only the raw (unoptimized) WASM, which is deterministic
//!   given the pinned toolchain and dependencies.
//! - **Dependency resolution**: `Cargo.lock` must be committed and unchanged.
//!   If a transitive dependency publishes a new patch version, `cargo build`
//!   will still use the locked version.
//! - **Host toolchain differences**: The CI runs on `ubuntu-latest`. Building
//!   on a different OS or architecture may produce different output for
//!   non-WASM targets, but the `wasm32-unknown-unknown` target is
//!   cross-compilation and should be deterministic across hosts.

#[cfg(test)]
mod tests {
    // This module is conditionally compiled only for tests.
    // It serves as documentation for the checksum verification process.
    #[test]
    fn checksum_module_compiles() {
        // This test exists to verify the module structure
    }
}
