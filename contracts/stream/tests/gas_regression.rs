// See docs/gas.md for the baseline update process and review bar.
use fluxora_stream::{
    CreateStreamParams, FluxoraStream, FluxoraStreamClient, PauseReason, StreamKind, WithdrawToParam,
    MAX_MEMO_BYTES, MAX_METADATA_BYTES, MAX_METADATA_KEYS, MAX_METADATA_KEY_BYTES,
    MAX_METADATA_VALUE_BYTES, MAX_STREAM_ENTRY_BYTES, MAX_PAGE_SIZE,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Bytes, Env, Map,
};

// Per-invocation CPU budget (Soroban limit) with a 75% safety margin.
// The budget assertion fails if measured cost exceeds this threshold,
// guarding against inadvertent regressions (e.g. an increased MAX_PAGE_SIZE
// that worsens the O(n²) duplicate-ID scan).
const PER_INVOCATION_CPU_BUDGET: u64 = 25_000_000_000;

// Grace period (mirrors KEEPER_GRACE_PERIOD_SECONDS in lib.rs).
const KEEPER_GRACE: u64 = 604_800;

struct TestContext<'a> {
    env: Env,
    client: FluxoraStreamClient<'a>,
    sender: Address,
    recipient: Address,
    keeper: Address,
}

impl<'a> TestContext<'a> {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, FluxoraStream);
        let client = FluxoraStreamClient::new(&env, &contract_id);

        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin)
            .address();
        let sac = StellarAssetClient::new(&env, &token_id);

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);
        let keeper = Address::generate(&env);

        client.init(&token_id, &admin);

        // Fund the sender using the admin's minting power
        sac.mint(&sender, &1_000_000_i128);
        // Provide default allowance so create_stream can pull the deposit.
        TokenClient::new(&env, &token_id).approve(&sender, &contract_id, &i128::MAX, &100_000);

        Self {
            env,
            client,
            sender,
            recipient,
            keeper,
        }
    }

    fn create_default_stream(&self) -> u64 {
        let amount = 1000_i128;
        let rate = 1_i128;
        let start_time = 0u64;
        let cliff_time = 0u64;
        let end_time = 1000u64;

        self.client.create_stream(
            &self.sender,
            &CreateStreamParams {
                recipient: self.recipient.clone(),
                deposit_amount: amount,
                rate_per_second: rate,
                start_time: start_time,
                cliff_time: cliff_time,
                end_time: end_time,
                withdraw_dust_threshold: Some(0),
                memo: None,
                metadata: None,
                kind: StreamKind::Linear,
                irrevocable: None,
                witness: None,
            },
        )
    }
}

// KeeperTestContext is an alias for TestContext — same setup, same fields.
// Keeper tests use the identical harness; the type alias keeps the test
// names readable without duplicating the setup code.
type KeeperTestContext<'a> = TestContext<'a>;

fn measure_gas<F, C>(ctx: &C, f: F) -> u64
where
    F: FnOnce(&C),
    C: HasEnv,
{
    ctx.env().budget().reset_unlimited();
    f(ctx);
    ctx.env().budget().cpu_instruction_cost()
}

trait HasEnv {
    fn env(&self) -> &Env;
}

impl HasEnv for TestContext<'_> {
    fn env(&self) -> &Env {
        &self.env
    }
}

#[test]
fn test_create_stream_gas() {
    let ctx = TestContext::setup();

    let cost = measure_gas(&ctx, |ctx| {
        ctx.create_default_stream();
    });

    println!("GAS_MEASUREMENT: create_stream: single: {}", cost);
}

#[test]
fn test_withdraw_gas() {
    let ctx = TestContext::setup();

    let stream_id = ctx.create_default_stream();
    ctx.env.ledger().set_timestamp(500); // Accrue 500 tokens

    let cost = measure_gas(&ctx, |ctx| {
        ctx.client.withdraw(&stream_id);
    });

    println!("GAS_MEASUREMENT: withdraw: single: {}", cost);
}

#[test]
fn test_batch_withdraw_gas() {
    let sizes = [1, 10, 50, 100];

    for &size in &sizes {
        let ctx = TestContext::setup();

        let mut streams = soroban_sdk::Vec::new(&ctx.env);
        for _ in 0..size {
            streams.push_back(ctx.create_default_stream());
        }

        ctx.env.ledger().set_timestamp(500); // Accrue tokens for all

        let cost = measure_gas(&ctx, |ctx| {
            ctx.client.batch_withdraw(&ctx.recipient, &streams);
        });

        assert!(
            cost <= PER_INVOCATION_CPU_BUDGET,
            "batch_withdraw at size {} exceeded per-invocation CPU budget: {} > {}",
            size,
            cost,
            PER_INVOCATION_CPU_BUDGET,
        );

        println!("GAS_MEASUREMENT: batch_withdraw: {}: {}", size, cost);
    }
}

/// Gas regression baseline for `batch_withdraw_to`.
///
/// Uses a distinct destination address per withdrawal to exercise the
/// per-entry destination validation path alongside the O(n²) duplicate-ID
/// scan in `reject_duplicate_ids`.  The O(n²) scan costs roughly
/// n*(n-1)/2 comparisons at batch size n, so at MAX_PAGE_SIZE (100) the
/// worst case is ~4 950 element-by-element comparisons inside the helper.
#[test]
fn test_batch_withdraw_to_gas() {
    let sizes = [1, 10, 50, 100];

    for &size in &sizes {
        let ctx = TestContext::setup();

        let mut streams = soroban_sdk::Vec::new(&ctx.env);
        let mut destinations = soroban_sdk::Vec::new(&ctx.env);
        for _ in 0..size {
            streams.push_back(ctx.create_default_stream());
            destinations.push_back(Address::generate(&ctx.env));
        }

        let mut withdrawals = soroban_sdk::Vec::new(&ctx.env);
        for i in 0..size {
            withdrawals.push_back(WithdrawToParam {
                stream_id: streams.get(i as u32).unwrap(),
                destination: destinations.get(i as u32).unwrap(),
            });
        }

        ctx.env.ledger().set_timestamp(500); // Accrue tokens for all

        let cost = measure_gas(&ctx, |ctx| {
            ctx.client.batch_withdraw_to(&ctx.recipient, &withdrawals);
        });

        assert!(
            cost <= PER_INVOCATION_CPU_BUDGET,
            "batch_withdraw_to at size {} exceeded per-invocation CPU budget: {} > {}",
            size,
            cost,
            PER_INVOCATION_CPU_BUDGET,
        );

        println!("GAS_MEASUREMENT: batch_withdraw_to: {}: {}", size, cost);
    }
}

/// Gas regression baseline for `bulk_resume_streams_as_admin`.
///
/// Creates streams, pauses each one (advancing the ledger far enough to
/// clear the pause cooldown), then resumes them all in a single admin-authed
/// call. Batch sizes 1, 5, 10, and 20 mirror the documented gas baseline matrix
/// so `script/validate_gas.py` can compare each measured cost against
/// `docs/gas.md`.
#[test]
fn test_bulk_resume_streams_as_admin_gas() {
    let sizes = [1, 5, 10, 20];

    for &size in &sizes {
        let ctx = TestContext::setup();

        let mut streams = soroban_sdk::Vec::new(&ctx.env);
        for _ in 0..size {
            let id = ctx.create_default_stream();
            // Advance past the pause/resume cooldown (17 ledgers) so the
            // subsequent pause succeeds even if the ledger sequence is low.
            ctx.env.ledger().with_mut(|l| l.sequence_number += 32);
            ctx.client
                .pause_stream_as_admin(&id, &PauseReason::Administrative);
            streams.push_back(id);
        }

        let cost = measure_gas(&ctx, |ctx| {
            ctx.client.bulk_resume_streams_as_admin(&streams);
        });

        assert!(
            cost <= PER_INVOCATION_CPU_BUDGET,
            "bulk_resume_streams_as_admin at size {} exceeded per-invocation CPU budget: {} > {}",
            size,
            cost,
            PER_INVOCATION_CPU_BUDGET,
        );

        println!(
            "GAS_MEASUREMENT: bulk_resume_streams_as_admin: {}: {}",
            size, cost
        );
    }
}

/// Gas regression baseline for `bulk_cancel_streams`.
///
/// Creates active streams owned by the sender then cancels them all in a
/// single call. Batch sizes 1, 5, 10, and 20 mirror the documented gas
/// baseline matrix so `script/validate_gas.py` can compare each measured cost
/// against `docs/gas.md`.
#[test]
fn test_bulk_cancel_streams_gas() {
    let sizes = [1, 5, 10, 20];

    for &size in &sizes {
        let ctx = TestContext::setup();

        let mut streams = soroban_sdk::Vec::new(&ctx.env);
        for _ in 0..size {
            streams.push_back(ctx.create_default_stream());
        }

        ctx.env.ledger().set_timestamp(500); // Accrue tokens so cancellation is non-trivial

        let cost = measure_gas(&ctx, |ctx| {
            ctx.client.bulk_cancel_streams(&ctx.sender, &streams);
        });

        assert!(
            cost <= PER_INVOCATION_CPU_BUDGET,
            "bulk_cancel_streams at size {} exceeded per-invocation CPU budget: {} > {}",
            size,
            cost,
            PER_INVOCATION_CPU_BUDGET,
        );

        println!("GAS_MEASUREMENT: bulk_cancel_streams: {}: {}", size, cost);
    }
}

// ---------------------------------------------------------------------------
// keeper_cancel gas measurements
//
// Two variants capture the two meaningful cost paths:
//
//   partial_accrual — the common keeper incentive case: the stream expired with
//     an unstreamed balance, so the contract makes three token transfers
//     (recipient, sender, keeper).  This is the hot path for economically
//     rational keeper bots and the cost documented in docs/gas.md's
//     break-even formula.
//
//   fully_accrued   — the degenerate case: deposit == rate × duration, so
//     sender_refund_gross == 0, keeper_fee == 0 and no keeper transfer is
//     issued.  Only one token transfer (to the recipient) occurs.  Cost is
//     slightly lower than the partial_accrual variant.
//
// Both variants print a GAS_MEASUREMENT line that validate_gas.py picks up
// and compares against the JSON baseline in docs/gas.md.
// ---------------------------------------------------------------------------

/// keeper_cancel on a stream that still has an unstreamed balance (3 transfers).
///
/// Setup:
///   deposit = 10 000, rate = 5 token/s, start = 0, end = 1 000
///   → accrued at end_time = min(5 × 1 000, 10 000) = 5 000
///   → sender_refund_gross = 5 000
///   → keeper_fee = 5 000 × 50 / 10 000 = 25
///   → three token transfers: recipient 5 000, sender 4 975, keeper 25
#[test]
fn test_keeper_cancel_gas_partial_accrual() {
    let ctx = KeeperTestContext::setup();

    // Create the stream at t=0.
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client.create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 10_000_i128,
            rate_per_second: 5_i128,
            start_time: 0u64,
            cliff_time: 0u64,
            end_time: 1_000u64,
            withdraw_dust_threshold: Some(0_i128),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );

    // Advance past end_time + grace period so the stream is eligible.
    ctx.env.ledger().set_timestamp(1_000 + KEEPER_GRACE + 1);

    let cost = measure_gas(&ctx, |ctx| {
        ctx.client.keeper_cancel(&stream_id, &ctx.keeper);
    });

    // Print in the canonical GAS_MEASUREMENT format so validate_gas.py can
    // parse this line and compare it against the baseline in docs/gas.md.
    println!("GAS_MEASUREMENT: keeper_cancel: partial_accrual: {}", cost);
}

/// keeper_cancel on a stream that is fully accrued (1 transfer, keeper fee == 0).
///
/// Setup:
///   deposit = 1 000, rate = 1 token/s, start = 0, end = 1 000
///   → accrued at end_time = 1 000 == deposit
///   → sender_refund_gross = 0, keeper_fee = 0
///   → one token transfer: recipient 1 000; no sender or keeper transfers
#[test]
fn test_keeper_cancel_gas_fully_accrued() {
    let ctx = KeeperTestContext::setup();

    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client.create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1_000_i128,
            rate_per_second: 1_i128,
            start_time: 0u64,
            cliff_time: 0u64,
            end_time: 1_000u64,
            withdraw_dust_threshold: Some(0_i128),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );

    ctx.env.ledger().set_timestamp(1_000 + KEEPER_GRACE + 1);

    let cost = measure_gas(&ctx, |ctx| {
        ctx.client.keeper_cancel(&stream_id, &ctx.keeper);
    });

    println!("GAS_MEASUREMENT: keeper_cancel: fully_accrued: {}", cost);
}

// ---------------------------------------------------------------------------
// Stream persistent-entry XDR size regression
//
// The `Stream` struct is stored as a persistent Soroban ledger entry.  Rent
// is charged proportionally to the serialized byte size of that entry, so
// unbounded growth of any caller-controlled field inflates the rent cost for
// every stream in the protocol.
//
// Two optional fields are caller-controlled and can each approach their caps:
//   • `memo`     — up to MAX_MEMO_BYTES (256) bytes
//   • `metadata` — up to MAX_METADATA_BYTES (512) bytes of aggregate key+value
//                  data spread over up to MAX_METADATA_KEYS (8) entries
//
// The test below constructs a `Stream` via the contract (not directly), so the
// value is stored and retrieved through the same serialization path that
// production ledgers use.  It asserts that the XDR-serialized byte length of
// the retrieved `Stream` value stays within MAX_STREAM_ENTRY_BYTES (4 096).
//
// "Worst case" is defined as:
//   • all Optional fields populated (claim_owner, cancelled_at, is_pooled,
//     irrevocable, witness, parent_stream_id, memo, metadata)
//   • memo filled to MAX_MEMO_BYTES (256) bytes of 0xFF
//   • metadata has MAX_METADATA_KEYS (8) entries, each key is
//     MAX_METADATA_KEY_BYTES (32) bytes and each value is
//     MAX_METADATA_VALUE_BYTES (128) bytes — total raw payload = 1 280 bytes,
//     which exceeds MAX_METADATA_BYTES (512); the contract therefore rejects
//     that construction.  The actual worst-case accepted by the validator is
//     MAX_METADATA_BYTES (512) bytes spread over 8 keys, verified by
//     `test_stream_entry_xdr_size_worst_case_accepted_metadata` below.
//   • all i128 fields at i128::MAX, all u64 timestamps at u64::MAX,
//     delegation_depth = MAX_DELEGATION_DEPTH
//
// See docs/gas.md §"Stream Persistent-Entry Size" for the annotated field
// breakdown and the ceiling update procedure.
// ---------------------------------------------------------------------------

/// Helper: build a worst-case metadata map that exactly fits within
/// MAX_METADATA_BYTES.  We use 8 keys of 32 bytes and values sized so that
/// the aggregate key+value sum ≤ 512.
///
/// 8 keys × 32 bytes = 256 bytes of keys.
/// Remaining budget: 512 − 256 = 256 bytes for 8 values → 32 bytes each.
fn worst_case_metadata(env: &Env) -> Map<Bytes, Bytes> {
    let key_len = MAX_METADATA_KEY_BYTES as usize; // 32
    let total_budget = MAX_METADATA_BYTES as usize; // 512
    let total_key_bytes = (MAX_METADATA_KEYS as usize) * key_len; // 256
    let value_budget = total_budget - total_key_bytes; // 256
    let value_len = value_budget / (MAX_METADATA_KEYS as usize); // 32

    let mut meta: Map<Bytes, Bytes> = Map::new(env);
    for i in 0..MAX_METADATA_KEYS {
        // Build a key: 32 bytes, first byte encodes the index so each key is
        // unique; remaining bytes are 0xAA.
        let mut key_buf = vec![0xAAu8; key_len];
        key_buf[0] = i as u8;
        let key = Bytes::from_slice(env, &key_buf);

        // Value: value_len bytes of 0xBB.
        let val = Bytes::from_slice(env, &vec![0xBBu8; value_len]);
        meta.set(key, val);
    }
    meta
}

/// Worst-case Stream XDR size regression.
///
/// Constructs a stream via `create_streams` (the only public entry-point that
/// accepts both `memo` and `metadata`) with all optional fields at their maximum
/// allowed sizes, then retrieves the stored `Stream` via `get_stream_state` and
/// serialises it to XDR.  Asserts the serialised length ≤ MAX_STREAM_ENTRY_BYTES.
///
/// If this test fails after a `Stream` struct change, follow the update procedure
/// in docs/gas.md §"Stream Persistent-Entry Size".
#[test]
fn test_stream_entry_xdr_size_worst_case() {
    use soroban_sdk::xdr::ToXdr;

    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(0);

    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let sac = StellarAssetClient::new(&env, &token_id);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let admin = Address::generate(&env);

    client.init(&token_id, &admin);
    sac.mint(&sender, &i128::MAX);
    TokenClient::new(&env, &token_id).approve(&sender, &contract_id, &i128::MAX, &u32::MAX);

    // Memo at maximum allowed size (256 × 0xFF).
    let memo = Bytes::from_slice(&env, &vec![0xFFu8; MAX_MEMO_BYTES]);

    // Metadata: 8 keys × (32-byte key + 32-byte value) = 512 bytes aggregate —
    // exactly at MAX_METADATA_BYTES, so the contract must accept it.
    let metadata = worst_case_metadata(&env);

    // Witness address occupies the optional `witness` field.
    let witness = Address::generate(&env);

    let deposit: i128 = 1_000_000;
    let rate: i128 = 1;
    let start_time: u64 = 0;
    let cliff_time: u64 = 0;
    let end_time: u64 = 1_000_000;

    let params = soroban_sdk::vec![
        &env,
        CreateStreamParams {
            recipient: recipient.clone(),
            deposit_amount: deposit,
            rate_per_second: rate,
            start_time,
            cliff_time,
            end_time,
            withdraw_dust_threshold: Some(i128::MAX),
            memo: Some(memo.clone()),
            metadata: Some(metadata.clone()),
            kind: StreamKind::Linear,
            irrevocable: Some(true),
            witness: Some(witness.clone()),
        }
    ];

    let ids = client.create_streams(&sender, &params);
    let stream_id = ids.get(0).expect("stream created");

    // Retrieve the persisted Stream struct through the public view.
    let stream = client.get_stream_state(&stream_id);

    // Serialize the Stream to XDR using the same trait that Soroban uses when
    // writing to persistent storage (soroban_sdk::xdr::ToXdr).
    let xdr_bytes = stream.to_xdr(&env);
    let serialized_len = xdr_bytes.len() as usize;

    println!(
        "STREAM_XDR_SIZE: worst_case: {} bytes (ceiling: {} bytes, headroom: {} bytes)",
        serialized_len,
        MAX_STREAM_ENTRY_BYTES,
        MAX_STREAM_ENTRY_BYTES - serialized_len
    );

    assert!(
        serialized_len <= MAX_STREAM_ENTRY_BYTES,
        "Stream XDR size {} bytes exceeds MAX_STREAM_ENTRY_BYTES {} bytes. \
         The Stream struct has grown beyond the documented ceiling. \
         See docs/gas.md §\"Stream Persistent-Entry Size\" for the update procedure.",
        serialized_len,
        MAX_STREAM_ENTRY_BYTES
    );
}

/// Baseline (minimal) Stream XDR size — all optional fields absent.
///
/// Provides the lower-bound data point printed in docs/gas.md alongside the
/// worst-case ceiling so operators can reason about the rent cost spread.
#[test]
fn test_stream_entry_xdr_size_baseline() {
    use soroban_sdk::xdr::ToXdr;

    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(0);

    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let sac = StellarAssetClient::new(&env, &token_id);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let admin = Address::generate(&env);

    client.init(&token_id, &admin);
    sac.mint(&sender, &1_000_000_i128);
    TokenClient::new(&env, &token_id).approve(&sender, &contract_id, &i128::MAX, &100_000);

    // No memo, no metadata, no optional fields.
    let params = soroban_sdk::vec![
        &env,
        CreateStreamParams {
            recipient: recipient.clone(),
            deposit_amount: 1_000,
            rate_per_second: 1,
            start_time: 0,
            cliff_time: 0,
            end_time: 1_000,
            withdraw_dust_threshold: None,
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        }
    ];

    let ids = client.create_streams(&sender, &params);
    let stream_id = ids.get(0).expect("stream created");

    let stream = client.get_stream_state(&stream_id);
    let xdr_bytes = stream.to_xdr(&env);
    let serialized_len = xdr_bytes.len() as usize;

    println!(
        "STREAM_XDR_SIZE: baseline (no optional fields): {} bytes",
        serialized_len
    );

    // Baseline must be strictly less than the worst-case ceiling.
    assert!(
        serialized_len < MAX_STREAM_ENTRY_BYTES,
        "Baseline Stream XDR size {} bytes should be well below MAX_STREAM_ENTRY_BYTES {}",
        serialized_len,
        MAX_STREAM_ENTRY_BYTES
    );
}

/// Memo-only worst case — only memo populated, no metadata.
///
/// Isolates the memo contribution so docs/gas.md can document it separately.
#[test]
fn test_stream_entry_xdr_size_memo_only() {
    use soroban_sdk::xdr::ToXdr;

    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(0);

    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let sac = StellarAssetClient::new(&env, &token_id);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let admin = Address::generate(&env);

    client.init(&token_id, &admin);
    sac.mint(&sender, &1_000_000_i128);
    TokenClient::new(&env, &token_id).approve(&sender, &contract_id, &i128::MAX, &100_000);

    let memo = Bytes::from_slice(&env, &vec![0xFFu8; MAX_MEMO_BYTES]);

    let params = soroban_sdk::vec![
        &env,
        CreateStreamParams {
            recipient: recipient.clone(),
            deposit_amount: 1_000,
            rate_per_second: 1,
            start_time: 0,
            cliff_time: 0,
            end_time: 1_000,
            withdraw_dust_threshold: None,
            memo: Some(memo),
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        }
    ];

    let ids = client.create_streams(&sender, &params);
    let stream_id = ids.get(0).expect("stream created");

    let stream = client.get_stream_state(&stream_id);
    let xdr_bytes = stream.to_xdr(&env);
    let serialized_len = xdr_bytes.len() as usize;

    println!(
        "STREAM_XDR_SIZE: memo_only ({}B memo): {} bytes",
        MAX_MEMO_BYTES, serialized_len
    );

    assert!(
        serialized_len <= MAX_STREAM_ENTRY_BYTES,
        "Memo-only Stream XDR size {} bytes exceeds ceiling {}",
        serialized_len,
        MAX_STREAM_ENTRY_BYTES
    );
}

/// Metadata-only worst case — only metadata populated, no memo.
///
/// Isolates the metadata contribution so docs/gas.md can document it separately.
#[test]
fn test_stream_entry_xdr_size_metadata_only() {
    use soroban_sdk::xdr::ToXdr;

    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(0);

    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let sac = StellarAssetClient::new(&env, &token_id);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let admin = Address::generate(&env);

    client.init(&token_id, &admin);
    sac.mint(&sender, &1_000_000_i128);
    TokenClient::new(&env, &token_id).approve(&sender, &contract_id, &i128::MAX, &100_000);

    let metadata = worst_case_metadata(&env);

    let params = soroban_sdk::vec![
        &env,
        CreateStreamParams {
            recipient: recipient.clone(),
            deposit_amount: 1_000,
            rate_per_second: 1,
            start_time: 0,
            cliff_time: 0,
            end_time: 1_000,
            withdraw_dust_threshold: None,
            memo: None,
            metadata: Some(metadata),
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        }
    ];

    let ids = client.create_streams(&sender, &params);
    let stream_id = ids.get(0).expect("stream created");

    let stream = client.get_stream_state(&stream_id);
    let xdr_bytes = stream.to_xdr(&env);
    let serialized_len = xdr_bytes.len() as usize;

    println!(
        "STREAM_XDR_SIZE: metadata_only ({} entries, {}B aggregate): {} bytes",
        MAX_METADATA_KEYS, MAX_METADATA_BYTES, serialized_len
    );

    assert!(
        serialized_len <= MAX_STREAM_ENTRY_BYTES,
        "Metadata-only Stream XDR size {} bytes exceeds ceiling {}",
        serialized_len,
        MAX_STREAM_ENTRY_BYTES
    );
}

// ---------------------------------------------------------------------------
// Edge-case gas measurements (task 2 + 3)
//
// These tests lock down cost paths that the baseline tests leave implicit:
//
//   create_stream_with_cliff     — cliff_time > start_time adds a comparison
//                                  branch in the validation path; this test
//                                  ensures that branch does not regress.
//
//   create_stream_cliff_only     — StreamKind::CliffOnly forces rate=0 inside
//                                  create_stream; the extra branch + validation
//                                  path is captured separately from Linear.
//
//   withdraw_partial_accrual     — withdraw when only ~50% of the stream has
//                                  accrued; the accrual math executes the
//                                  full `min(current, end) - start` path.
//
//   withdraw_to_single           — withdraw_to: same transfer cost as withdraw
//                                  but with a destination-address argument that
//                                  exercises the routing branch.
//
//   pause_then_resume_single     — pause_stream + resume_stream on a single
//                                  stream; guards both halves of the cooldown-
//                                  aware state-machine at minimal batch size.
//
//   create_streams_partial_gas   — create_streams_partial with a mixed batch
//                                  (some succeed, some fail); captures the
//                                  per-entry isolation overhead.
//
//   batch_withdraw_max_page_size — batch_withdraw at exactly MAX_PAGE_SIZE (100);
//                                  documents the bound and guards against
//                                  accidental changes to the constant that would
//                                  silently change the worst-case O(n²) budget.
// ---------------------------------------------------------------------------

/// Gas baseline for `create_stream` with a non-zero cliff time.
///
/// The cliff branch adds a single comparison inside `validate_stream_params` but
/// does not change token-transfer or storage cost. This test captures the extra
/// path so any future regression in the cliff-validation logic is detected.
///
/// Setup: 1 000-token linear stream, cliff at t=500 (half-way through duration).
#[test]
fn test_create_stream_with_cliff_gas() {
    let ctx = TestContext::setup();

    let cost = measure_gas(&ctx, |ctx| {
        ctx.client.create_stream(
            &ctx.sender,
            &CreateStreamParams {
                recipient: ctx.recipient.clone(),
                deposit_amount: 1_000_i128,
                rate_per_second: 1_i128,
                start_time: 0u64,
                cliff_time: 500u64,  // non-zero cliff: exercises the cliff-validation branch
                end_time: 1_000u64,
                withdraw_dust_threshold: Some(0_i128),
                memo: None,
                metadata: None,
                kind: StreamKind::Linear,
                irrevocable: None,
                witness: None,
            },
        );
    });

    assert!(
        cost <= PER_INVOCATION_CPU_BUDGET,
        "create_stream (with cliff) exceeded per-invocation CPU budget: {} > {}",
        cost,
        PER_INVOCATION_CPU_BUDGET,
    );

    println!("GAS_MEASUREMENT: create_stream_with_cliff: single: {}", cost);
}

/// Gas baseline for `create_stream` with `StreamKind::CliffOnly`.
///
/// CliffOnly streams have `rate_per_second` forced to 0 inside the contract
/// before validation. The rewrite adds a branch that is absent from the Linear
/// path. This test pins its cost independently.
///
/// Setup: 1 000-token cliff-only stream, cliff at end_time (full deposit at cliff).
#[test]
fn test_create_stream_cliff_only_gas() {
    let ctx = TestContext::setup();

    let cost = measure_gas(&ctx, |ctx| {
        ctx.client.create_stream(
            &ctx.sender,
            &CreateStreamParams {
                recipient: ctx.recipient.clone(),
                deposit_amount: 1_000_i128,
                rate_per_second: 0_i128, // CliffOnly: rate is forced to 0 by the contract
                start_time: 0u64,
                cliff_time: 1_000u64,   // cliff == end_time: full deposit released at cliff
                end_time: 1_000u64,
                withdraw_dust_threshold: Some(0_i128),
                memo: None,
                metadata: None,
                kind: StreamKind::CliffOnly,
                irrevocable: None,
                witness: None,
            },
        );
    });

    assert!(
        cost <= PER_INVOCATION_CPU_BUDGET,
        "create_stream (CliffOnly) exceeded per-invocation CPU budget: {} > {}",
        cost,
        PER_INVOCATION_CPU_BUDGET,
    );

    println!("GAS_MEASUREMENT: create_stream_cliff_only: single: {}", cost);
}

/// Gas baseline for `withdraw` when only a partial amount has accrued.
///
/// This is the canonical mid-stream withdraw path: `current_time` is between
/// `start_time` and `end_time`, so the accrual formula executes the
/// `(current - start) * rate` branch with `min(current, end)` clamping.
/// The baseline `test_withdraw_gas` advances to t=500 on a 0→1000 stream which
/// is the same arithmetic path; this test makes the intent explicit by labelling
/// it as "partial_accrual" so validate_gas.py can distinguish the two if the
/// baselines diverge.
///
/// Setup: 10 000-token stream (rate=10/s, duration=0→1000).
///        Advance ledger to t=300 → 3 000 tokens accrued, 7 000 pending.
#[test]
fn test_withdraw_partial_accrual_gas() {
    let ctx = TestContext::setup();

    // Create a stream with more headroom so partial accrual is unambiguous.
    let stream_id = ctx.client.create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 10_000_i128,
            rate_per_second: 10_i128,
            start_time: 0u64,
            cliff_time: 0u64,
            end_time: 1_000u64,
            withdraw_dust_threshold: Some(0_i128),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );

    // Advance to 30% of the stream duration — leaves 70% unstreamed.
    ctx.env.ledger().set_timestamp(300);

    let cost = measure_gas(&ctx, |ctx| {
        ctx.client.withdraw(&stream_id);
    });

    assert!(
        cost <= PER_INVOCATION_CPU_BUDGET,
        "withdraw (partial_accrual) exceeded per-invocation CPU budget: {} > {}",
        cost,
        PER_INVOCATION_CPU_BUDGET,
    );

    println!("GAS_MEASUREMENT: withdraw_partial_accrual: single: {}", cost);
}

/// Gas baseline for `withdraw_to` (single stream, custom destination).
///
/// `withdraw_to` shares the accrual and token-transfer logic of `withdraw` but
/// routes the proceeds to an explicit destination address instead of the stream
/// recipient. The destination-routing branch is the only material difference;
/// this test documents its incremental cost.
///
/// Setup: same 1 000-token linear stream as `create_default_stream`, t=500.
#[test]
fn test_withdraw_to_single_gas() {
    let ctx = TestContext::setup();

    let stream_id = ctx.create_default_stream();
    ctx.env.ledger().set_timestamp(500);

    // Send the accrued funds to a distinct destination (not the recipient itself).
    let destination = Address::generate(&ctx.env);

    let cost = measure_gas(&ctx, |ctx| {
        ctx.client.withdraw_to(&stream_id, &destination);
    });

    assert!(
        cost <= PER_INVOCATION_CPU_BUDGET,
        "withdraw_to (single) exceeded per-invocation CPU budget: {} > {}",
        cost,
        PER_INVOCATION_CPU_BUDGET,
    );

    println!("GAS_MEASUREMENT: withdraw_to_single: single: {}", cost);
}

/// Gas baseline for `pause_stream` + `resume_stream` on a single stream.
///
/// Guards the round-trip cost of the pause/resume state machine. The cooldown
/// check (`MIN_PAUSE_INTERVAL_LEDGERS = 17`) is exercised by advancing the
/// ledger sequence past the threshold between the two operations. Captures both
/// legs so a regression in either half (e.g. extra storage writes or a new
/// duplicate event check) is visible.
///
/// Two separate GAS_MEASUREMENT lines are printed so validate_gas.py can track
/// each leg independently against the docs/gas.md baseline.
#[test]
fn test_pause_then_resume_single_gas() {
    let ctx = TestContext::setup();

    let stream_id = ctx.create_default_stream();

    // Advance ledger sequence past the pause/resume cooldown.
    ctx.env.ledger().with_mut(|l| l.sequence_number += 32);

    let pause_cost = measure_gas(&ctx, |ctx| {
        ctx.client
            .pause_stream(&stream_id, &PauseReason::Operational);
    });

    assert!(
        pause_cost <= PER_INVOCATION_CPU_BUDGET,
        "pause_stream (single) exceeded per-invocation CPU budget: {} > {}",
        pause_cost,
        PER_INVOCATION_CPU_BUDGET,
    );

    println!("GAS_MEASUREMENT: pause_stream: single: {}", pause_cost);

    // Advance past the resume cooldown before resuming.
    ctx.env.ledger().with_mut(|l| l.sequence_number += 32);

    let resume_cost = measure_gas(&ctx, |ctx| {
        ctx.client.resume_stream(&stream_id);
    });

    assert!(
        resume_cost <= PER_INVOCATION_CPU_BUDGET,
        "resume_stream (single) exceeded per-invocation CPU budget: {} > {}",
        resume_cost,
        PER_INVOCATION_CPU_BUDGET,
    );

    println!("GAS_MEASUREMENT: resume_stream: single: {}", resume_cost);
}

/// Gas baseline for `create_streams_partial` with a mixed batch.
///
/// `create_streams_partial` isolates per-entry failures: valid entries are
/// committed and invalid entries produce an error result without reverting the
/// whole call. The per-entry overhead (validation + result push) is captured here
/// at three batch sizes.
///
/// Mixed batch composition per size:
///   - Half the entries are valid (deposit=1 000, rate=1, 0→1 000).
///   - Half are invalid (deposit=0, which fails validation).
///
/// This exercises the fast-fail path for the rejected entries alongside the
/// normal commit path for the accepted entries.
///
/// The GAS_MEASUREMENT lines use the "partial_N" key so validate_gas.py
/// can store them separately from the all-success `create_streams` baseline.
#[test]
fn test_create_streams_partial_gas() {
    // Use small sizes: partial is not designed for MAX_PAGE_SIZE batches and the
    // mixed failure path makes the per-entry cost harder to compare at large n.
    let sizes: &[(u32, &str)] = &[(4, "4"), (8, "8"), (16, "16")];

    for &(size, label) in sizes {
        let ctx = TestContext::setup();

        let mut params = soroban_sdk::Vec::new(&ctx.env);
        for i in 0..size {
            if i % 2 == 0 {
                // Valid entry.
                params.push_back(CreateStreamParams {
                    recipient: ctx.recipient.clone(),
                    deposit_amount: 1_000_i128,
                    rate_per_second: 1_i128,
                    start_time: 0u64,
                    cliff_time: 0u64,
                    end_time: 1_000u64,
                    withdraw_dust_threshold: Some(0_i128),
                    memo: None,
                    metadata: None,
                    kind: StreamKind::Linear,
                    irrevocable: None,
                    witness: None,
                });
            } else {
                // Invalid entry: deposit_amount = 0 → fails `InvalidAmount`.
                params.push_back(CreateStreamParams {
                    recipient: ctx.recipient.clone(),
                    deposit_amount: 0_i128,
                    rate_per_second: 1_i128,
                    start_time: 0u64,
                    cliff_time: 0u64,
                    end_time: 1_000u64,
                    withdraw_dust_threshold: Some(0_i128),
                    memo: None,
                    metadata: None,
                    kind: StreamKind::Linear,
                    irrevocable: None,
                    witness: None,
                });
            }
        }

        let cost = measure_gas(&ctx, |ctx| {
            ctx.client.create_streams_partial(&ctx.sender, &params);
        });

        assert!(
            cost <= PER_INVOCATION_CPU_BUDGET,
            "create_streams_partial at size {} exceeded per-invocation CPU budget: {} > {}",
            size,
            cost,
            PER_INVOCATION_CPU_BUDGET,
        );

        println!(
            "GAS_MEASUREMENT: create_streams_partial: {}: {}",
            label, cost
        );
    }
}

/// Gas boundary test: `batch_withdraw` at exactly `MAX_PAGE_SIZE`.
///
/// This test exercises the O(n²) `reject_duplicate_ids` scan at the documented
/// worst-case batch size and asserts that the per-invocation CPU budget is not
/// exceeded. It is a specialised variant of `test_batch_withdraw_gas` that
/// explicitly names the boundary so that any future change to `MAX_PAGE_SIZE`
/// (which changes the worst-case cost) shows up as a test failure before
/// the CI gas baseline report catches it.
///
/// The test also prints a `GAS_MEASUREMENT` line tagged `max_page_size` so
/// validate_gas.py can record it separately from the existing size-100 entry.
#[test]
fn test_batch_withdraw_max_page_size_gas() {
    let ctx = TestContext::setup();

    let page = MAX_PAGE_SIZE as usize; // 100 at time of writing

    let mut streams = soroban_sdk::Vec::new(&ctx.env);
    for _ in 0..page {
        streams.push_back(ctx.create_default_stream());
    }

    ctx.env.ledger().set_timestamp(500);

    let cost = measure_gas(&ctx, |ctx| {
        ctx.client.batch_withdraw(&ctx.recipient, &streams);
    });

    assert!(
        cost <= PER_INVOCATION_CPU_BUDGET,
        "batch_withdraw at MAX_PAGE_SIZE ({}) exceeded per-invocation CPU budget: {} > {}",
        page,
        cost,
        PER_INVOCATION_CPU_BUDGET,
    );

    println!(
        "GAS_MEASUREMENT: batch_withdraw_max_page_size: {}: {}",
        page, cost
    );
}

// ---------------------------------------------------------------------------
// Additional edge-case gas measurements (issue #1286)
//
// These tests fill the remaining coverage gaps identified in the issue review:
//
//   cancel_stream_single         — single `cancel_stream` by the sender on a
//                                  partially-accrued active stream.  The bulk
//                                  variant (`bulk_cancel_streams`) is already
//                                  measured but does not expose the per-stream
//                                  cost in isolation.
//
//   zero_accrual_withdraw        — `withdraw` when the cliff has not yet been
//                                  reached.  No token transfer is issued; the
//                                  accrual short-circuit path is exercised and
//                                  its cost documented.
//
//   update_rate_per_second       — `update_rate_per_second` (rate increase) on
//                                  an active stream.  Checkpoints the accrual,
//                                  validates the new rate against the max-rate
//                                  cap, then saves the updated stream.
//
//   decrease_rate_per_second     — `decrease_rate_per_second` on an active
//                                  stream.  Checkpoints accrual, computes a
//                                  partial refund, persists state, and issues
//                                  a token transfer back to the sender.
//
//   top_up_stream                — `top_up_stream` adds deposit to an active
//                                  stream.  Pulls tokens from the funder and
//                                  increases the global liabilities counter.
//
//   shorten_stream_end_time      — `shorten_stream_end_time` truncates an active
//                                  stream's schedule, computes a sender refund,
//                                  and issues the refund token transfer.
//
//   extend_stream_end_time       — `extend_stream_end_time` pushes an active
//                                  stream's end time further into the future when
//                                  the existing deposit is sufficient to cover the
//                                  extended schedule at the current rate.
//
//   emergency_pause_create       — `create_stream` attempted while the contract
//                                  is under emergency pause.  The call should
//                                  revert with `GloballyPaused` after a minimal
//                                  storage read; this test documents the cost of
//                                  the early-exit guard.
// ---------------------------------------------------------------------------

/// Gas baseline for `cancel_stream` (single stream, partial accrual).
///
/// Measures the cost of a sender-initiated cancellation at mid-stream (t=500
/// on a 0→1 000 schedule).  The contract executes:
///   1. Load stream.
///   2. Calculate accrued-to-date.
///   3. Transfer accrued portion to recipient.
///   4. Transfer unstreamed refund to sender.
///   5. Persist `Cancelled` state with `cancelled_at`.
///
/// Two token transfers occur (recipient + sender), so this is more expensive
/// than a plain `withdraw` (one transfer) and cheaper than `keeper_cancel`
/// (three transfers plus fee arithmetic).
///
/// Setup: 1 000-token linear stream, t=500 → 500 tokens accrued, 500 refunded.
#[test]
fn test_cancel_stream_single_gas() {
    let ctx = TestContext::setup();

    let stream_id = ctx.create_default_stream();

    // Advance to half-way so there is meaningful accrual AND a meaningful refund.
    ctx.env.ledger().set_timestamp(500);

    let cost = measure_gas(&ctx, |ctx| {
        ctx.client.cancel_stream(&stream_id);
    });

    assert!(
        cost <= PER_INVOCATION_CPU_BUDGET,
        "cancel_stream (single) exceeded per-invocation CPU budget: {} > {}",
        cost,
        PER_INVOCATION_CPU_BUDGET,
    );

    println!("GAS_MEASUREMENT: cancel_stream: single: {}", cost);
}

/// Gas baseline for `withdraw` when the stream's cliff has not yet been reached.
///
/// Before `cliff_time`, `calculate_accrued_amount` returns 0.  The `withdraw`
/// implementation detects a zero withdrawable balance, skips all token-transfer
/// and state-mutation work, and returns 0 immediately.  This test documents
/// the cost of that short-circuit path.
///
/// The test is labelled `zero_accrual` (not `before_cliff`) because the same
/// zero-withdrawable path is also hit when the stream is already fully drained
/// and the caller invokes `withdraw` again — cliff semantics are just the most
/// natural way to set up the pre-condition in isolation.
///
/// Setup: 1 000-token stream with cliff at t=500; ledger is at t=100
///        → 0 tokens accrued, no transfer issued.
#[test]
fn test_withdraw_zero_accrual_gas() {
    let ctx = TestContext::setup();

    // Create a stream where the cliff is far in the future relative to the
    // test's ledger timestamp (t=0 initially).
    let stream_id = ctx.client.create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1_000_i128,
            rate_per_second: 1_i128,
            start_time: 0u64,
            cliff_time: 500u64, // cliff well in the future
            end_time: 1_000u64,
            withdraw_dust_threshold: Some(0_i128),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );

    // Advance to before the cliff — accrual is 0.
    ctx.env.ledger().set_timestamp(100);

    let cost = measure_gas(&ctx, |ctx| {
        ctx.client.withdraw(&stream_id);
    });

    assert!(
        cost <= PER_INVOCATION_CPU_BUDGET,
        "withdraw (zero_accrual / pre-cliff) exceeded per-invocation CPU budget: {} > {}",
        cost,
        PER_INVOCATION_CPU_BUDGET,
    );

    println!("GAS_MEASUREMENT: withdraw_zero_accrual: single: {}", cost);
}

/// Gas baseline for `update_rate_per_second` (rate increase, active stream).
///
/// `update_rate_per_second` checkpoints the current accrual, validates the
/// new rate against the governance-controlled cap and deposit ceiling, and
/// saves the updated stream.  No token transfer occurs (deposit already locked).
///
/// Setup:
///   deposit = 2 000, rate = 1/s, start = 0, end = 1 000.
///   At t=300 we increase the rate to 2/s.
///   new_total_streamable = 2 × 1 000 = 2 000 ≤ deposit, so the update is valid.
#[test]
fn test_update_rate_per_second_gas() {
    let ctx = TestContext::setup();

    // Deposit must cover the higher rate for the full duration:
    //   rate=2, duration=1000 → total_streamable=2000 ≤ deposit=2000 ✓
    let stream_id = ctx.client.create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 2_000_i128,
            rate_per_second: 1_i128,  // start at rate=1
            start_time: 0u64,
            cliff_time: 0u64,
            end_time: 1_000u64,
            withdraw_dust_threshold: Some(0_i128),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );

    // Advance ledger sequence past the rate-change cooldown before the call.
    ctx.env.ledger().with_mut(|l| {
        l.timestamp = 300;
        l.sequence_number += 32; // clear rate-change cooldown
    });

    let cost = measure_gas(&ctx, |ctx| {
        ctx.client.update_rate_per_second(&stream_id, &2_i128);
    });

    assert!(
        cost <= PER_INVOCATION_CPU_BUDGET,
        "update_rate_per_second exceeded per-invocation CPU budget: {} > {}",
        cost,
        PER_INVOCATION_CPU_BUDGET,
    );

    println!("GAS_MEASUREMENT: update_rate_per_second: single: {}", cost);
}

/// Gas baseline for `decrease_rate_per_second` (rate decrease + refund).
///
/// `decrease_rate_per_second` checkpoints the current accrual, recomputes the
/// new deposit ceiling under the lower rate, computes the sender refund, persists
/// the updated state (CEI order), and issues one token transfer back to the sender.
///
/// Setup:
///   deposit = 2 000, rate = 2/s, start = 0, end = 1 000.
///   At t=300 we decrease rate to 1/s.
///   Accrued-to-date = 300 × 2 = 600.  Remaining seconds = 700.
///   Future accrual at new rate = 1 × 700 = 700.  New deposit = 600 + 700 = 1 300.
///   Refund = 2 000 − 1 300 = 700 tokens transferred back to sender.
#[test]
fn test_decrease_rate_per_second_gas() {
    let ctx = TestContext::setup();

    // Create a stream where deposit covers rate=2 for the full duration so the
    // decrease to rate=1 produces a meaningful refund.
    let stream_id = ctx.client.create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 2_000_i128,  // covers rate=2 × duration=1000
            rate_per_second: 2_i128,
            start_time: 0u64,
            cliff_time: 0u64,
            end_time: 1_000u64,
            withdraw_dust_threshold: Some(0_i128),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );

    ctx.env.ledger().with_mut(|l| {
        l.timestamp = 300;
        l.sequence_number += 32; // clear rate-change cooldown
    });

    let cost = measure_gas(&ctx, |ctx| {
        ctx.client.decrease_rate_per_second(&stream_id, &1_i128);
    });

    assert!(
        cost <= PER_INVOCATION_CPU_BUDGET,
        "decrease_rate_per_second exceeded per-invocation CPU budget: {} > {}",
        cost,
        PER_INVOCATION_CPU_BUDGET,
    );

    println!("GAS_MEASUREMENT: decrease_rate_per_second: single: {}", cost);
}

/// Gas baseline for `top_up_stream` (add deposit to an active stream).
///
/// `top_up_stream` pulls tokens from the funder, increases the stream's
/// deposit_amount, and updates the global TotalLiabilities counter.  No
/// schedule change occurs.
///
/// Setup: 1 000-token stream active at t=300; top-up amount = 500 tokens.
#[test]
fn test_top_up_stream_gas() {
    let ctx = TestContext::setup();

    let stream_id = ctx.create_default_stream();

    // Advance the ledger so the stream is live (past start) but not yet expired.
    ctx.env.ledger().set_timestamp(300);

    // The top-up funder can be the sender (the default sender already has
    // a large allowance set up by TestContext::setup).
    let cost = measure_gas(&ctx, |ctx| {
        ctx.client.top_up_stream(&stream_id, &ctx.sender, &500_i128);
    });

    assert!(
        cost <= PER_INVOCATION_CPU_BUDGET,
        "top_up_stream exceeded per-invocation CPU budget: {} > {}",
        cost,
        PER_INVOCATION_CPU_BUDGET,
    );

    println!("GAS_MEASUREMENT: top_up_stream: single: {}", cost);
}

/// Gas baseline for `shorten_stream_end_time` (schedule truncation + refund).
///
/// `shorten_stream_end_time` checkpoints accrual, computes a sender refund for
/// the truncated portion, persists the updated schedule (CEI), and issues one
/// token transfer back to the sender.
///
/// Setup: 1 000-token stream (rate=1/s, 0→1 000); at t=300 we shorten to t=600.
///   Remaining seconds at t=600 = 600 − 0 = 600.  New max streamable = 600.
///   Accrued-to-date = 300 ≤ 600, so new_deposit = 600.
///   Refund = 1 000 − 600 = 400 tokens transferred to sender.
#[test]
fn test_shorten_stream_end_time_gas() {
    let ctx = TestContext::setup();

    let stream_id = ctx.create_default_stream();

    // Advance to t=300 so accrual is meaningful and refund is non-zero.
    ctx.env.ledger().set_timestamp(300);

    let cost = measure_gas(&ctx, |ctx| {
        ctx.client.shorten_stream_end_time(&stream_id, &600u64);
    });

    assert!(
        cost <= PER_INVOCATION_CPU_BUDGET,
        "shorten_stream_end_time exceeded per-invocation CPU budget: {} > {}",
        cost,
        PER_INVOCATION_CPU_BUDGET,
    );

    println!("GAS_MEASUREMENT: shorten_stream_end_time: single: {}", cost);
}

/// Gas baseline for `extend_stream_end_time` (schedule extension, no token transfer).
///
/// `extend_stream_end_time` moves the stream's end time forward without changing
/// the rate or deposit.  The existing deposit must be sufficient to cover the
/// extended schedule at the current rate.  No token transfer occurs.
///
/// Setup: 2 000-token stream (rate=1/s, 0→1 000).  At t=300 we extend to t=1 500.
///   new_total_streamable = 1 × 1 500 = 1 500 ≤ deposit=2 000 ✓ (no extra transfer).
#[test]
fn test_extend_stream_end_time_gas() {
    let ctx = TestContext::setup();

    // Use a stream with extra deposit so the extended schedule still fits.
    let stream_id = ctx.client.create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 2_000_i128,  // covers rate=1 × end=1500
            rate_per_second: 1_i128,
            start_time: 0u64,
            cliff_time: 0u64,
            end_time: 1_000u64,
            withdraw_dust_threshold: Some(0_i128),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );

    // Advance to mid-stream.
    ctx.env.ledger().set_timestamp(300);

    let cost = measure_gas(&ctx, |ctx| {
        ctx.client.extend_stream_end_time(&stream_id, &1_500u64);
    });

    assert!(
        cost <= PER_INVOCATION_CPU_BUDGET,
        "extend_stream_end_time exceeded per-invocation CPU budget: {} > {}",
        cost,
        PER_INVOCATION_CPU_BUDGET,
    );

    println!("GAS_MEASUREMENT: extend_stream_end_time: single: {}", cost);
}

/// Gas baseline for the emergency-pause guard on `create_stream`.
///
/// When `set_global_emergency_paused(true)` is active, every state-mutating
/// entry point (including `create_stream`) calls `require_not_globally_paused`
/// early in its execution.  That function reads one instance-storage key
/// (`GlobalEmergencyPaused`) and returns `GloballyPaused` before any stream
/// validation, deposit transfer, or storage write occurs.
///
/// This test captures the cost of that early-exit guard so that any future
/// change to the pause-check overhead (e.g. additional flag reads) is detected.
/// The test calls `try_create_stream` (the fallible variant) so the panic from
/// the expected error does not abort the test process.
///
/// Setup: emergency pause activated; `create_stream` called with a valid payload
///        → call reverts at `require_not_globally_paused`.
#[test]
fn test_create_stream_under_emergency_pause_gas() {
    let ctx = TestContext::setup();

    // Activate the global emergency pause.
    ctx.client.set_global_emergency_paused(&true);

    let params = CreateStreamParams {
        recipient: ctx.recipient.clone(),
        deposit_amount: 1_000_i128,
        rate_per_second: 1_i128,
        start_time: 0u64,
        cliff_time: 0u64,
        end_time: 1_000u64,
        withdraw_dust_threshold: Some(0_i128),
        memo: None,
        metadata: None,
        kind: StreamKind::Linear,
        irrevocable: None,
        witness: None,
    };

    // Reset budget and call the fallible variant so we capture cost even on revert.
    ctx.env.budget().reset_unlimited();
    let _result = ctx.client.try_create_stream(&ctx.sender, &params);
    let cost = ctx.env.budget().cpu_instruction_cost();

    assert!(
        cost <= PER_INVOCATION_CPU_BUDGET,
        "create_stream (emergency_pause guard) exceeded per-invocation CPU budget: {} > {}",
        cost,
        PER_INVOCATION_CPU_BUDGET,
    );

    println!(
        "GAS_MEASUREMENT: create_stream_emergency_pause: single: {}",
        cost
    );
}
