#![cfg(test)]
extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Events as _, Ledger as _},
    token, vec, Address, Env,
};

// ⬇ (A) adjust these imports if your crate paths differ (DataKey may be crate::storage::DataKey)
use crate::{DataKey, Error, FluxoraStream, FluxoraStreamClient};

const DEPOSIT: i128 = 1_000_000_000;
const DAY: u64 = 86_400;

struct Fixture<'a> {
    env: Env,
    client: FluxoraStreamClient<'a>,
    contract_id: Address,
    token: Address,
    sender: Address,
    recipient: Address,
}

fn setup<'a>() -> Fixture<'a> {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = 1_000);

    let contract_id = env.register(FluxoraStream, ());
    let client = FluxoraStreamClient::new(&env, &contract_id);
    // ⬇ (B) if existing tests call an init/setup function on the client here, copy that line

    let token_admin = Address::generate(&env);
    let token = env.register_stellar_asset_contract_v2(token_admin).address();
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    token::StellarAssetClient::new(&env, &token).mint(&sender, &(DEPOSIT * 10));

    Fixture { env, client, contract_id, token, sender, recipient }
}

fn make_stream(f: &Fixture) -> u64 {
    let now = f.env.ledger().timestamp();
    // ⬇ (C) COPY the create_stream call from an existing test (same argument order and count)
    f.client.create_stream(
        &f.sender, &f.recipient, &f.token, &DEPOSIT,
        &now, &(now + DAY), &now,
        &true, &true, &true,
    )
}

/// Simulates archival: the persistent entry is gone, but the id is still < stream_count().
/// ⬇ (D) if Step 2 found an existing archival helper, call that instead of this body.
fn archive_stream(f: &Fixture, id: u64) {
    f.env.as_contract(&f.contract_id, || {
        f.env.storage().persistent().remove(&DataKey::Stream(id));
    });
}

fn stream_ttl(f: &Fixture, id: u64) -> u32 {
    f.env.as_contract(&f.contract_id, || {
        f.env.storage().persistent().get_ttl(&DataKey::Stream(id))
    })
}

#[test]
fn batch_extend_ttl_skips_an_archived_stream_and_extends_the_live_ones() {
    let f = setup();
    let live_a = make_stream(&f);
    let archived = make_stream(&f);
    let live_b = make_stream(&f);

    let tok = token::Client::new(&f.env, &f.token);
    let pool_before = tok.balance(&f.contract_id);
    let sender_before = tok.balance(&f.sender);
    let recipient_before = tok.balance(&f.recipient);
    let total_before = pool_before + sender_before + recipient_before;
    assert_eq!(pool_before, 3 * DEPOSIT);

    // --- one stream falls out of the live ledger -----------------------------
    archive_stream(&f, archived);
    assert!(!f.client.stream_exists(&archived));
    assert!(archived < f.client.stream_count()); // archived, not "never issued"

    let a_before = f.client.get_stream(&live_a);
    let b_before = f.client.get_stream(&live_b);

    // Age the entries so an extension is observable.
    f.env.ledger().with_mut(|l| l.sequence_number += 100_000);
    let ttl_a_before = stream_ttl(&f, live_a);
    let ttl_b_before = stream_ttl(&f, live_b);

    // --- the keeper's mixed batch: live, archived, live -----------------------
    let ids = vec![&f.env, live_a, archived, live_b];
    let extended = f.client.batch_extend_ttl(&ids);
    let events = f.env.events().all(); // read immediately after the call

    // 1. Result matches ABI.md: archived id skipped, not counted, call does not fail.
    assert_eq!(extended, 2);

    // 2. Live streams were extended.
    assert!(stream_ttl(&f, live_a) > ttl_a_before);
    assert!(stream_ttl(&f, live_b) > ttl_b_before);

    // 3. The archived stream was neither restored nor resurrected.
    assert!(!f.client.stream_exists(&archived));
    assert_eq!(f.client.try_get_stream(&archived), Err(Ok(Error::StreamNotFound)));

    // 4. TTL only: no stream data changed.
    assert_eq!(f.client.get_stream(&live_a), a_before);
    assert_eq!(f.client.get_stream(&live_b), b_before);

    // 5. Funds conservation: no token moved anywhere.
    assert_eq!(tok.balance(&f.contract_id), pool_before);
    assert_eq!(tok.balance(&f.sender), sender_before);
    assert_eq!(tok.balance(&f.recipient), recipient_before);
    assert_eq!(
        tok.balance(&f.contract_id) + tok.balance(&f.sender) + tok.balance(&f.recipient),
        total_before
    );

    // 6. Events. ABI.md only documents `ttl_extended` for the single-stream call, so observe first:
    std::println!("EVENTS = {:?}", events);
    // After running once (Step 5), replace the println above with an assertion, e.g.
    //   exactly 2 ttl_extended events (live_a, live_b) and none naming `archived`, OR
    //   no events at all if batch_extend_ttl emits none.
}

#[test]
fn batch_extend_ttl_with_only_archived_streams_extends_nothing_and_does_not_fail() {
    let f = setup();
    let a = make_stream(&f);
    let b = make_stream(&f);
    archive_stream(&f, a);
    archive_stream(&f, b);

    let extended = f.client.batch_extend_ttl(&vec![&f.env, a, b]);

    assert_eq!(extended, 0);
    assert!(!f.client.stream_exists(&a));
    assert!(!f.client.stream_exists(&b));
}