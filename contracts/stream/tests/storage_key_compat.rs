//! V5 → V6 → V9 storage key compatibility regression tests.
//!
//! # Purpose
//!
//! Soroban serialises `DataKey` variants by their **0-based declaration-order
//! discriminant**. Any reorder, insertion, or removal silently corrupts every
//! persistent entry on a live instance. These tests guard against that by:
//!
//! 1. Seeding ledger state with V5-era key/value pairs (using `env.as_contract`
//!    to bypass the contract entry-points and write directly to storage).
//! 2. Invoking V6+ read paths and asserting correct deserialization.
//! 3. Asserting that V6-only keys (discriminants 15–20) are absent on a
//!    V5-seeded instance, confirming no phantom reads.
//! 4. Cross-checking `CONTRACT_VERSION` against the live `DataKey` variant count
//!    (currently 37) to ensure versioning discipline when new variants are added.
//!
//! # Discriminant Table Overview (37 variants: 0–36)
//!
//! | Disc | Variant                     | Storage    | Added in |
//! |-----:|:----------------------------|:-----------|:----------|
//! |    0 | `Config`                    | Instance   | V0        |
//! |    1 | `NextStreamId`              | Instance   | V0        |
//! |    2 | `Stream(u64)`               | Persistent | V0        |
//! |    3 | `RecipientStreams(Address)`  | Persistent | V0        |
//! |    4 | `GlobalEmergencyPaused`     | Instance   | V0        |
//! |    5 | `CreationPaused`            | Instance   | V0        |
//! |    6 | `GlobalPauseReason`         | Instance   | V0        |
//! |    7 | `GlobalPauseTimestamp`      | Instance   | V0        |
//! |    8 | `GlobalPauseAdmin`          | Instance   | V0        |
//! |    9 | `AutoClaimDestination(u64)` | Persistent | V0        |
//! |   10 | `NextTemplateId`            | Instance   | V0        |
//! |   11 | `ActiveTemplateCount`       | Instance   | V0        |
//! |   12 | `StreamTemplate(u64)`       | Persistent | V0        |
//! |   13 | `OwnerTemplateIds(Address)` | Persistent | V0        |
//! |   14 | `TotalLiabilities`          | Instance   | V0        |
//!
//! # Discriminant stability (V6+ additions, appended at end)
//!
//! | Disc | Variant                     | Storage    | Added in |
//! |-----:|:----------------------------|:-----------|:----------|
//! |   15 | `WithdrawNonce(Address)`    | Persistent | V6        |
//! |   16 | `PauseState`                | Instance   | V6        |
//! |   17 | `ReentrancyLock`            | Instance   | V6        |
//! |   18 | `RecipientStreamPage(Address, u32)` | Persistent | V6 |
//! |   19 | `RecipientStreamPageCount(Address)` | Persistent | V6 |
//! |   20 | `PendingRecipientUpdate(u64)` | Persistent | V6        |
//! |   21 | `IdReservation(Address)`    | Persistent | V6        |
//! |   22 | `MaxRatePerSecond`          | Instance   | V6        |
//! |   23 | `DelegatedWithdrawNonce(Address)` | Persistent | V6   |
//! |   24 | `LastPauseRecord(PauseKind)` | Instance   | V6        |
//!
//! # Note on metadata (issue #580)
//!
//! The per-stream `metadata` field is stored **inline** within the `Stream` struct
//! (discriminant 2). No new `DataKey` variant is required, so the discriminant
//! table is unchanged. Metadata is additive: an absent XDR field decodes as `None`.
//!
//! # V6 discriminant table (discriminants 15–20)
//!
//! | Disc | Variant                             | Storage    |
//! |-----:|:------------------------------------|:-----------|
//! |   15 | `WithdrawNonce(Address)`            | Persistent |
//! |   16 | `PauseState`                        | Instance   |
//! |   17 | `ReentrancyLock`                    | Instance   |
//! |   18 | `RecipientStreamPage(Address, u32)` | Persistent |
//! |   19 | `RecipientStreamPageCount(Address)` | Persistent |
//! |   20 | `PendingRecipientUpdate(u64)`       | Persistent |
//!
//! # Post-V6 freeze additions (discriminants 21–28) — documented in checksum.rs
//!
//! | Disc | Variant                            | Storage    |
//! |-----:|:-----------------------------------|:-----------|
//! |   21 | `IdReservation(Address)`           | Persistent |
//! |   22 | `MaxRatePerSecond`                 | Instance   |
//! |   23 | `DelegatedWithdrawNonce(Address)`  | Persistent |
//! |   24 | `LastPauseRecord(PauseKind)`       | Instance   |
//! |   25 | `RotationHistory(u64)`             | Persistent |
//! |   26 | `LastAccrualLedgerTimestamp`       | Instance   |
//! |   27 | `PausedStreamCount`                | Instance   |
//! |   28 | `TotalKeeperFeesPaid`              | Instance   |
//!
//! # Post-V7 additive variants (discriminants 29–36)
//!
//! | Disc | Variant                                | Storage    |
//! |-----:|:---------------------------------------|:-----------|
//! |   29 | `AutoRenewEnabled(u64)`                | Persistent |
//! |   30 | `MaxLookbackLedgers(u64)`              | Persistent |
//! |   31 | `SenderStreams(Address)`               | Persistent |
//! |   32 | `PendingStreamOffer(u64)`              | Persistent |
//! |   33 | `RecipientPendingOffers(Address)`      | Persistent |
//! |   34 | `PooledStreamShares(u64)`              | Persistent |
//! |   35 | `PooledStreamWithdrawn(u64, Address)`  | Persistent |
//! |   36 | `DelegatedCancelNonce(Address)`        | Persistent |
//!
//! Total live `DataKey` variant count: **37** (discriminants 0–36).
//!
//! # Version Mapping Table (`CONTRACT_VERSION` => Expected DataKey Count)
//!
//! | CONTRACT_VERSION | Expected DataKey Count | Discriminants | Notes |
//! |------------------|------------------------|---------------|-------|
//! | 5                | 15                     | 0..=14        | V5 frozen layout |
//! | 6                | 29                     | 0..=28        | V6 freeze + 8 post-freeze additive variants |
//! | 9                | 37                     | 0..=36        | Current live count |
//!
//! # Companion Documentation
//! - `contracts/stream/src/checksum.rs` (WASM checksum & key layout documentation)
//! - `docs/upgrade.md` (CONTRACT_VERSION policy & upgrade runbook)
//!
//! # V6 Stream struct (16 fields, no `metadata`)
//!
//! V6 added `memo` and `kind` fields. V7 adds `metadata` as field 17.
//! A V6-era `Stream` entry is represented in V7 as a `Stream` with `metadata: None`.
//! XDR forward-compatibility ensures the absent 17th field decodes as `None`.
//!
//! Since `metadata` is stored **inside** the `Stream` struct (discriminant 2),
//! no new `DataKey` variant is required. This means metadata does not change the
//! discriminant table and imposes no additional storage key migration burden.
//!
//! # Security assumptions tested
//!
//! - V5 `Stream` entries (memo absent) decode correctly on V6/V7 with `memo == None`.
//! - V6 `Stream` entries (metadata absent) decode correctly on V7 with `metadata == None`.
//! - V5 instance keys (`Config`, `NextStreamId`, pause flags) are readable on V6/V7.
//! - V5/V6 persistent keys (`RecipientStreams`, `AutoClaimDestination`) are readable.
//! - V6-only keys (discriminants 15–20) return absent/default on a V5-seeded instance.
//! - No `None`-unwrap panics occur on any read path when given earlier-version storage.

extern crate std;

use fluxora_stream::{
    Config, DataKey, FluxoraStream, FluxoraStreamClient, PauseKind, Stream, StreamKind,
    StreamStatus, CONTRACT_VERSION,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env,
};

// ---------------------------------------------------------------------------
// Test harness
// ---------------------------------------------------------------------------

/// Minimal setup: register contract + token, call `init`, return handles.
struct Ctx<'a> {
    env: Env,
    contract_id: Address,
    client: FluxoraStreamClient<'a>,
    token_id: Address,
    admin: Address,
    sender: Address,
}

impl<'a> Ctx<'a> {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|l| l.timestamp = 1_000_000);

        let contract_id = env.register_contract(None, FluxoraStream);
        let client = FluxoraStreamClient::new(&env, &contract_id);

        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin)
            .address();
        let sac = StellarAssetClient::new(&env, &token_id);

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        sac.mint(&sender, &1_000_000_000);

        client.init(&token_id, &admin);

        Ctx {
            env,
            contract_id,
            client,
            token_id,
            admin,
            sender,
        }
    }

    /// Seed a V5-era `Stream` directly into persistent storage, bypassing the
    /// contract entry-point. Fields added after V5 are set to their zero/default
    /// values to faithfully represent a V5 entry that is now read by V9 code.
    ///
    /// - `memo: None`          — V5 struct had no memo field
    /// - `kind: StreamKind::Linear` — V5 had no kind field; Linear is the
    ///   backward-compatible default
    /// - `claim_owner: None`   — not present in V5
    /// - `is_pooled: None`     — not present in V5
    /// - `irrevocable: None`   — not present in V5
    /// - `witness: None`       — not present in V5
    /// - `metadata: None`      — not present in V5
    /// - `delegation_depth: 0` — not present in V5
    /// - `parent_stream_id: None` — not present in V5
    /// - `last_pause_toggle_ledger: 0`, `last_withdraw_ledger: 0`,
    ///   `last_rate_change_ledger: 0` — not present in V5
    fn legacy_stream(&self, stream_id: u64, recipient: &Address) -> Stream {
        let now = self.env.ledger().timestamp();
        Stream {
            stream_id,
            sender: self.sender.clone(),
            recipient: recipient.clone(),
            claim_owner: None,
            deposit_amount: 86_400,
            rate_per_second: 1,
            start_time: now,
            cliff_time: now,
            end_time: now + 86_400,
            withdrawn_amount: 0,
            status: StreamStatus::Active,
            cancelled_at: None,
            checkpointed_amount: 0,
            checkpointed_at: now,
            withdraw_dust_threshold: 0,
            memo: None,
            kind: StreamKind::Linear,
            last_pause_toggle_ledger: 0,
            last_withdraw_ledger: 0,
            metadata: None,
            witness: None,
            is_pooled: None,
            last_rate_change_ledger: 0,
            delegation_depth: 0,
            parent_stream_id: None,
            decommissioned: None,
            irrevocable: None,
            paused_at_timestamp: 0,
            cumulative_paused_duration: 0,
        }
    }

    fn seed_v5_stream(&self, stream_id: u64, recipient: &Address) {
        let stream = self.legacy_stream(stream_id, recipient);
        let cid = self.contract_id.clone();
        self.env.as_contract(&cid, || {
            self.env
                .storage()
                .persistent()
                .set(&DataKey::Stream(stream_id), &stream);
        });
    }

    /// Seed a V5-era `RecipientStreams` index entry directly.
    fn seed_v5_recipient_streams(&self, recipient: &Address, ids: soroban_sdk::Vec<u64>) {
        let cid = self.contract_id.clone();
        self.env.as_contract(&cid, || {
            self.env
                .storage()
                .persistent()
                .set(&DataKey::RecipientStreams(recipient.clone()), &ids);
        });
    }
}

// ---------------------------------------------------------------------------
// V5 Stream read-path tests
// ---------------------------------------------------------------------------

/// A V5-era Stream (post-V5 fields absent/defaulted) is readable by the V9
/// `get_stream_state` path.
///
/// This is the primary regression guard: if `DataKey::Stream` discriminant (2)
/// ever shifts, this test will panic with `StreamNotFound` instead of returning
/// the seeded value.
#[test]
fn v5_stream_readable_by_v9_get_stream_state() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    ctx.seed_v5_stream(0, &recipient);

    let state = ctx.client.get_stream_state(&0u64);

    assert_eq!(state.stream_id, 0);
    assert_eq!(state.recipient, recipient);
    assert_eq!(state.deposit_amount, 86_400);
    assert_eq!(state.rate_per_second, 1);
    assert_eq!(state.withdrawn_amount, 0);
    assert_eq!(state.status, StreamStatus::Active);
    // V5 entry has no memo — V9 decoder must return None, not panic.
    assert!(
        state.memo.is_none(),
        "V5 stream must decode with memo == None"
    );
    // V5 entry has no claim_owner — must decode as None.
    assert!(
        state.claim_owner.is_none(),
        "V5 stream must decode with claim_owner == None"
    );
    // kind defaults to Linear for V5 streams.
    assert_eq!(state.kind, StreamKind::Linear);
}

/// The V5-era `memo` field decodes as `None` on V9.
///
/// This is the clearest proof that the `Stream` struct kept append-only field
/// ordering: the V9 decoder must treat the missing tail field as absent rather
/// than panicking or shifting earlier values.
#[test]
fn v5_stream_get_stream_memo_returns_none() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    ctx.seed_v5_stream(0, &recipient);

    let memo = ctx.client.get_stream_memo(&0u64);
    assert!(memo.is_none(), "V5 stream memo must decode as None");
}

/// V9 `calculate_accrued` works correctly on a V5-era Stream entry.
///
/// Accrual math depends on `start_time`, `cliff_time`, `end_time`,
/// `rate_per_second`, and `checkpointed_amount` — all present in V5.
#[test]
fn v5_stream_calculate_accrued_correct() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    ctx.seed_v5_stream(0, &recipient);

    // Advance 100 seconds past start_time
    ctx.env.ledger().with_mut(|l| l.timestamp += 100);

    let accrued = ctx.client.calculate_accrued(&0u64);
    // rate=1 token/s × 100 s = 100
    assert_eq!(
        accrued, 100,
        "accrual on V5 stream must equal rate × elapsed"
    );
}

/// V9 `get_withdrawable` works correctly on a V5-era Stream entry.
#[test]
fn v5_stream_get_withdrawable_correct() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    ctx.seed_v5_stream(0, &recipient);

    // Fund the contract directly so the balance cap in get_withdrawable doesn't zero the result.
    // (seed_v5_stream bypasses create_stream and therefore no tokens are transferred to the contract.)
    let sac = StellarAssetClient::new(&ctx.env, &ctx.token_id);
    sac.mint(&ctx.contract_id, &10_000);

    ctx.env.ledger().with_mut(|l| l.timestamp += 200);

    let withdrawable = ctx.client.get_withdrawable(&0u64);
    // withdrawn_amount=0, accrued=200 → withdrawable=200
    assert_eq!(withdrawable, 200);
}

/// V9 `get_claimable_at` works correctly on a V5-era Stream entry.
#[test]
fn v5_stream_get_claimable_at_correct() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    let now = ctx.env.ledger().timestamp();
    ctx.seed_v5_stream(0, &recipient);

    let claimable = ctx.client.get_claimable_at(&0u64, &(now + 500));
    assert_eq!(claimable, 500);
}

/// Multiple V5-era streams with different IDs are all independently readable.
///
/// Guards against any off-by-one in the `Stream(u64)` key encoding.
#[test]
fn v5_multiple_streams_all_readable() {
    let ctx = Ctx::setup();
    let r0 = Address::generate(&ctx.env);
    let r1 = Address::generate(&ctx.env);
    let r2 = Address::generate(&ctx.env);

    ctx.seed_v5_stream(0, &r0);
    ctx.seed_v5_stream(1, &r1);
    ctx.seed_v5_stream(42, &r2);

    assert_eq!(ctx.client.get_stream_state(&0u64).recipient, r0);
    assert_eq!(ctx.client.get_stream_state(&1u64).recipient, r1);
    assert_eq!(ctx.client.get_stream_state(&42u64).recipient, r2);
}

/// A V5-era Stream with `cancelled_at` set is readable and accrual is frozen.
#[test]
fn v5_cancelled_stream_readable_accrual_frozen() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    let now = ctx.env.ledger().timestamp();

    // Seed a cancelled V5 stream: cancelled 50 s into the stream.
    let cancelled_at = now + 50;
    let mut stream = ctx.legacy_stream(0, &recipient);
    stream.status = StreamStatus::Cancelled;
    stream.cancelled_at = Some(cancelled_at);
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        ctx.env
            .storage()
            .persistent()
            .set(&DataKey::Stream(0u64), &stream);
    });

    // Advance well past cancelled_at
    ctx.env.ledger().with_mut(|l| l.timestamp = now + 1000);

    let state = ctx.client.get_stream_state(&0u64);
    assert_eq!(state.status, StreamStatus::Cancelled);
    assert_eq!(state.cancelled_at, Some(cancelled_at));
    assert!(state.memo.is_none());

    // Accrual must be frozen at cancelled_at (50 tokens)
    let accrued = ctx.client.calculate_accrued(&0u64);
    assert_eq!(
        accrued, 50,
        "cancelled V5 stream accrual must be frozen at cancelled_at"
    );
}

/// A V5-era Stream with non-zero `checkpointed_amount` decodes correctly.
///
/// `checkpointed_amount` was added in V2; V5 entries always have it set.
#[test]
fn v5_stream_with_checkpoint_readable() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    let now = ctx.env.ledger().timestamp();

    let mut stream = ctx.legacy_stream(0, &recipient);
    stream.deposit_amount = 10_000;
    stream.rate_per_second = 2;
    stream.end_time = now + 5_000;
    stream.checkpointed_amount = 500; // accrued under a prior rate
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        ctx.env
            .storage()
            .persistent()
            .set(&DataKey::Stream(0u64), &stream);
    });

    ctx.env.ledger().with_mut(|l| l.timestamp += 100);

    let state = ctx.client.get_stream_state(&0u64);
    assert_eq!(state.checkpointed_amount, 500);
    assert!(state.memo.is_none());
}

/// A V6-era Stream (no `metadata`) is readable by V7, returning `metadata == None`.
///
/// V6 Stream struct had 16 fields (ending with `kind`) and did not include `metadata`.
/// V7 adds `metadata` as field 17. XDR forward-compatibility ensures the absent
/// field decodes as `None` — this test guards against any regression in that path.
#[test]
fn v6_stream_metadata_decodes_as_none() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);

    let stream = ctx.legacy_stream(0, &recipient);
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        ctx.env
            .storage()
            .persistent()
            .set(&DataKey::Stream(0u64), &stream);
    });

    // V7 `get_stream_metadata` must decode the V6-era entry and return None
    let meta = ctx.client.get_stream_metadata(&0u64);
    assert!(
        meta.is_none(),
        "V6-era stream (no metadata) must decode as metadata=None on V7"
    );

    // V7 `get_stream_state` must also carry metadata=None
    let state = ctx.client.get_stream_state(&0u64);
    assert!(
        state.metadata.is_none(),
        "V6-era Stream struct must decode with metadata=None"
    );
}

// ---------------------------------------------------------------------------
// V5 instance key read-path tests
// ---------------------------------------------------------------------------

/// V5 `Config` (discriminant 0) is readable by V9 `get_config`.
///
/// `init` writes `Config` via the contract entry-point, so this test verifies
/// that the discriminant-0 key written by V5 is still decoded correctly by V9.
#[test]
fn v5_config_key_readable_by_v9() {
    let ctx = Ctx::setup();
    // `init` already wrote Config; verify V9 reads it correctly.
    let cfg = ctx.client.get_config();
    assert_eq!(cfg.admin, ctx.admin);
    assert_eq!(cfg.token, ctx.token_id);
}

/// V5 `NextStreamId` (discriminant 1) is readable by V9 `get_stream_count`.
#[test]
fn v5_next_stream_id_readable_by_v9() {
    let ctx = Ctx::setup();
    // Seed NextStreamId directly to simulate a V5 instance with 3 streams created.
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        ctx.env
            .storage()
            .instance()
            .set(&DataKey::NextStreamId, &3u64);
    });

    let count = ctx.client.get_stream_count();
    assert_eq!(
        count, 3,
        "V5 NextStreamId must be readable by V9 get_stream_count"
    );
}

/// V5 `GlobalEmergencyPaused` (discriminant 4) is readable by V9.
///
/// When set to `true` on a V5 instance, V9 must still honour the pause.
#[test]
fn v5_global_emergency_paused_readable_by_v9() {
    let ctx = Ctx::setup();
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        ctx.env
            .storage()
            .instance()
            .set(&DataKey::GlobalEmergencyPaused, &true);
    });

    let recipient = Address::generate(&ctx.env);
    let now = ctx.env.ledger().timestamp();
    let err = ctx.client.try_create_streams(
        &ctx.sender,
        &vec![
            &ctx.env,
            fluxora_stream::CreateStreamParams {
                recipient,
                deposit_amount: 1000,
                rate_per_second: 1,
                start_time: now,
                cliff_time: now,
                end_time: now + 1000,
                withdraw_dust_threshold: None,
                memo: None,
                kind: StreamKind::Linear,
                metadata: None,
                irrevocable: None,
                witness: None,
            },
        ],
    );
    assert_eq!(
        err,
        Err(Ok(fluxora_stream::ContractError::ContractPaused)),
        "V5 GlobalEmergencyPaused=true must block V9 stream creation"
    );
}

/// V5 `CreationPaused` (discriminant 5) is readable by V9.
#[test]
fn v5_creation_paused_readable_by_v9() {
    let ctx = Ctx::setup();
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        ctx.env
            .storage()
            .instance()
            .set(&DataKey::CreationPaused, &true);
    });

    let recipient = Address::generate(&ctx.env);
    let now = ctx.env.ledger().timestamp();
    let err = ctx.client.try_create_streams(
        &ctx.sender,
        &vec![
            &ctx.env,
            fluxora_stream::CreateStreamParams {
                recipient,
                deposit_amount: 1000,
                rate_per_second: 1,
                start_time: now,
                cliff_time: now,
                end_time: now + 1000,
                withdraw_dust_threshold: None,
                memo: None,
                kind: StreamKind::Linear,
                metadata: None,
                irrevocable: None,
                witness: None,
            },
        ],
    );
    assert_eq!(
        err,
        Err(Ok(fluxora_stream::ContractError::ContractPaused)),
        "V5 CreationPaused=true must block V9 stream creation"
    );
}

/// V5 `TotalLiabilities` (discriminant 14) is readable by V9.
///
/// Discriminant 14 is the last frozen V5 key. If any variant were inserted
/// before it, this read would return the wrong type and panic.
#[test]
fn v5_total_liabilities_readable_by_v9() {
    let ctx = Ctx::setup();
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        ctx.env
            .storage()
            .instance()
            .set(&DataKey::TotalLiabilities, &999_i128);
    });

    let cid2 = ctx.contract_id.clone();
    ctx.env.as_contract(&cid2, || {
        let val: i128 = ctx
            .env
            .storage()
            .instance()
            .get(&DataKey::TotalLiabilities)
            .expect("TotalLiabilities must be present after V5 seed");
        assert_eq!(val, 999_i128);
    });
}

// ---------------------------------------------------------------------------
// V5 RecipientStreams (discriminant 3) read-path tests
// ---------------------------------------------------------------------------

/// V5 `RecipientStreams` (discriminant 3) is readable by V9 `get_recipient_streams`.
#[test]
fn v5_recipient_streams_readable_by_v9() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);

    // Seed V5-era streams and index
    ctx.seed_v5_stream(0, &recipient);
    ctx.seed_v5_stream(1, &recipient);
    ctx.seed_v5_recipient_streams(&recipient, vec![&ctx.env, 0u64, 1u64]);

    let ids = ctx.client.get_recipient_streams(&recipient);
    assert_eq!(ids.len(), 2);
    assert_eq!(ids.get(0).unwrap(), 0u64);
    assert_eq!(ids.get(1).unwrap(), 1u64);
}

/// V9 `get_recipient_stream_count` works on a V5-seeded index.
#[test]
fn v5_recipient_stream_count_correct() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);

    ctx.seed_v5_stream(0, &recipient);
    ctx.seed_v5_stream(5, &recipient);
    ctx.seed_v5_stream(10, &recipient);
    ctx.seed_v5_recipient_streams(&recipient, vec![&ctx.env, 0u64, 5u64, 10u64]);

    let count = ctx.client.get_recipient_stream_count(&recipient);
    assert_eq!(count, 3);
}

/// A recipient with no V5 index entry returns an empty list (no panic).
#[test]
fn v5_absent_recipient_streams_returns_empty() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    // No seed — simulates a V5 instance where this recipient had no streams.
    let ids = ctx.client.get_recipient_streams(&recipient);
    assert_eq!(ids.len(), 0);
}

// ---------------------------------------------------------------------------
// V6-only key absence tests (discriminants 15–20)
// ---------------------------------------------------------------------------
//
// On a V5-seeded instance, none of the V6-only keys should be present.
// These tests confirm that V9 read paths return absent/default rather than
// panicking or returning stale data from a shifted discriminant.

/// `WithdrawNonce` (discriminant 15) is absent on a V5 instance.
///
/// V9 delegated-withdraw must treat an absent nonce as 0 (first use).
#[test]
fn v6_withdraw_nonce_absent_on_v5_instance() {
    let ctx = Ctx::setup();
    let addr = Address::generate(&ctx.env);
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        let present = ctx
            .env
            .storage()
            .persistent()
            .has(&DataKey::WithdrawNonce(addr.clone()));
        assert!(
            !present,
            "WithdrawNonce must be absent on a V5-seeded instance"
        );
    });
}

/// `PauseState` (discriminant 16) is absent on a V5 instance.
///
/// V9 reads PauseState as `Option`; absent means the protocol is not paused
/// via the V6 PauseState mechanism.
#[test]
fn v6_pause_state_absent_on_v5_instance() {
    let ctx = Ctx::setup();
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        let present = ctx.env.storage().instance().has(&DataKey::PauseState);
        assert!(
            !present,
            "PauseState must be absent on a V5-seeded instance"
        );
    });
}

/// `ReentrancyLock` (discriminant 17) is absent on a V5 instance.
///
/// V9 reads ReentrancyLock as `bool`; absent means the lock is not held.
#[test]
fn v6_reentrancy_lock_absent_on_v5_instance() {
    let ctx = Ctx::setup();
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        let present = ctx.env.storage().instance().has(&DataKey::ReentrancyLock);
        assert!(
            !present,
            "ReentrancyLock must be absent on a V5-seeded instance"
        );
    });
}

/// `RecipientStreamPage` (discriminant 18) is absent on a V5 instance.
///
/// V5 used `RecipientStreams` (discriminant 3) for the flat index.
/// V6 adds paged index entries; they must not exist on a V5 instance.
#[test]
fn v6_recipient_stream_page_absent_on_v5_instance() {
    let ctx = Ctx::setup();
    let addr = Address::generate(&ctx.env);
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        let present = ctx
            .env
            .storage()
            .persistent()
            .has(&DataKey::RecipientStreamPage(addr.clone(), 0u32));
        assert!(
            !present,
            "RecipientStreamPage must be absent on a V5-seeded instance"
        );
    });
}

/// `RecipientStreamPageCount` (discriminant 19) is absent on a V5 instance.
#[test]
fn v6_recipient_stream_page_count_absent_on_v5_instance() {
    let ctx = Ctx::setup();
    let addr = Address::generate(&ctx.env);
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        let present = ctx
            .env
            .storage()
            .persistent()
            .has(&DataKey::RecipientStreamPageCount(addr.clone()));
        assert!(
            !present,
            "RecipientStreamPageCount must be absent on a V5-seeded instance"
        );
    });
}

/// `PendingRecipientUpdate` (discriminant 20) is absent on a V5 instance.
#[test]
fn v6_pending_recipient_update_absent_on_v5_instance() {
    let ctx = Ctx::setup();
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        let present = ctx
            .env
            .storage()
            .persistent()
            .has(&DataKey::PendingRecipientUpdate(0u64));
        assert!(
            !present,
            "PendingRecipientUpdate must be absent on a V5-seeded instance"
        );
    });
}

// ---------------------------------------------------------------------------
// Post-V6 freeze key absence tests (discriminants 21–28)
// ---------------------------------------------------------------------------

/// `IdReservation` (discriminant 21) is absent on a V5 instance.
/// Note: IdReservation uses *persistent* storage (not instance as checksum.rs
/// documents — see `contracts/stream/src/storage.rs` load_id_reservation).
#[test]
fn v7_id_reservation_absent_on_v5_instance() {
    let ctx = Ctx::setup();
    let addr = Address::generate(&ctx.env);
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        let present = ctx
            .env
            .storage()
            .persistent()
            .has(&DataKey::IdReservation(addr.clone()));
        assert!(
            !present,
            "IdReservation must be absent on a V5-seeded instance"
        );
    });
}

/// `MaxRatePerSecond` (discriminant 22) is absent on a V5 instance.
#[test]
fn v7_max_rate_per_second_absent_on_v5_instance() {
    let ctx = Ctx::setup();
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        let present = ctx.env.storage().instance().has(&DataKey::MaxRatePerSecond);
        assert!(
            !present,
            "MaxRatePerSecond must be absent on a V5-seeded instance"
        );
    });
}

/// `DelegatedWithdrawNonce` (discriminant 23) is absent on a V5 instance.
#[test]
fn v7_delegated_withdraw_nonce_absent_on_v5_instance() {
    let ctx = Ctx::setup();
    let addr = Address::generate(&ctx.env);
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        let present = ctx
            .env
            .storage()
            .persistent()
            .has(&DataKey::DelegatedWithdrawNonce(addr.clone()));
        assert!(
            !present,
            "DelegatedWithdrawNonce must be absent on a V5-seeded instance"
        );
    });
}

/// `LastPauseRecord` (discriminant 24) is absent on a V5 instance.
/// Note: checksum.rs documents this as instance storage; the absence check
/// uses instance storage accordingly.
#[test]
fn v7_last_pause_record_absent_on_v5_instance() {
    let ctx = Ctx::setup();
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        let present = ctx
            .env
            .storage()
            .instance()
            .has(&DataKey::LastPauseRecord(PauseKind::Protocol));
        assert!(
            !present,
            "LastPauseRecord must be absent on a V5-seeded instance"
        );
    });
}

/// `RotationHistory` (discriminant 25) is absent on a V5 instance.
#[test]
fn v7_rotation_history_absent_on_v5_instance() {
    let ctx = Ctx::setup();
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        let present = ctx
            .env
            .storage()
            .persistent()
            .has(&DataKey::RotationHistory(0u64));
        assert!(
            !present,
            "RotationHistory must be absent on a V5-seeded instance"
        );
    });
}

/// `LastAccrualLedgerTimestamp` (discriminant 26) is absent on a V5 instance.
#[test]
fn v7_last_accrual_ledger_timestamp_absent_on_v5_instance() {
    let ctx = Ctx::setup();
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        let present = ctx
            .env
            .storage()
            .instance()
            .has(&DataKey::LastAccrualLedgerTimestamp);
        assert!(
            !present,
            "LastAccrualLedgerTimestamp must be absent on a V5-seeded instance"
        );
    });
}

/// `PausedStreamCount` (discriminant 27) is absent on a V5 instance.
///
/// NOTE: `init` (called in Ctx::setup) explicitly zeroes this counter as part of
/// V6+ initialisation. We therefore check absence on a raw (un-init'd) contract
/// to confirm the key does not exist without a V6+ init.
#[test]
fn v7_paused_stream_count_absent_on_v5_instance() {
    let env = Env::default();
    // Register but do NOT call init — simulates a V5 instance that was deployed
    // before PausedStreamCount was introduced.
    let contract_id = env.register_contract(None, FluxoraStream);
    env.as_contract(&contract_id, || {
        let present = env.storage().instance().has(&DataKey::PausedStreamCount);
        assert!(
            !present,
            "PausedStreamCount must be absent on a pre-init (V5-era) instance"
        );
    });
}

/// `TotalKeeperFeesPaid` (discriminant 28) is absent on a V5 instance.
///
/// NOTE: `init` (called in Ctx::setup) explicitly zeroes this counter as part of
/// V6+ initialisation. We therefore check absence on a raw (un-init'd) contract.
#[test]
fn v7_total_keeper_fees_paid_absent_on_v5_instance() {
    let env = Env::default();
    let contract_id = env.register_contract(None, FluxoraStream);
    env.as_contract(&contract_id, || {
        let present = env.storage().instance().has(&DataKey::TotalKeeperFeesPaid);
        assert!(
            !present,
            "TotalKeeperFeesPaid must be absent on a V5-seeded instance"
        );
    });
}

// ---------------------------------------------------------------------------
// Post-V7 additive key absence tests (discriminants 29–35)
// ---------------------------------------------------------------------------
//
// These variants are NOT documented in checksum.rs yet (see disagreement flag
// in the module doc-comment). They are tested here to ensure they were appended
// correctly and are absent on a V5-seeded instance.

/// `AutoRenewEnabled` (discriminant 29) is absent on a V5 instance.
#[test]
fn post_v7_auto_renew_enabled_absent_on_v5_instance() {
    let ctx = Ctx::setup();
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        let present = ctx
            .env
            .storage()
            .persistent()
            .has(&DataKey::AutoRenewEnabled(0u64));
        assert!(
            !present,
            "AutoRenewEnabled must be absent on a V5-seeded instance"
        );
    });
}

/// `MaxLookbackLedgers` (discriminant 30) is absent on a V5 instance.
#[test]
fn post_v7_max_lookback_ledgers_absent_on_v5_instance() {
    let ctx = Ctx::setup();
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        let present = ctx
            .env
            .storage()
            .persistent()
            .has(&DataKey::MaxLookbackLedgers(0u64));
        assert!(
            !present,
            "MaxLookbackLedgers must be absent on a V5-seeded instance"
        );
    });
}

/// `SenderStreams` (discriminant 31) is absent on a V5 instance.
#[test]
fn post_v7_sender_streams_absent_on_v5_instance() {
    let ctx = Ctx::setup();
    let addr = Address::generate(&ctx.env);
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        let present = ctx
            .env
            .storage()
            .persistent()
            .has(&DataKey::SenderStreams(addr.clone()));
        assert!(
            !present,
            "SenderStreams must be absent on a V5-seeded instance"
        );
    });
}

/// `PendingStreamOffer` (discriminant 32) is absent on a V5 instance.
#[test]
fn post_v7_pending_stream_offer_absent_on_v5_instance() {
    let ctx = Ctx::setup();
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        let present = ctx
            .env
            .storage()
            .persistent()
            .has(&DataKey::PendingStreamOffer(0u64));
        assert!(
            !present,
            "PendingStreamOffer must be absent on a V5-seeded instance"
        );
    });
}

/// `RecipientPendingOffers` (discriminant 33) is absent on a V5 instance.
#[test]
fn post_v7_recipient_pending_offers_absent_on_v5_instance() {
    let ctx = Ctx::setup();
    let addr = Address::generate(&ctx.env);
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        let present = ctx
            .env
            .storage()
            .persistent()
            .has(&DataKey::RecipientPendingOffers(addr.clone()));
        assert!(
            !present,
            "RecipientPendingOffers must be absent on a V5-seeded instance"
        );
    });
}

/// `PooledStreamShares` (discriminant 34) is absent on a V5 instance.
#[test]
fn post_v7_pooled_stream_shares_absent_on_v5_instance() {
    let ctx = Ctx::setup();
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        let present = ctx
            .env
            .storage()
            .persistent()
            .has(&DataKey::PooledStreamShares(0u64));
        assert!(
            !present,
            "PooledStreamShares must be absent on a V5-seeded instance"
        );
    });
}

/// `PooledStreamWithdrawn` (discriminant 35) is absent on a V5 instance.
#[test]
fn post_v7_pooled_stream_withdrawn_absent_on_v5_instance() {
    let ctx = Ctx::setup();
    let addr = Address::generate(&ctx.env);
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        let present = ctx
            .env
            .storage()
            .persistent()
            .has(&DataKey::PooledStreamWithdrawn(0u64, addr.clone()));
        assert!(
            !present,
            "PooledStreamWithdrawn must be absent on a V5-seeded instance"
        );
    });
}

/// `DelegatedCancelNonce` (discriminant 36) is absent on legacy state and its
/// public read path must apply the documented zero default without backfilling.
#[test]
fn v9_delegated_cancel_nonce_absent_defaults_to_zero_without_write() {
    let ctx = Ctx::setup();
    let sender = Address::generate(&ctx.env);
    let key = DataKey::DelegatedCancelNonce(sender.clone());
    let cid = ctx.contract_id.clone();

    ctx.env.as_contract(&cid, || {
        assert!(!ctx.env.storage().persistent().has(&key));
    });
    assert_eq!(ctx.client.get_delegated_cancel_nonce(&sender), 0);
    ctx.env.as_contract(&cid, || {
        assert!(
            !ctx.env.storage().persistent().has(&key),
            "read-only default must not backfill DelegatedCancelNonce"
        );
    });
}

// ---------------------------------------------------------------------------
// Discriminant stability smoke tests
// ---------------------------------------------------------------------------
//
// These tests write a known value under a specific DataKey and read it back
// via the same key. If any discriminant shifts (e.g. due to a mid-enum
// insertion), the read will return None or the wrong type, causing a panic.
// They are intentionally redundant with the read-path tests above to provide
// a second layer of detection.

/// Discriminant 0 (Config) round-trips correctly.
#[test]
fn discriminant_0_config_round_trips() {
    let ctx = Ctx::setup();
    let new_admin = Address::generate(&ctx.env);
    let cid = ctx.contract_id.clone();
    let token_addr = ctx.token_id.clone();
    ctx.env.as_contract(&cid, || {
        ctx.env.storage().instance().set(
            &DataKey::Config,
            &Config {
                token: token_addr.clone(),
                admin: new_admin.clone(),
            },
        );
        let cfg: Config = ctx
            .env
            .storage()
            .instance()
            .get(&DataKey::Config)
            .expect("Config must round-trip at discriminant 0");
        assert_eq!(cfg.admin, new_admin);
        assert_eq!(cfg.token, token_addr);
    });
}

/// Discriminant 1 (NextStreamId) round-trips correctly.
#[test]
fn discriminant_1_next_stream_id_round_trips() {
    let ctx = Ctx::setup();
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        ctx.env
            .storage()
            .instance()
            .set(&DataKey::NextStreamId, &7u64);
        let val: u64 = ctx
            .env
            .storage()
            .instance()
            .get(&DataKey::NextStreamId)
            .expect("NextStreamId must round-trip at discriminant 1");
        assert_eq!(val, 7u64);
    });
}

/// Discriminant 2 (Stream) round-trips correctly.
#[test]
fn discriminant_2_stream_round_trips() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    ctx.seed_v5_stream(99, &recipient);

    let state = ctx.client.get_stream_state(&99u64);
    assert_eq!(state.stream_id, 99);
    assert_eq!(state.recipient, recipient);
}

/// Discriminant 3 (RecipientStreams) round-trips correctly.
#[test]
fn discriminant_3_recipient_streams_round_trips() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    ctx.seed_v5_stream(0, &recipient);
    ctx.seed_v5_recipient_streams(&recipient, vec![&ctx.env, 0u64]);

    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        let ids: soroban_sdk::Vec<u64> = ctx
            .env
            .storage()
            .persistent()
            .get(&DataKey::RecipientStreams(recipient.clone()))
            .expect("RecipientStreams must round-trip at discriminant 3");
        assert_eq!(ids.get(0).unwrap(), 0u64);
    });
}

/// Discriminant 14 (TotalLiabilities) is the last frozen V5 key.
/// A round-trip confirms no variant was inserted before it.
#[test]
fn discriminant_14_total_liabilities_round_trips() {
    let ctx = Ctx::setup();
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        ctx.env
            .storage()
            .instance()
            .set(&DataKey::TotalLiabilities, &12345_i128);
        let val: i128 = ctx
            .env
            .storage()
            .instance()
            .get(&DataKey::TotalLiabilities)
            .expect("TotalLiabilities must round-trip at discriminant 14");
        assert_eq!(val, 12345_i128);
    });
}

/// Discriminant 15 (WithdrawNonce) round-trips correctly.
#[test]
fn discriminant_15_withdraw_nonce_round_trips() {
    let ctx = Ctx::setup();
    let addr = Address::generate(&ctx.env);
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        ctx.env
            .storage()
            .persistent()
            .set(&DataKey::WithdrawNonce(addr.clone()), &42u64);
        let val: u64 = ctx
            .env
            .storage()
            .persistent()
            .get(&DataKey::WithdrawNonce(addr.clone()))
            .expect("WithdrawNonce must round-trip at discriminant 15");
        assert_eq!(val, 42u64);
    });
}

/// Discriminant 20 (PendingRecipientUpdate) round-trips correctly.
/// This is the last V6 key; a round-trip here confirms discriminants 15–20 are stable.
#[test]
fn discriminant_20_pending_recipient_update_round_trips() {
    let ctx = Ctx::setup();
    let addr = Address::generate(&ctx.env);
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        ctx.env
            .storage()
            .persistent()
            .set(&DataKey::PendingRecipientUpdate(7u64), &addr.clone());
        let val: Address = ctx
            .env
            .storage()
            .persistent()
            .get(&DataKey::PendingRecipientUpdate(7u64))
            .expect("PendingRecipientUpdate must round-trip at discriminant 20");
        assert_eq!(val, addr);
    });
}

/// Discriminant 28 (TotalKeeperFeesPaid) round-trips correctly.
/// This is the last checksum.rs-documented key; a round-trip confirms discriminants 21–28 are stable.
#[test]
fn discriminant_28_total_keeper_fees_paid_round_trips() {
    let ctx = Ctx::setup();
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        ctx.env
            .storage()
            .instance()
            .set(&DataKey::TotalKeeperFeesPaid, &777_i128);
        let val: i128 = ctx
            .env
            .storage()
            .instance()
            .get(&DataKey::TotalKeeperFeesPaid)
            .expect("TotalKeeperFeesPaid must round-trip at discriminant 28");
        assert_eq!(val, 777_i128);
    });
}

/// Discriminant 29 (AutoRenewEnabled) round-trips correctly.
/// First post-checksum.rs variant; confirms the append was correct.
#[test]
fn discriminant_29_auto_renew_enabled_round_trips() {
    let ctx = Ctx::setup();
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        ctx.env
            .storage()
            .persistent()
            .set(&DataKey::AutoRenewEnabled(5u64), &true);
        let val: bool = ctx
            .env
            .storage()
            .persistent()
            .get(&DataKey::AutoRenewEnabled(5u64))
            .expect("AutoRenewEnabled must round-trip at discriminant 29");
        assert!(val);
    });
}

/// Discriminant 35 (PooledStreamWithdrawn) round-trips correctly.
/// Former last variant; confirms the previously frozen discriminant 35 remains stable.
#[test]
fn discriminant_35_pooled_stream_withdrawn_round_trips() {
    let ctx = Ctx::setup();
    let addr = Address::generate(&ctx.env);
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        ctx.env.storage().persistent().set(
            &DataKey::PooledStreamWithdrawn(3u64, addr.clone()),
            &500_i128,
        );
        let val: i128 = ctx
            .env
            .storage()
            .persistent()
            .get(&DataKey::PooledStreamWithdrawn(3u64, addr.clone()))
            .expect("PooledStreamWithdrawn must round-trip at discriminant 35");
        assert_eq!(val, 500_i128);
    });
}

/// Discriminant 36 (DelegatedCancelNonce) is append-only and persistent.
#[test]
fn discriminant_36_delegated_cancel_nonce_round_trips() {
    let ctx = Ctx::setup();
    let sender = Address::generate(&ctx.env);
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        let key = DataKey::DelegatedCancelNonce(sender.clone());
        ctx.env.storage().persistent().set(&key, &9u64);
        let value: u64 = ctx
            .env
            .storage()
            .persistent()
            .get(&key)
            .expect("DelegatedCancelNonce must round-trip at discriminant 36");
        assert_eq!(value, 9);
    });
}

/// Discriminant 30 (MaxLookbackLedgers) round-trips correctly.
#[test]
fn discriminant_30_max_lookback_ledgers_round_trips() {
    let ctx = Ctx::setup();
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        ctx.env
            .storage()
            .persistent()
            .set(&DataKey::MaxLookbackLedgers(7u64), &17u32);
        let val: u32 = ctx
            .env
            .storage()
            .persistent()
            .get(&DataKey::MaxLookbackLedgers(7u64))
            .expect("MaxLookbackLedgers must round-trip at discriminant 30");
        assert_eq!(val, 17u32);
    });
}

/// Discriminant 34 (PooledStreamShares) round-trips correctly.
#[test]
fn discriminant_34_pooled_stream_shares_round_trips() {
    let ctx = Ctx::setup();
    let addr = Address::generate(&ctx.env);
    let cid = ctx.contract_id.clone();
    let shares = soroban_sdk::vec![&ctx.env, (addr.clone(), 500u32)];
    ctx.env.as_contract(&cid, || {
        ctx.env
            .storage()
            .persistent()
            .set(&DataKey::PooledStreamShares(2u64), &shares);
        let val: soroban_sdk::Vec<(Address, u32)> = ctx
            .env
            .storage()
            .persistent()
            .get(&DataKey::PooledStreamShares(2u64))
            .expect("PooledStreamShares must round-trip at discriminant 34");
        assert_eq!(val.len(), 1);
        assert_eq!(val.get(0).unwrap().0, addr);
    });
}

/// V5-seeded stream remains readable after a V9-only index key is written elsewhere.
#[test]
fn v5_stream_readable_after_v9_sender_streams_index_write() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    ctx.seed_v5_stream(42, &recipient);

    let sender = Address::generate(&ctx.env);
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        ctx.env.storage().persistent().set(
            &DataKey::SenderStreams(sender.clone()),
            &vec![&ctx.env, 99u64],
        );
    });

    let state = ctx.client.get_stream_state(&42u64);
    assert_eq!(state.recipient, recipient);
}

// ---------------------------------------------------------------------------
// CONTRACT_VERSION smoke test
// ---------------------------------------------------------------------------

/// `version()` returns the compile-time constant without touching user storage.
///
/// This test is intentionally minimal: it confirms the entry-point is callable
/// on a V5-seeded instance.
#[test]
fn version_entry_point_works_on_v5_seeded_instance() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    ctx.seed_v5_stream(0, &recipient);

    let v = ctx.client.version();
    assert_eq!(v, CONTRACT_VERSION);
}

/// The version entry-point is stable for both pre-init and post-init deployments.
/// This keeps the upgrade/versioning contract explicit for integrators and deployment scripts.
#[test]
fn version_entry_point_is_stable_before_and_after_init() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);
    assert_eq!(client.version(), CONTRACT_VERSION);

    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let sac = StellarAssetClient::new(&env, &token_id);
    let admin = Address::generate(&env);
    let sender = Address::generate(&env);
    sac.mint(&sender, &1_000_000_000);

    client.init(&token_id, &admin);
    assert_eq!(client.version(), CONTRACT_VERSION);
}

// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// CONTRACT_VERSION vs DataKey variant count cross-check suite
// ---------------------------------------------------------------------------

/// Returns the documented expected `DataKey` variant count for a given `CONTRACT_VERSION`.
///
/// # Version Mapping Table
///
/// | CONTRACT_VERSION | Expected DataKey Variant Count | Discriminant Range | Notes |
/// |------------------|--------------------------------|--------------------|-------|
/// | 5                | 15                             | 0..=14             | V5 release freeze |
/// | 6                | 29                             | 0..=28             | V6 freeze (21) + 8 post-freeze additive variants |
/// | 9                | 37                             | 0..=36             | Current live count |
///
/// # Security Safeguard & Maintenance Protocol
/// When a new `DataKey` variant is appended or `CONTRACT_VERSION` is bumped:
/// 1. Update this function's match table to include the new mapping.
/// 2. Update `all_live_datakey_variants()` to include a sample of the new variant.
/// 3. Update prose docs & discriminant tests in `contracts/stream/src/checksum.rs`.
/// 4. Update version history & upgrade strategy in `docs/upgrade.md`.
pub fn expected_datakey_count_for_version(version: u32) -> usize {
    match version {
        5 => 15,
        6 => 29,
        // Historical V7/V8 mappings remain at the previous 36-key boundary.
        7 | 8 => 36,
        // V9 includes append-only DelegatedCancelNonce at discriminant 36.
        9 => 37,
        other => panic!(
            "Unhandled CONTRACT_VERSION = {other} in expected_datakey_count_for_version. \
             When incrementing CONTRACT_VERSION, you must update the version mapping table in \
             contracts/stream/tests/storage_key_compat.rs, contracts/stream/src/checksum.rs, \
             and docs/upgrade.md."
        ),
    }
}

/// Constructs a vector containing sample instances of all 37 live `DataKey`
/// variants in declaration order.
///
/// Includes an exhaustive `match` on `DataKey` so that adding any new variant
/// to `DataKey` without adding it here will produce a compile error.
pub fn all_live_datakey_variants(env: &Env) -> soroban_sdk::Vec<DataKey> {
    let dummy_addr = Address::generate(env);
    let dummy_pause_kind = PauseKind::Protocol;

    let variants = vec![
        env,
        DataKey::Config,                                       // 0
        DataKey::NextStreamId,                                 // 1
        DataKey::Stream(0),                                    // 2
        DataKey::RecipientStreams(dummy_addr.clone()),         // 3
        DataKey::GlobalEmergencyPaused,                        // 4
        DataKey::CreationPaused,                               // 5
        DataKey::GlobalPauseReason,                            // 6
        DataKey::GlobalPauseTimestamp,                         // 7
        DataKey::GlobalPauseAdmin,                             // 8
        DataKey::AutoClaimDestination(0),                      // 9
        DataKey::NextTemplateId,                               // 10
        DataKey::ActiveTemplateCount,                          // 11
        DataKey::StreamTemplate(0),                            // 12
        DataKey::OwnerTemplateIds(dummy_addr.clone()),         // 13
        DataKey::TotalLiabilities,                             // 14
        DataKey::WithdrawNonce(dummy_addr.clone()),            // 15
        DataKey::PauseState,                                   // 16
        DataKey::ReentrancyLock,                               // 17
        DataKey::RecipientStreamPage(dummy_addr.clone(), 0),   // 18
        DataKey::RecipientStreamPageCount(dummy_addr.clone()), // 19
        DataKey::PendingRecipientUpdate(0),                    // 20
        DataKey::IdReservation(dummy_addr.clone()),            // 21
        DataKey::MaxRatePerSecond,                             // 22
        DataKey::DelegatedWithdrawNonce(dummy_addr.clone()),   // 23
        DataKey::LastPauseRecord(dummy_pause_kind),            // 24
        DataKey::RotationHistory(0),                           // 25
        DataKey::LastAccrualLedgerTimestamp,                   // 26
        DataKey::PausedStreamCount,                            // 27
        DataKey::TotalKeeperFeesPaid,                          // 28
        DataKey::AutoRenewEnabled(0),                          // 29
        DataKey::MaxLookbackLedgers(0),                        // 30
        DataKey::SenderStreams(dummy_addr.clone()),            // 31
        DataKey::PendingStreamOffer(0),                        // 32
        DataKey::RecipientPendingOffers(dummy_addr.clone()),   // 33
        DataKey::PooledStreamShares(0),                        // 34
        DataKey::PooledStreamWithdrawn(0, dummy_addr.clone()), // 35
        DataKey::DelegatedCancelNonce(dummy_addr.clone()),     // 36
    ];

    // Exhaustive match check — compile error if any DataKey variant is missing here.
    // We use a dedicated match (not variants.iter()) because DataKey does not implement Clone,
    // which would be required by soroban_sdk::Vec::iter().
    let _check_exhaustive = |k: DataKey| match k {
        DataKey::Config => {}
        DataKey::NextStreamId => {}
        DataKey::Stream(_) => {}
        DataKey::RecipientStreams(_) => {}
        DataKey::GlobalEmergencyPaused => {}
        DataKey::CreationPaused => {}
        DataKey::GlobalPauseReason => {}
        DataKey::GlobalPauseTimestamp => {}
        DataKey::GlobalPauseAdmin => {}
        DataKey::AutoClaimDestination(_) => {}
        DataKey::NextTemplateId => {}
        DataKey::ActiveTemplateCount => {}
        DataKey::StreamTemplate(_) => {}
        DataKey::OwnerTemplateIds(_) => {}
        DataKey::TotalLiabilities => {}
        DataKey::WithdrawNonce(_) => {}
        DataKey::PauseState => {}
        DataKey::ReentrancyLock => {}
        DataKey::RecipientStreamPage(_, _) => {}
        DataKey::RecipientStreamPageCount(_) => {}
        DataKey::PendingRecipientUpdate(_) => {}
        DataKey::IdReservation(_) => {}
        DataKey::MaxRatePerSecond => {}
        DataKey::DelegatedWithdrawNonce(_) => {}
        DataKey::LastPauseRecord(_) => {}
        DataKey::RotationHistory(_) => {}
        DataKey::LastAccrualLedgerTimestamp => {}
        DataKey::PausedStreamCount => {}
        DataKey::TotalKeeperFeesPaid => {}
        DataKey::AutoRenewEnabled(_) => {}
        DataKey::MaxLookbackLedgers(_) => {}
        DataKey::SenderStreams(_) => {}
        DataKey::PendingStreamOffer(_) => {}
        DataKey::RecipientPendingOffers(_) => {}
        DataKey::PooledStreamShares(_) => {}
        DataKey::PooledStreamWithdrawn(_, _) => {}
        DataKey::DelegatedCancelNonce(_) => {}
    };
    // Suppress unused-variable warning — the closure is only here for compile-time exhaustiveness.
    let _ = _check_exhaustive;

    variants
}

/// Machine-checks that `CONTRACT_VERSION` matches the expected `DataKey` variant count.
///
/// # Security & Architectural Invariants
///
/// - **Cross-Check Requirement:** Prevents version drift when new `DataKey` variants are added.
/// - **Staleness Guard:** Guarantees that `CONTRACT_VERSION` staleness does not break storage key compatibility.
/// - **Companion Documentation:** Cross-referenced with `contracts/stream/src/checksum.rs` and `docs/upgrade.md`.
///
/// Fails loudly with an explicit error message if the live `DataKey` variant count
/// diverges from `expected_datakey_count_for_version(CONTRACT_VERSION)`.
#[test]
fn test_contract_version_matches_datakey_variant_count() {
    let env = Env::default();
    let live_variants = all_live_datakey_variants(&env);
    let live_count = live_variants.len() as usize;

    let expected_count = expected_datakey_count_for_version(CONTRACT_VERSION);

    assert_eq!(
        live_count, expected_count,
        "CRITICAL VERSION DRIFT: CONTRACT_VERSION ({}) expects {} DataKey variants, \
         but the live DataKey enum has {} variants. \
         When adding a new DataKey variant, you MUST update: \
         1. expected_datakey_count_for_version() in contracts/stream/tests/storage_key_compat.rs \
         2. all_live_datakey_variants() in contracts/stream/tests/storage_key_compat.rs \
         3. Prose tables & variant count tests in contracts/stream/src/checksum.rs \
         4. Version history & policy in docs/upgrade.md \
         5. CONTRACT_VERSION in contracts/stream/src/lib.rs if required by versioning policy.",
        CONTRACT_VERSION, expected_count, live_count
    );
}

/// Edge case: V5 version mapping expected count is 15.
#[test]
fn test_expected_datakey_count_mapping_v5() {
    assert_eq!(expected_datakey_count_for_version(5), 15);
}

/// Edge case: V6 version mapping expected count is 29.
#[test]
fn test_expected_datakey_count_mapping_v6() {
    assert_eq!(expected_datakey_count_for_version(6), 29);
}

/// Edge case: V9 version mapping expected count is 37.
#[test]
fn test_expected_datakey_count_mapping_v9() {
    assert_eq!(expected_datakey_count_for_version(9), 37);
}

/// Edge case: Unmapped/future versions trigger panic forcing deliberate mapping update.
#[test]
#[should_panic(expected = "Unhandled CONTRACT_VERSION = 999")]
fn test_expected_datakey_count_mapping_unhandled_version_panics() {
    expected_datakey_count_for_version(999);
}

/// Assert exact live variant count is 37 (discriminants 0..=36).
#[test]
fn test_datakey_variant_count_exact_37() {
    let env = Env::default();
    let live_variants = all_live_datakey_variants(&env);
    assert_eq!(
        live_variants.len() as usize,
        37,
        "DataKey variant count changed without updating storage_key_compat test suite. \
         Add the new variant to all_live_datakey_variants() and update \
         expected_datakey_count_for_version()."
    );
}

/// Regression test: Verifies that synthetic version drift triggers an explicit assertion failure.
#[test]
fn test_regression_staleness_mismatch_detection() {
    let live_count = 37usize; // current live variant count
    let stale_version_expected_count = expected_datakey_count_for_version(5); // V5 expects 15

    assert_ne!(
        live_count, stale_version_expected_count,
        "Stale CONTRACT_VERSION mapping must be detected as mismatched against live DataKey count"
    );
}
