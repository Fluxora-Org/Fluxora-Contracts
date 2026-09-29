#![no_std]
//! # Archival probe — **not part of the product**
//!
//! This contract exists for exactly one reason: to prove, against a real
//! network, the one thing Fluxora's unit tests structurally cannot.
//!
//! ## What it is for
//!
//! The SDK's test host runs storage in recording mode, where reading an expired
//! persistent entry is silently auto-restored rather than failing. The unit
//! suite therefore proves that crossing the archive/restore boundary preserves
//! accounting, but on its own it cannot say what a live network does at that
//! boundary.
//!
//! On testnet, 2026-09-28, it turned out to do the same thing (protocol 23 and
//! later): an invocation that touched an archived entry restored it in place,
//! succeeded, and returned the value intact. There was no failed read and no
//! `RestoreFootprint` resubmission. This probe is what established that; see
//! `docs/KNOWN-LIMITATIONS.md` §1 for the recorded run and the reasoning.
//!
//! ```text
//!   read archived entry  ->  transaction succeeds, entry restored by it
//! ```
//!
//! The probe now serves as the record of that measurement, and as the thing
//! `script/archival-canary.sh --round-trip` re-asserts if anyone wants to know
//! whether a future protocol changed it back.
//!
//! ## Why a separate contract
//!
//! Fluxora floors every stream entry's TTL at 30 days, and the network floors
//! *any* persistent entry at `min_persistent_ttl` — 120,960 ledgers, about 7
//! days, on both testnet and local quickstart. A real Fluxora stream therefore
//! cannot archive for a month.
//!
//! This probe deliberately does the one thing Fluxora never does: it writes a
//! persistent entry and **never extends its TTL**. The entry then lives exactly
//! `min_persistent_ttl` and archives as early as the network permits. The
//! restore mechanism it exercises is identical for any persistent entry — it is
//! a property of the ledger, not of the contract — so proving it here proves it
//! for `DataKey::Stream(id)`.
//!
//! ## What it deliberately does not do
//!
//! No auth, no tokens, no value of any kind. It holds a single symbol. If it
//! archives and is never touched again, nothing is lost. Do not build on it,
//! and do not deploy it to mainnet.
//!
//! ## Release isolation (issue #1543)
//!
//! This probe is a workspace member but is **not** part of the deployable product
//! contract list. It stays a member so its smoke test remains wired into the
//! standard workspace checks (`cargo test --workspace`, `cargo fmt --all`,
//! `cargo clippy --all-targets`), but `script/release.sh` — the only command that
//! produces release artifacts — builds **only** the `fluxora-stream` package and
//! rejects any probe wasm among its outputs. To build this probe explicitly, use:
//!
//! ```text
//! cargo build -p fluxora-archival-probe --target wasm32v1-none --release
//! ```
//!
//! and for the live-network round trip, `script/archival-canary.sh`.

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Env, Symbol};

#[contracttype]
#[derive(Clone)]
pub enum Key {
    /// The single persistent entry whose archival we are waiting for.
    Canary,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    /// The canary has never been planted.
    NotPlanted = 1,
}

#[contract]
pub struct ArchivalProbe;

#[contractimpl]
impl ArchivalProbe {
    /// Write the canary. **Deliberately does not extend the entry's TTL**, so
    /// it receives exactly the network's `min_persistent_ttl` and begins the
    /// shortest possible countdown to archival.
    ///
    /// Permissionless: there is nothing here worth protecting.
    pub fn plant(env: Env, note: Symbol) {
        env.storage().persistent().set(&Key::Canary, &note);
        // No extend_ttl call. That omission is the entire point of this
        // contract; do not "fix" it.
    }

    /// Read the canary.
    ///
    /// If the entry has archived, this is the invocation that restores it: the
    /// ledger resurrects the archived entries named in the transaction footprint
    /// and the call proceeds, charging the rent to this transaction. The read
    /// does not fail, which is the behaviour the canary measured.
    pub fn read(env: Env) -> Result<Symbol, Error> {
        env.storage()
            .persistent()
            .get(&Key::Canary)
            .ok_or(Error::NotPlanted)
    }

    /// Whether the canary is present and live.
    ///
    /// Mirrors `FluxoraStream::stream_exists`. It is **not** an archived-state
    /// signal: a read is what restores an archived entry, so a caller polling
    /// this cannot observe the archived state at all — it answers `true` either
    /// way. Pinned by `test::presence_stays_true_across_archival`.
    pub fn planted(env: Env) -> bool {
        env.storage().persistent().has(&Key::Canary)
    }
}

#[cfg(test)]
mod test;
