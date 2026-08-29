#![no_std]
//! # Archival probe — **not part of the product**
//!
//! This contract exists for exactly one reason: to prove, against a real
//! network, the one thing Fluxora's unit tests structurally cannot.
//!
//! ## What it is for
//!
//! The SDK's test host runs storage in recording mode, where reading an expired
//! persistent entry is silently auto-restored rather than failing. So the unit
//! suite proves that crossing the archive/restore boundary preserves accounting,
//! but it never sees the live-network sequence:
//!
//! ```text
//!   read archived entry  ->  transaction FAILS
//!   resubmit with RestoreFootprint
//!   read again           ->  succeeds, data intact
//! ```
//!
//! See `docs/KNOWN-LIMITATIONS.md` §1.
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
//! archives and is never restored, nothing is lost. Do not build on it, and do
//! not deploy it to mainnet.
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
    /// Once the entry archives, invoking this fails at the network level before
    /// the contract body runs — the caller must resubmit with a
    /// `RestoreFootprint` operation. That failure is the behaviour under test.
    pub fn read(env: Env) -> Result<Symbol, Error> {
        env.storage()
            .persistent()
            .get(&Key::Canary)
            .ok_or(Error::NotPlanted)
    }

    /// Whether the canary is present and live.
    ///
    /// Mirrors `FluxoraStream::stream_exists`: this is the signal an SDK keys
    /// on to tell "never existed" apart from "archived, needs restoring".
    pub fn planted(env: Env) -> bool {
        env.storage().persistent().has(&Key::Canary)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::symbol_short;

    #[test]
    fn plant_and_read_round_trip() {
        let env = Env::default();
        let id = env.register(ArchivalProbe, ());
        let client = ArchivalProbeClient::new(&env, &id);

        assert!(!client.planted());
        assert_eq!(client.try_read().unwrap_err().unwrap(), Error::NotPlanted);

        client.plant(&symbol_short!("canary"));
        assert!(client.planted());
        assert_eq!(client.read(), symbol_short!("canary"));
    }

    /// The probe must never extend its own TTL — that omission is what makes it
    /// archive on the network's schedule instead of the contract's.
    #[test]
    fn the_probe_does_not_extend_its_own_ttl() {
        use soroban_sdk::testutils::storage::Persistent as _;
        use soroban_sdk::testutils::Ledger as _;

        let env = Env::default();
        let id = env.register(ArchivalProbe, ());
        ArchivalProbeClient::new(&env, &id).plant(&symbol_short!("canary"));

        let ttl = env.as_contract(&id, || env.storage().persistent().get_ttl(&Key::Canary));
        let min = env.ledger().get().min_persistent_entry_ttl;

        assert!(
            ttl < min,
            "probe TTL {ttl} exceeds the network minimum {min}; something is \
             extending it and the probe will not archive on schedule",
        );
    }
}
