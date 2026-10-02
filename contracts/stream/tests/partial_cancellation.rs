//! Acceptance tests for issue #1814 — partial cancellation
//! (`FluxoraStream::reduce_stream`).
//!
//! The issue asks for a way for a sender to reclaim part of a stream's
//! **unvested** principal while leaving the stream running. The four criteria
//! are:
//!
//! * the sender can reclaim a stated amount of unvested principal;
//! * vested-but-unwithdrawn funds are never reclaimable;
//! * the remaining schedule is recomputed (and documented on the entry point);
//! * an event records the reduction.
//!
//! These tests drive the contract through its public API only. That keeps them
//! independent of the in-repo `src/test` harness, which is currently
//! inconsistent on this synthetic history.

use fluxora_stream::{Error, FluxoraStream, FluxoraStreamClient};
use soroban_sdk::testutils::{Address as _, Events as _, Ledger as _};
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::xdr::ContractEventBody;
use soroban_sdk::{Address, Env, Symbol, TryFromVal, TryIntoVal, Val};

const T0: u64 = 1_700_000_000;

struct Fixture<'a> {
    env: Env,
    client: FluxoraStreamClient<'a>,
    contract_id: Address,
    token: Address,
    token_client: TokenClient<'a>,
    sender: Address,
    recipient: Address,
    start: u64,
}

/// Deploy the stream contract and a SAC token, fund the sender, and create one
/// `1000` stroop / `100` second stream starting now. At `start + 50` exactly
/// half has vested, which makes the arithmetic in every assertion exact.
fn setup() -> Fixture<'static> {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(T0);

    let contract_id = env.register(FluxoraStream, ());
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let issuer = Address::generate(&env);
    let asset = env.register_stellar_asset_contract_v2(issuer);
    let token = asset.address();
    let token_admin = StellarAssetClient::new(&env, &token);
    let token_client = TokenClient::new(&env, &token);

    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    token_admin.mint(&sender, &1_000_000);

    Fixture {
        env,
        client,
        contract_id,
        token,
        token_client,
        sender,
        recipient,
        start: T0,
    }
}

impl Fixture<'_> {
    /// Create the standard `1000 over 100s` cancellable stream.
    fn create(&self) -> u64 {
        self.client.create_stream(
            &self.sender,
            &self.recipient,
            &self.token,
            &1_000,
            &self.start,
            &(self.start + 100),
            &self.start,
            &true,
            &true,
            &true,
            &None,
        )
    }

    fn pool(&self) -> i128 {
        self.token_client.balance(&self.contract_id)
    }
}

/// The core acceptance test: reduce mid-schedule and assert vested is
/// unchanged while refundable falls by exactly the stated amount.
#[test]
fn reduce_reclaims_only_unvested_and_recomputes_the_schedule() {
    let f = setup();
    let id = f.create();

    // Halfway: vested 500, refundable 500.
    f.env.ledger().set_timestamp(f.start + 50);
    assert_eq!(f.client.vested_of(&id), 500);
    assert_eq!(f.client.refundable_of(&id), 500);

    let sender_before = f.token_client.balance(&f.sender);
    f.client.reduce_stream(&id, &100);

    // Vested is untouched; refundable falls by exactly 100.
    assert_eq!(f.client.vested_of(&id), 500, "vested must be unchanged");
    assert_eq!(
        f.client.refundable_of(&id),
        400,
        "refundable must fall by exactly the reclaimed amount",
    );

    // The remaining schedule is recomputed: 900 over the rate-preserving
    // 90-second duration, so `vested(now)` stays exactly 500.
    let stream = f.client.get_stream(&id);
    assert_eq!(stream.deposited, 900);
    assert_eq!(stream.end_time, f.start + 90);

    // The sender got the 100 back and the pool holds only what it still owes.
    assert_eq!(f.token_client.balance(&f.sender), sender_before + 100);
    assert_eq!(f.pool(), 900);

    // The stream keeps running: it still vests after the reduction.
    f.env.ledger().set_timestamp(f.start + 90);
    assert_eq!(f.client.vested_of(&id), 900);
    let withdrawn = f.client.withdraw(&id, &None);
    assert_eq!(withdrawn, 900);
}

/// Vested-but-unwithdrawn funds are never reclaimable.
#[test]
fn reduce_rejects_more_than_the_unvested_remainder() {
    let f = setup();
    let id = f.create();
    f.env.ledger().set_timestamp(f.start + 50);

    // 500 has vested; 501 is not the sender's to take.
    assert_eq!(
        f.client.try_reduce_stream(&id, &501),
        Err(Ok(Error::ReductionExceedsRefundable)),
    );
    assert_eq!(
        f.client.vested_of(&id),
        500,
        "a refused call changes nothing"
    );
    assert_eq!(f.client.refundable_of(&id), 500);

    // The boundary — reclaiming the whole unvested remainder — is allowed and
    // collapses the schedule onto the current instant without cancelling.
    f.client.reduce_stream(&id, &500);
    assert_eq!(f.client.vested_of(&id), 500);
    assert_eq!(f.client.refundable_of(&id), 0);
    let stream = f.client.get_stream(&id);
    assert_eq!(stream.deposited, 500);
    assert_eq!(stream.end_time, f.start + 50);
}

/// `reduce_stream` is gated exactly like `cancel`.
#[test]
fn reduce_is_gated_like_cancel() {
    let f = setup();
    let id = f.create();
    f.env.ledger().set_timestamp(f.start + 50);

    assert_eq!(
        f.client.try_reduce_stream(&id, &0),
        Err(Ok(Error::InvalidAmount)),
    );
    assert_eq!(
        f.client.try_reduce_stream(&id, &-1),
        Err(Ok(Error::InvalidAmount)),
    );

    // A non-cancellable stream cannot be reduced any more than it can be
    // cancelled.
    let locked = f.client.create_stream(
        &f.sender,
        &f.recipient,
        &f.token,
        &1_000,
        &f.start,
        &(f.start + 100),
        &f.start,
        &false,
        &true,
        &true,
        &None,
    );
    assert_eq!(
        f.client.try_reduce_stream(&locked, &100),
        Err(Ok(Error::NotCancellable)),
    );

    // A settled stream refuses the reduction.
    f.client.cancel(&id);
    assert_eq!(
        f.client.try_reduce_stream(&id, &1),
        Err(Ok(Error::StreamTerminated)),
    );
}

/// The operation is delegable through the new `op::REDUCE` bit, and the refund
/// still goes to the stream's sender.
#[test]
fn reduce_can_be_delegated() {
    let f = setup();
    let id = f.create();
    f.env.ledger().set_timestamp(f.start + 50);

    let delegate = Address::generate(&f.env);
    f.client.grant_delegate(
        &id,
        &f.sender,
        &delegate,
        &fluxora_stream::op::REDUCE,
        &None,
    );

    let sender_before = f.token_client.balance(&f.sender);
    f.client.delegate_reduce_stream(&id, &delegate, &100);

    assert_eq!(f.client.refundable_of(&id), 400);
    assert_eq!(f.client.vested_of(&id), 500);
    assert_eq!(f.token_client.balance(&f.sender), sender_before + 100);
}

/// A delegate without the `op::REDUCE` bit cannot reduce the stream.
#[test]
fn reduce_rejects_a_delegate_without_the_bit() {
    let f = setup();
    let id = f.create();
    f.env.ledger().set_timestamp(f.start + 50);

    let delegate = Address::generate(&f.env);
    f.client.grant_delegate(
        &id,
        &f.sender,
        &delegate,
        &fluxora_stream::op::TOP_UP,
        &None,
    );

    assert_eq!(
        f.client.try_delegate_reduce_stream(&id, &delegate, &100),
        Err(Ok(Error::DelegateNotPermitted)),
    );
}

/// A reduction records an event naming the stream and both parties.
#[test]
fn reduce_emits_a_stream_reduced_event() {
    let f = setup();
    let id = f.create();
    f.env.ledger().set_timestamp(f.start + 50);

    f.client.reduce_stream(&id, &100);

    // The invocation also moves tokens, so the token contract publishes its own
    // `transfer` event alongside ours; find the `StreamReduced` one.
    let all = f.env.events().all();
    let events = all.events();
    let wanted = Symbol::new(&f.env, "stream_reduced");
    let mut seen = 0;
    for event in events.iter() {
        let ContractEventBody::V0(body) = &event.body;
        let mut topics = soroban_sdk::vec![&f.env];
        for t in body.topics.iter() {
            topics.push_back(Val::try_from_val(&f.env, t).unwrap());
        }
        if topics.is_empty() {
            continue;
        }
        let name: Symbol = topics.get(0).unwrap().try_into_val(&f.env).unwrap();
        if name == wanted {
            seen += 1;
            // event name + stream_id + sender + recipient.
            assert_eq!(topics.len(), 4);
        }
    }
    assert_eq!(seen, 1, "reduce_stream must emit exactly one StreamReduced");
}
