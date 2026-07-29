//! Adversarial test: a malicious SEP-41 token must NOT be able to re-enter
//! `delegated_withdraw` from inside its own payout `transfer` callback.
//!
//! This file has two tests, because building it surfaced a platform fact
//! that splits the single claim in the task brief ("the nested call is
//! blocked by the contract's reentrancy lock and returns
//! `ContractError::InvalidState`") into two separate, independently-true
//! statements on this codebase's pinned dependencies:
//!
//! 1. `malicious_token_reentrant_call_is_rejected_before_reaching_the_contract` --
//!    a real, independent malicious token contract that attempts a genuine
//!    cross-contract call back into `delegated_withdraw` from its `transfer`
//!    callback is rejected by the **Soroban host itself**, before
//!    `delegated_withdraw`'s code (and therefore its lock) ever runs.
//! 2. `delegated_withdraw_returns_invalid_state_when_reentrancy_lock_already_held` --
//!    the contract's own `acquire_reentrancy_lock` (src/storage.rs), exercised
//!    directly and in isolation, does independently return exactly
//!    `Err(ContractError::InvalidState)` when it is already held -- i.e. the
//!    exact state that exists for the duration of every in-flight
//!    `push_token` sequence inside `withdraw` / `withdraw_to` / `batch_withdraw`
//!    / `cancel_stream` / `delegated_withdraw` / `delegated_cancel`.
//!
//! # Why one test can't cover both (a necessary platform caveat)
//!
//! On the pinned dependency versions here (`soroban-sdk = "21.7.7"`,
//! `soroban-env-host = "=21.2.1"`), **every** guest-to-host contract call --
//! i.e. every call made through a generated `Client`, including
//! `FluxoraStreamClient` and the standard `token::Client` -- is hard-coded to
//! `ContractReentryMode::Prohibited` (see `Host::call` / `Host::try_call` in
//! `soroban-env-host-21.2.1/src/host.rs`, which literally comment "this is
//! the recommended path of calling a contract, with `reentry` always set
//! `ContractReentryMode::Prohibited`" / "TODO: A `reentry` flag will be
//! passed from `try_call` into here. For now, we are passing in
//! `ContractReentryMode::Prohibited`..."). `ContractReentryMode::Allowed` is
//! defined in that same host crate but is never actually used anywhere
//! (`#[allow(dead_code)]`); `SelfAllowed` is used only by the built-in
//! classic-account contract and only tolerates *immediate* self-calls
//! (distance 0), not the distance-1 "A calls token, token calls A" pattern a
//! reentrant payout callback needs.
//!
//! Concretely, this means a genuinely independent malicious token contract
//! cannot, on this platform version, issue a real cross-contract call back
//! into `delegated_withdraw` at all: the host aborts the attempt
//! (`InvokeError::Abort`, diagnostic message "Contract re-entry is not
//! allowed") *before* `delegated_withdraw`'s code -- and hence its
//! `acquire_reentrancy_lock` check -- ever runs. This was reproduced twice
//! while building this test:
//! - Calling back via `FluxoraStreamClient::try_delegated_withdraw` (the only
//!   channel a real malicious token contract actually has) returns a raw,
//!   undecodable `Err(Err(InvokeError::Abort))` -- never a typed
//!   `ContractError`.
//! - Attempting to route around that using `env.as_contract(&stream_id, ||
//!   FluxoraStream::delegated_withdraw(...))` (a testutils frame-push helper
//!   used safely elsewhere in this codebase, e.g.
//!   `tests/liability_invariant.rs`, but only ever from *top-level* test
//!   code) from *inside* the malicious token's own live `transfer` frame is
//!   worse: `delegated_withdraw` itself, at step 7, makes a real (non-`try_`)
//!   `token::Client::balance(...)` call back to its configured token to cap
//!   `gross_withdrawable` -- and since that token (the malicious contract) is
//!   already on the call stack, that call *also* hits
//!   `ContractReentryMode::Prohibited`, and because it is not a `try_` call it
//!   escalates straight to an unrecoverable host panic
//!   (`HostError: Error(Context, InternalError)`, "frame-depth mismatch")
//!   rather than a catchable error.
//!
//! Both failure modes independently confirm what this repository already
//! documents:
//! - `docs/token-assumptions.md`: "**Malicious token reentrancy**: Cannot be
//!   tested with standard Soroban test utilities. Requires manual review of
//!   CEI ordering and state persistence."
//! - `contracts/stream/CEI_ANALYSIS.md`: "**Residual risk:** Soroban's
//!   execution model does not support mid-transaction reentrancy in the same
//!   way EVM does, but CEI is maintained throughout as a defense-in-depth
//!   invariant..."
//!
//! So "a malicious token cannot re-enter `delegated_withdraw`" is actually
//! enforced *twice* on this platform, and the outer layer (the host's
//! blanket reentry prohibition, test 1) is strictly stronger than -- and
//! makes structurally unreachable -- the inner layer (this contract's own
//! lock, test 2). Test 2 exercises that inner layer directly (bypassing only
//! the now-proven-unreachable "live nested call" framing, not the contract's
//! real code) so the lock's own `InvalidState` behavior, specifically named
//! in the task, is still independently verified against the real,
//! unmodified `delegated_withdraw` code path.
//!
//! # Why test 1 requires TWO streams for the same recipient
//!
//! `delegated_withdraw` follows checks-effects-interactions: it persists the
//! *entire* accrued amount as `withdrawn_amount` (step 10) and increments the
//! recipient's nonce (step 11) *before* acquiring the reentrancy lock and
//! calling `push_token` (step 12). So by the time the malicious token's
//! `transfer` callback fires, on the *same* stream `gross_withdrawable` would
//! already be `0` (a misleading early `Ok(0)` return that proves nothing).
//! Nonces are explicitly scoped per-recipient, not per-stream (see the doc
//! comment on `delegated_withdraw`'s `nonce` parameter), so the reentrant
//! attempt must target a *second* stream owned by the same recipient with a
//! pre-signed nonce-`N+1` payload for the attempt to be well-formed at all
//! (even though, per the above, it is rejected before that payload is ever
//! validated).
//!
//! # Nonce trap
//!
//! The outer call uses `nonce = N`. Step 11 increments the stored nonce to
//! `N + 1` *before* the payout transfer (step 12) that triggers the
//! callback, so the reentrant attempt is pre-signed for `nonce = N + 1`.
//!
//! # Swallow trap
//!
//! `push_token` (src/storage.rs, `#[cfg(test)]` branch) calls
//! `token_client.try_transfer(...)` and swallows any error from the token by
//! returning `Ok(())`. So the *outer* `delegated_withdraw`'s return value
//! cannot tell us whether the reentrant attempt was blocked -- it succeeds
//! either way. The malicious token therefore records the attempt's outcome
//! into its own instance storage, and test 1 reads it back directly.

extern crate std;

use ed25519_dalek::{Signer, SigningKey};
use fluxora_stream::{
    ContractError, CreateStreamParams, FluxoraStream, FluxoraStreamClient, StreamKind,
};
use soroban_sdk::{
    contract, contractimpl, contracttype,
    testutils::{Address as _, Ledger},
    xdr::{AccountId, PublicKey, ScAddress, Uint256},
    Address, Bytes, BytesN, Env, TryIntoVal,
};

// ---------------------------------------------------------------------------
// XDR / signing helpers (mirrors tests/delegated_cancel.rs)
// ---------------------------------------------------------------------------

/// Construct a Soroban `Address` from a raw ed25519 public-key byte array.
fn address_from_pk(env: &Env, pk: &[u8; 32]) -> Address {
    ScAddress::Account(AccountId(PublicKey::PublicKeyTypeEd25519(Uint256(*pk))))
        .try_into_val(env)
        .expect("valid ed25519 key -> address")
}

/// Build the canonical `delegated_withdraw` signed payload (56 bytes, NO
/// domain tag), big-endian:
/// `stream_id(8) | nonce(8) | deadline(8) | expected_minimum_amount(16) | relayer_fee(16)`.
/// See `FluxoraStream::delegated_withdraw` step 5 in `src/lib.rs`.
fn build_withdraw_msg(
    env: &Env,
    stream_id: u64,
    nonce: u64,
    deadline: u64,
    expected_minimum_amount: i128,
    relayer_fee: i128,
) -> Bytes {
    let mut msg = Bytes::new(env);
    msg.extend_from_array(&stream_id.to_be_bytes());
    msg.extend_from_array(&nonce.to_be_bytes());
    msg.extend_from_array(&deadline.to_be_bytes());
    msg.extend_from_array(&expected_minimum_amount.to_be_bytes());
    msg.extend_from_array(&relayer_fee.to_be_bytes());
    msg
}

fn sign_msg(env: &Env, signing_key: &SigningKey, msg: &Bytes) -> BytesN<64> {
    let bytes: std::vec::Vec<u8> = (0..msg.len()).map(|i| msg.get_unchecked(i)).collect();
    BytesN::from_array(env, &signing_key.sign(&bytes).to_bytes())
}

// ---------------------------------------------------------------------------
// Malicious SEP-41 token (used by test 1)
// ---------------------------------------------------------------------------
//
// Implements just enough of SEP-41 (balance / transfer / transfer_from, plus
// inert approve / allowance stubs) to pass `verify_token_behavior` at `init`
// (zero-value self-transfer must be a strict no-op; balances must be
// non-negative) and to fund + settle real streams via an internal balance
// map. On the payout leg of `delegated_withdraw` -- a `transfer` call FROM
// the stream contract with `amount > 0` -- it attempts to re-enter
// `delegated_withdraw` exactly once, targeting a second stream owned by the
// same recipient with a pre-signed nonce-`N+1` payload, via the only channel
// a real malicious contract actually has: `FluxoraStreamClient`. See the
// module doc comment for why this is rejected by the Soroban host itself.

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
enum TKey {
    /// Internal per-address token balance.
    Balance(Address),
    /// The FluxoraStream contract address (the reentrancy target / guard).
    StreamAddr,
    /// Guarantees the reentrant attempt happens at most once.
    AlreadyReentered,
    /// Pre-signed `delegated_withdraw` args for the reentrant attempt.
    ReentryArgsKey,
    /// Whether the reentrant attempt was ever made.
    ReentryAttempted,
    /// Outcome of the reentrant attempt. See `MaliciousReentrantToken::transfer`
    /// for the sentinel encoding.
    ReentryCode,
}

/// Pre-signed `delegated_withdraw` arguments for the reentrant attempt,
/// configured by the test via `configure` once it knows the victim
/// stream_id and has computed the nonce-`N+1` signature.
#[contracttype]
#[derive(Clone, Debug)]
pub struct ReentryArgs {
    pub stream_id: u64,
    pub relayer: Address,
    pub recipient_pk: BytesN<32>,
    pub nonce: u64,
    pub deadline: u64,
    pub expected_minimum_amount: i128,
    pub relayer_fee: i128,
    pub signature: BytesN<64>,
}

#[contract]
pub struct MaliciousReentrantToken;

#[contractimpl]
impl MaliciousReentrantToken {
    /// Test-only faucet (not part of SEP-41) to seed the sender's balance
    /// before `create_stream` pulls the deposit via `transfer_from`.
    pub fn mint(env: Env, to: Address, amount: i128) {
        let key = TKey::Balance(to);
        let current: i128 = env.storage().instance().get(&key).unwrap_or(0);
        env.storage().instance().set(&key, &(current + amount));
    }

    /// Wire up the stream contract address and the pre-signed nonce-`N+1`
    /// `delegated_withdraw` args that will be attempted on the next payout
    /// leg (`transfer` FROM the stream contract).
    pub fn configure(env: Env, stream_contract: Address, args: ReentryArgs) {
        env.storage()
            .instance()
            .set(&TKey::StreamAddr, &stream_contract);
        env.storage().instance().set(&TKey::ReentryArgsKey, &args);
    }

    /// Whether the malicious `transfer` callback ever attempted the
    /// reentrant `delegated_withdraw` call.
    pub fn reentry_attempted(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&TKey::ReentryAttempted)
            .unwrap_or(false)
    }

    /// Outcome of the reentrant attempt:
    /// - `-100` if it unexpectedly *succeeded*.
    /// - `-200` if it returned a decoded `ContractError` (would mean
    ///   `delegated_withdraw`'s code actually ran -- see module doc comment
    ///   for why this is not expected on this platform).
    /// - `-300` if it was rejected as a raw, undecodable host error before
    ///   reaching the contract (the expected outcome: the host's blanket
    ///   `ContractReentryMode::Prohibited`).
    /// - `i64::MIN` if no attempt was ever recorded.
    pub fn reentry_result_code(env: Env) -> i64 {
        env.storage()
            .instance()
            .get(&TKey::ReentryCode)
            .unwrap_or(i64::MIN)
    }

    pub fn balance(env: Env, id: Address) -> i128 {
        env.storage()
            .instance()
            .get(&TKey::Balance(id))
            .unwrap_or(0)
    }

    /// SEP-41 `transfer`. Attempts to re-enter `delegated_withdraw` at most
    /// once, only on the payout leg: `amount > 0` AND `from` is the stream
    /// contract (the zero-value self-transfer that `verify_token_behavior`
    /// performs at `init` has `amount == 0` and is therefore never a
    /// trigger).
    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        if amount > 0 {
            let already: bool = env
                .storage()
                .instance()
                .get(&TKey::AlreadyReentered)
                .unwrap_or(false);
            let stream_addr: Option<Address> = env.storage().instance().get(&TKey::StreamAddr);

            if !already {
                if let Some(stream_addr) = stream_addr {
                    if from == stream_addr {
                        // Mark BEFORE issuing the attempt so it happens at
                        // most once even if it somehow recursed further.
                        env.storage().instance().set(&TKey::AlreadyReentered, &true);
                        env.storage().instance().set(&TKey::ReentryAttempted, &true);

                        let args: ReentryArgs = env
                            .storage()
                            .instance()
                            .get(&TKey::ReentryArgsKey)
                            .expect("configure() must be called before the payout leg fires");

                        // The only channel a real malicious contract has:
                        // a genuine cross-contract call via the generated
                        // Client. See the module doc comment for why the
                        // Soroban host rejects this outright on this SDK
                        // version, before delegated_withdraw's own code (and
                        // hence its reentrancy lock) ever runs.
                        let victim = FluxoraStreamClient::new(&env, &stream_addr);
                        let res = victim.try_delegated_withdraw(
                            &args.stream_id,
                            &args.relayer,
                            &args.recipient_pk,
                            &args.nonce,
                            &args.deadline,
                            &args.expected_minimum_amount,
                            &args.relayer_fee,
                            &args.signature,
                        );

                        // try_* clients return
                        // Result<Result<T, E>, Result<E, InvokeError>>.
                        let code: i64 = match res {
                            Ok(Ok(_amount)) => -100, // succeeded -- would be a real problem
                            Ok(Err(_e)) => -200,     // unexpected shape
                            Err(Ok(e)) => -200 - (e as i64), // decoded ContractError (unexpected on this platform)
                            Err(Err(_)) => -300, // raw host rejection (expected: Prohibited reentry)
                        };
                        env.storage().instance().set(&TKey::ReentryCode, &code);
                    }
                }
            }
        }

        Self::book(&env, &from, &to, amount);
    }

    pub fn transfer_from(env: Env, _spender: Address, from: Address, to: Address, amount: i128) {
        Self::book(&env, &from, &to, amount);
    }

    pub fn approve(
        _env: Env,
        _from: Address,
        _spender: Address,
        _amount: i128,
        _expiration_ledger: u32,
    ) {
    }

    pub fn allowance(_env: Env, _from: Address, _spender: Address) -> i128 {
        i128::MAX
    }

    fn book(env: &Env, from: &Address, to: &Address, amount: i128) {
        if amount == 0 {
            return;
        }
        let from_key = TKey::Balance(from.clone());
        let to_key = TKey::Balance(to.clone());
        let from_bal: i128 = env.storage().instance().get(&from_key).unwrap_or(0);
        let to_bal: i128 = env.storage().instance().get(&to_key).unwrap_or(0);
        env.storage().instance().set(&from_key, &(from_bal - amount));
        env.storage().instance().set(&to_key, &(to_bal + amount));
    }
}

// ---------------------------------------------------------------------------
// Test 1: a real malicious token's reentrant attempt is rejected
// ---------------------------------------------------------------------------

#[test]
fn malicious_token_reentrant_call_is_rejected_before_reaching_the_contract() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(0);

    let contract_id = env.register_contract(None, FluxoraStream);
    let token_id = env.register_contract(None, MaliciousReentrantToken);
    let stream_client = FluxoraStreamClient::new(&env, &contract_id);
    let token_client = MaliciousReentrantTokenClient::new(&env, &token_id);

    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let relayer = Address::generate(&env);

    // Recipient MUST be the ed25519-derived address: delegated_withdraw
    // checks the supplied public key against the stream's recipient via
    // `ed25519_pubkey_from_address`.
    let recipient_sk = SigningKey::from_bytes(&[0x99u8; 32]);
    let recipient_pk_arr = recipient_sk.verifying_key().to_bytes();
    let recipient_pk = BytesN::from_array(&env, &recipient_pk_arr);
    let recipient = address_from_pk(&env, &recipient_pk_arr);

    // Two streams, same recipient, same rate/deposit: stream_a is the
    // outer call's target; stream_b is the reentrant attempt's target (see
    // the module doc comment for why a single stream cannot work here).
    let deposit_per_stream: i128 = 2000;
    token_client.mint(&sender, &(deposit_per_stream * 2));

    stream_client
        .try_init(&token_id, &admin)
        .expect("init call must not trap")
        .expect("init must succeed: the malicious token must pass verify_token_behavior");

    let make_params = |recipient: Address| CreateStreamParams {
        recipient,
        deposit_amount: deposit_per_stream,
        rate_per_second: 1,
        start_time: 0,
        cliff_time: 0,
        end_time: 2000,
        withdraw_dust_threshold: Some(0),
        memo: None,
        metadata: None,
        kind: StreamKind::Linear,
        irrevocable: None,
        witness: None,
    };

    let stream_a = stream_client
        .try_create_stream(&sender, &make_params(recipient.clone()))
        .expect("create_stream(a) call must not trap")
        .expect("create_stream(a) must succeed");
    let stream_b = stream_client
        .try_create_stream(&sender, &make_params(recipient.clone()))
        .expect("create_stream(b) call must not trap")
        .expect("create_stream(b) must succeed");

    // Advance time so both streams have accrued (but not fully claimed)
    // balance. last_withdraw_ledger is 0 on both fresh streams, so the
    // withdraw-frequency check (MIN_WITHDRAW_INTERVAL_LEDGERS) is a no-op
    // here regardless of ledger sequence.
    env.ledger().set_timestamp(500);

    let nonce_n = stream_client.get_delegated_nonce(&recipient);
    assert_eq!(
        nonce_n, 0,
        "recipient's delegated-withdraw nonce must start at 0"
    );

    let deadline: u64 = 10_000;

    // Outer call: withdraw from stream_a using nonce N.
    let msg_a = build_withdraw_msg(&env, stream_a, nonce_n, deadline, 0, 0);
    let sig_a = sign_msg(&env, &recipient_sk, &msg_a);

    // Reentrant attempt: MUST target stream_b with nonce N + 1 (the nonce
    // trap -- see module doc comment) to be well-formed at all, even though
    // it is rejected before that would ever be checked.
    let nonce_n_plus_1 = nonce_n + 1;
    let msg_b = build_withdraw_msg(&env, stream_b, nonce_n_plus_1, deadline, 0, 0);
    let sig_b = sign_msg(&env, &recipient_sk, &msg_b);

    token_client.configure(
        &contract_id,
        &ReentryArgs {
            stream_id: stream_b,
            relayer: relayer.clone(),
            recipient_pk: recipient_pk.clone(),
            nonce: nonce_n_plus_1,
            deadline,
            expected_minimum_amount: 0,
            relayer_fee: 0,
            signature: sig_b,
        },
    );

    // Fire the outer call. relayer_fee = 0 so push_token performs exactly
    // one transfer (net payout to the recipient) -- that single transfer
    // callback is where the malicious token attempts its reentry.
    let outer_result = stream_client.try_delegated_withdraw(
        &stream_a,
        &relayer,
        &recipient_pk,
        &nonce_n,
        &deadline,
        &0i128,
        &0i128,
        &sig_a,
    );
    let net_amount = outer_result
        .expect("outer delegated_withdraw call must not trap")
        .expect(
            "outer delegated_withdraw must succeed: push_token's #[cfg(test)] \
             try_transfer swallows the blocked reentrant attempt's failure, \
             so the legitimate withdrawal still completes",
        );
    assert_eq!(
        net_amount, 500,
        "recipient must receive the full amount accrued on stream_a (500 = 500s * 1/s)"
    );
    assert_eq!(
        token_client.balance(&recipient),
        500,
        "recipient's token balance must reflect the successful outer payout"
    );

    // --- The proof -------------------------------------------------------
    assert!(
        token_client.reentry_attempted(),
        "malicious token's transfer() callback must have attempted the \
         reentrant delegated_withdraw(stream_b) call during the outer payout"
    );

    let code = token_client.reentry_result_code();
    assert_ne!(
        code, -100,
        "the reentrant call must NOT succeed"
    );
    assert_eq!(
        code, -300,
        "the reentrant call must be rejected by the Soroban host's blanket \
         ContractReentryMode::Prohibited BEFORE it ever reaches \
         delegated_withdraw's code -- got code {} (see module doc comment)",
        code
    );
}

// ---------------------------------------------------------------------------
// Test 2: the contract's own reentrancy lock independently returns
// InvalidState -- the exact property named in the task, proven directly
// against the real, unmodified delegated_withdraw code path.
// ---------------------------------------------------------------------------

#[test]
fn delegated_withdraw_returns_invalid_state_when_reentrancy_lock_already_held() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(0);

    let contract_id = env.register_contract(None, FluxoraStream);
    let stream_client = FluxoraStreamClient::new(&env, &contract_id);

    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let sac = soroban_sdk::token::StellarAssetClient::new(&env, &token_id);

    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let relayer = Address::generate(&env);

    let recipient_sk = SigningKey::from_bytes(&[0x55u8; 32]);
    let recipient_pk_arr = recipient_sk.verifying_key().to_bytes();
    let recipient_pk = BytesN::from_array(&env, &recipient_pk_arr);
    let recipient = address_from_pk(&env, &recipient_pk_arr);

    sac.mint(&sender, &2000i128);
    let token = soroban_sdk::token::Client::new(&env, &token_id);
    token.approve(&sender, &contract_id, &2000i128, &100_000);

    stream_client
        .try_init(&token_id, &admin)
        .expect("init call must not trap")
        .expect("init must succeed against a real Stellar Asset Contract");

    let stream_id = stream_client
        .try_create_stream(
            &sender,
            &CreateStreamParams {
                recipient: recipient.clone(),
                deposit_amount: 2000,
                rate_per_second: 1,
                start_time: 0,
                cliff_time: 0,
                end_time: 2000,
                withdraw_dust_threshold: Some(0),
                memo: None,
                metadata: None,
                kind: StreamKind::Linear,
                irrevocable: None,
                witness: None,
            },
        )
        .expect("create_stream call must not trap")
        .expect("create_stream must succeed");

    env.ledger().set_timestamp(500);

    let nonce = stream_client.get_delegated_nonce(&recipient);
    let deadline: u64 = 10_000;
    let msg = build_withdraw_msg(&env, stream_id, nonce, deadline, 0, 0);
    let sig = sign_msg(&env, &recipient_sk, &msg);

    // Simulate the exact state that exists for the entire duration of any
    // in-flight push_token sequence (see `acquire_reentrancy_lock` /
    // `release_reentrancy_lock`, src/storage.rs, called from
    // `delegated_withdraw` step 12 and from every other token-transferring
    // entrypoint). This is the same top-level `as_contract` technique
    // already used elsewhere in this codebase to exercise contract-internal
    // state directly (e.g. `tests/liability_invariant.rs`); called here from
    // the test function itself, with no live invocation frame beneath it, so
    // (unlike a genuine nested call -- see the module doc comment) it does
    // not touch the host's reentry machinery at all.
    env.as_contract(&contract_id, || {
        fluxora_stream::storage::acquire_reentrancy_lock(&env)
            .expect("lock must be free before this test acquires it");
    });

    // Everything about this call is valid (correct nonce, correct
    // signature, correct deadline, fresh stream with real accrued balance)
    // except that the lock is already held.
    let result = stream_client.try_delegated_withdraw(
        &stream_id,
        &relayer,
        &recipient_pk,
        &nonce,
        &deadline,
        &0i128,
        &0i128,
        &sig,
    );

    // --- Correctness proof: rule out the decoys named in the task --------
    assert_ne!(
        result,
        Err(Ok(ContractError::InvalidSignature)),
        "must not fail on signature/nonce validation -- the payload is valid"
    );
    assert_ne!(
        result,
        Err(Ok(ContractError::WithdrawalTooFrequent)),
        "must not fail on the withdraw-frequency check -- this is a fresh stream"
    );
    assert!(
        result.is_err(),
        "delegated_withdraw must fail while the reentrancy lock is held, got {:?}",
        result
    );

    // The actual assertion named in the task: `acquire_reentrancy_lock`
    // (src/storage.rs) returns exactly `Err(ContractError::InvalidState)`
    // (discriminant 2, confirmed in src/lib.rs's `ContractError` enum) when
    // the lock is already held.
    assert_eq!(
        result,
        Err(Ok(ContractError::InvalidState)),
        "a delegated_withdraw call made while the reentrancy lock is already \
         held must fail with exactly InvalidState"
    );

    env.as_contract(&contract_id, || {
        fluxora_stream::storage::release_reentrancy_lock(&env);
    });
}

// ---------------------------------------------------------------------------
// Test 3 — the FIX: cancel_stream_internal now guards its refund transfer.
//
// `delegated_cancel` routes through `cancel_stream_internal` (as do
// `cancel_stream`, `cancel_stream_as_admin`, and `witnessed_cancel_stream`).
// Before this change, that helper's unstreamed-refund `push_token(sender, ..)`
// was NOT wrapped in the reentrancy lock, unlike the withdraw paths. With the
// lock pre-held (the state that exists during an in-flight token transfer /
// nested re-entry), cancelling a stream that owes a refund must now fail with
// `InvalidState`, exactly like `delegated_withdraw` does. This test fails
// against the pre-fix contract, where cancel never touched the lock and the
// refund would have gone out.
// ---------------------------------------------------------------------------
#[test]
fn cancel_refund_transfer_acquires_reentrancy_lock() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1000;
        li.sequence_number = 10;
    });

    let contract_id = env.register_contract(None, FluxoraStream {});
    let stream_client = FluxoraStreamClient::new(&env, &contract_id);

    let token_admin = Address::generate(&env);
    let token_id = env.register_stellar_asset_contract(token_admin);
    stream_client.init(&token_id, &Address::generate(&env));

    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);

    // Large unstreamed balance so cancel yields refund > 0 (only then is the
    // guarded refund transfer reached).
    let deposit = 10_000i128;
    let sac = soroban_sdk::token::StellarAssetClient::new(&env, &token_id);
    sac.mint(&sender, &deposit);
    let token = soroban_sdk::token::Client::new(&env, &token_id);
    token.approve(&sender, &contract_id, &deposit, &100_000);

    let now = env.ledger().timestamp();
    let stream_id = stream_client.create_stream(
        &sender,
        &CreateStreamParams {
            recipient: recipient.clone(),
            deposit_amount: deposit,
            rate_per_second: 10,
            start_time: now,
            cliff_time: now,
            end_time: now + 1000,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );

    // Pre-acquire the real reentrancy lock (top-level as_contract), simulating
    // the state during an in-flight token transfer, mirroring the technique in
    // the lock-held delegated_withdraw test above.
    env.as_contract(&contract_id, || {
        fluxora_stream::storage::acquire_reentrancy_lock(&env)
            .expect("lock should be free at start");
    });

    // Cancel reaches cancel_stream_internal's refund transfer, which now tries
    // to acquire the already-held lock and reverts with InvalidState.
    let result = stream_client.try_cancel_stream(&stream_id);
    assert_eq!(
        result,
        Err(Ok(ContractError::InvalidState)),
        "cancel refund transfer must acquire the reentrancy lock; with the lock \
         held the cancel must fail with InvalidState (fails against the pre-fix \
         contract, where cancel never acquired the lock)"
    );

    env.as_contract(&contract_id, || {
        fluxora_stream::storage::release_reentrancy_lock(&env);
    });
}
