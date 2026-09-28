#![cfg(test)]

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
