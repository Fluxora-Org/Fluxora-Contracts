//! Focused tests documenting stream storage invariants.
//!
//! See `docs/storage-invariants.md` and module docs in `storage.rs`.

extern crate std;

use fluxora_stream::{
    CreateStreamParams, FluxoraStream, FluxoraStreamClient, StreamKind, StreamStatus,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

struct Ctx<'a> {
    env: Env,
    client: FluxoraStreamClient<'a>,
    sender: Address,
    recipient: Address,
    token: TokenClient<'a>,
}

impl<'a> Ctx<'a> {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, FluxoraStream);
        let client = FluxoraStreamClient::new(&env, &contract_id);

        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin)
            .address();

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        client.init(&token_id, &admin);

        let sac = StellarAssetClient::new(&env, &token_id);
        sac.mint(&sender, &10_000_000_i128);

        let token = TokenClient::new(&env, &token_id);
        token.approve(&sender, &contract_id, &i128::MAX, &100_000);

        env.ledger().set_timestamp(0);

        Ctx {
            env,
            client,
            sender,
            recipient,
            token,
        }
    }

    fn create_linear(
        &self,
        recipient: &Address,
        deposit: i128,
        threshold: i128,
        end_time: u64,
    ) -> u64 {
        self.client.create_stream(
            &self.sender,
            &CreateStreamParams {
                recipient: recipient.clone(),
                deposit_amount: deposit,
                rate_per_second: 1_i128,
                start_time: 0u64,
                cliff_time: 0u64,
                end_time,
                withdraw_dust_threshold: Some(threshold),
                memo: None,
                metadata: None,
                kind: StreamKind::Linear,
                irrevocable: None,
                witness: None,
            },
        )
    }
}

#[test]
fn total_liabilities_increments_on_create() {
    let ctx = Ctx::setup();
    assert_eq!(ctx.client.get_total_liabilities(), 0);

    let deposit = 10_000_i128;
    ctx.create_linear(&ctx.recipient, deposit, 0, 10_000);

    assert_eq!(
        ctx.client.get_total_liabilities(),
        deposit,
        "TotalLiabilities must increase by deposit_amount on create"
    );
}

#[test]
fn stream_state_round_trips_via_public_api() {
    let ctx = Ctx::setup();
    let deposit = 8_000_i128;
    let end = 8_000u64;
    let stream_id = ctx.create_linear(&ctx.recipient, deposit, 0, end);

    let state = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state.stream_id, stream_id);
    assert_eq!(state.deposit_amount, deposit);
    assert_eq!(state.end_time, end);
    assert_eq!(state.recipient, ctx.recipient);
    assert_eq!(state.status, StreamStatus::Active);

    ctx.env.ledger().set_timestamp(500);
    let accrued = ctx.client.calculate_accrued(&stream_id);
    assert_eq!(accrued, 500);
}

#[test]
fn recipient_index_sorted_after_multiple_creates() {
    let ctx = Ctx::setup();
    let r = ctx.recipient.clone();

    let id0 = ctx.create_linear(&r, 5_000, 0, 5_000);
    let id1 = ctx.create_linear(&r, 5_000, 0, 5_000);
    let id2 = ctx.create_linear(&r, 5_000, 0, 5_000);

    assert!(id0 < id1 && id1 < id2);

    let ids = ctx.client.get_recipient_streams(&r);
    assert_eq!(ids.len(), 3);
    assert_eq!(ids.get(0).unwrap(), id0);
    assert_eq!(ids.get(1).unwrap(), id1);
    assert_eq!(ids.get(2).unwrap(), id2);
}

#[test]
fn terminal_cancelled_stream_bypasses_dust_threshold() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_linear(&ctx.recipient, 1_000, 500, 1_000);

    ctx.env.ledger().set_timestamp(50);
    ctx.client.cancel_stream(&stream_id);

    let withdrawn = ctx.client.withdraw(&stream_id, &None);
    assert_eq!(
        withdrawn, 50,
        "cancelled terminal stream must bypass dust threshold (50 < 500)"
    );
    assert_eq!(ctx.token.balance(&ctx.recipient), 50);
}

#[test]
fn final_drain_bypasses_dust_threshold() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_linear(&ctx.recipient, 1_000, 500, 1_000);

    // Advance to withdraw 900, which is above the threshold (500).
    ctx.env.ledger().set_timestamp(900);
    let withdrawn1 = ctx.client.withdraw(&stream_id, &None);
    assert_eq!(withdrawn1, 900);

    // Advance to end of stream. 100 tokens remain, which is below the threshold (500).
    // Because it's the final drain (remaining balance = 100), the withdrawal should bypass the threshold.
    ctx.env.ledger().set_timestamp(1_000);
    let withdrawn2 = ctx.client.withdraw(&stream_id, &None);
    assert_eq!(
        withdrawn2, 100,
        "final drain must bypass dust threshold (100 < 500)"
    );
    assert_eq!(ctx.token.balance(&ctx.recipient), 1_000);
}

#[test]
fn recipient_index_insertion_is_idempotent() {
    let ctx = Ctx::setup();
    let r = ctx.recipient.clone();
    let id = ctx.create_linear(&r, 1_000, 0, 1_000);

    let ids_before = ctx.client.get_recipient_streams(&r);
    assert_eq!(ids_before.len(), 1);

    // Manually attempt to re-add the same stream ID to the recipient index
    ctx.env.as_contract(&ctx.client.address, || {
        fluxora_stream::storage::add_stream_to_recipient_index(&ctx.env, &r, id, Some(1_000));
    });

    let ids_after = ctx.client.get_recipient_streams(&r);
    assert_eq!(ids_after.len(), 1, "Duplicate stream ID must not be inserted into recipient index");
}

#[test]
fn stream_structural_invariants_validation() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_linear(&ctx.recipient, 1_000, 0, 1_000);

    ctx.env.as_contract(&ctx.client.address, || {
        let mut stream = fluxora_stream::storage::load_stream(&ctx.env, stream_id).unwrap();

        // Valid stream passes
        assert!(fluxora_stream::storage::validate_stream_invariants(&stream).is_ok());

        // Invalid: start_time > end_time
        stream.start_time = 2_000;
        assert!(fluxora_stream::storage::validate_stream_invariants(&stream).is_err());
        stream.start_time = 0;

        // Invalid: withdrawn_amount > deposit_amount
        stream.withdrawn_amount = 2_000;
        assert!(fluxora_stream::storage::validate_stream_invariants(&stream).is_err());
        stream.withdrawn_amount = 0;

        // Invalid: negative deposit_amount
        stream.deposit_amount = -500;
        assert!(fluxora_stream::storage::validate_stream_invariants(&stream).is_err());
    });
}
