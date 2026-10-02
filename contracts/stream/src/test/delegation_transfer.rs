//! Issue #1845 — Delegation surviving a recipient transfer.
//!
//! # The question
//!
//! When a stream's recipient changes via `transfer_recipient`, what happens to
//! existing `Delegate(stream_id, delegate)` entries?  Two behaviours are
//! conceivable:
//!
//! 1. **Grants follow the stream** — they are stored under
//!    `(stream_id, delegate)`, not under `(recipient, delegate)`, so a transfer
//!    does not touch them.  A delegate that held `WITHDRAW` before the transfer
//!    can still call `delegate_withdraw` after it; the payout now goes to the
//!    *new* recipient, as it must, because `apply_withdrawal` always reads
//!    `stream.recipient` from live storage.
//!
//! 2. **Grants are silently cleared** — the transfer removes every grant that
//!    names the old recipient, treating the slot change as an implicit revoke.
//!
//! docs/ABI.md documents option 1:
//!
//! > "A transfer reassigns who is paid; it does not touch
//! > `Delegate(stream_id, delegate)` entries. Recipient-issued grants therefore
//! > pass to the new holder of the recipient slot."
//!
//! # What these tests prove
//!
//! | Test | What it pins |
//! |---|---|
//! | `withdraw_delegate_pays_new_recipient_after_transfer` | End-to-end: `WITHDRAW` grant survives, payout reaches the *new* recipient, conservation exact, events match storage |
//! | `stale_delegate_cannot_divert_funds_to_old_recipient` | After transfer the delegate's payout is determined by the live `stream.recipient` — the old address is unreachable |
//! | `sender_grant_survives_recipient_transfer` | Sender-side grants (`CANCEL`, `PAUSE`, `RESUME`, `TOP_UP`) are unaffected because the sender did not change |
//! | `new_recipient_can_revoke_surviving_withdraw_grant` | Authority over a survived recipient-issued grant moves to the new recipient immediately |
//! | `old_recipient_cannot_revoke_after_transfer` | The displaced recipient is no longer a party; revocation is rejected |
//! | `funds_conservation_holds_across_transfer_and_delegate_withdraw` | Pool invariant and exact conservation after the full sequence |

use soroban_sdk::testutils::{Address as _, Events as _};
use soroban_sdk::{Address, Event as _};

use super::common::*;
use crate::events::{RecipientTransferred, Withdrawn};
use crate::{op, Error, StreamStatus};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// All stream-contract events published during the last invocation, as a
/// plain `std::vec::Vec` so they can be indexed with `[n]` and `.len()`.
fn stream_events(h: &Harness) -> std::vec::Vec<soroban_sdk::xdr::ContractEvent> {
    h.env
        .events()
        .all()
        .filter_by_contract(&h.contract_id)
        .events()
        .to_vec()
}

// ---------------------------------------------------------------------------
// Core end-to-end scenario
// ---------------------------------------------------------------------------

/// # Scenario
///
/// 1. Create a 1 000 ONE / 100-day stream (`sender` → `recipient`).
/// 2. At day 40, `recipient` grants `WITHDRAW` to `agent`.
/// 3. `sender` reassigns the stream to `new_recipient` (a fresh address) via
///    `transfer_recipient`.
/// 4. At day 60, `agent` calls `delegate_withdraw(None)`.
///
/// # Expected outcomes (all per docs/ABI.md)
///
/// * `delegate_withdraw` succeeds — `DelegateNotPermitted` would mean the grant
///   was cleared on transfer.
/// * The payout (200 ONE: days 41–60 at 10 ONE/day) goes to `new_recipient`,
///   not to the original `recipient`.  The delegate only authorises the
///   operation; the destination is always `stream.recipient`.
/// * The old `recipient` receives no tokens from the stream.
/// * Pool conservation is exact after every step.
/// * The `withdrawn` event carries `new_recipient` as the `recipient` topic and
///   matches storage byte-for-byte.
#[test]
fn withdraw_delegate_pays_new_recipient_after_transfer() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    // Step 2: grant at day 40.
    h.advance(40 * DAY);
    let agent = Address::generate(&h.env);
    h.client
        .grant_delegate(&id, &h.recipient, &agent, &op::WITHDRAW, &None);
    h.assert_pool_exact();

    // At day 40, 400 ONE have vested but nothing has been withdrawn yet.
    assert_eq!(h.client.withdrawable_of(&id), 400 * ONE);

    // Step 3: transfer to a new recipient.  No tokens move here.
    let new_recipient = Address::generate(&h.env);
    let new_recipient_before = h.balance(&new_recipient);
    let old_recipient_before = h.balance(&h.recipient);

    h.client.transfer_recipient(&id, &new_recipient);

    // Capture the single `recipient_transferred` event.
    let transfer_events = stream_events(&h);
    assert_eq!(
        transfer_events.len(),
        1,
        "transfer_recipient must emit exactly one stream event"
    );
    let expected_transfer_event = RecipientTransferred {
        stream_id: id,
        old_recipient: h.recipient.clone(),
        new_recipient: new_recipient.clone(),
        sender: h.sender.clone(),
    };
    assert_eq!(
        transfer_events[0],
        expected_transfer_event.to_xdr(&h.env, &h.contract_id),
        "recipient_transferred event must match old and new recipient exactly"
    );

    // No tokens should have moved on the transfer call itself.
    assert_eq!(
        h.balance(&new_recipient),
        new_recipient_before,
        "transfer_recipient must not move tokens"
    );
    assert_eq!(
        h.balance(&h.recipient),
        old_recipient_before,
        "old recipient balance unchanged on transfer"
    );
    h.assert_pool_exact();

    // Stream state: only recipient changed.
    let stream_after_transfer = h.get(id);
    assert_eq!(stream_after_transfer.recipient, new_recipient);
    assert_eq!(stream_after_transfer.deposited, 1_000 * ONE);
    assert_eq!(stream_after_transfer.withdrawn, 0);
    assert_eq!(stream_after_transfer.status, StreamStatus::Active);

    // Step 4: agent calls delegate_withdraw at day 60.
    // Accrual from day 40 to day 60 = 200 ONE (the already-vested 400 ONE are
    // still available too; total withdrawable = 600 ONE).
    h.advance(20 * DAY);
    let expected_withdrawable = 600 * ONE;
    assert_eq!(h.client.withdrawable_of(&id), expected_withdrawable);

    let paid = h.client.delegate_withdraw(&id, &agent, &None);

    // Collect the withdrawn event immediately — before any other call.
    let withdraw_events = stream_events(&h);
    assert_eq!(
        withdraw_events.len(),
        1,
        "delegate_withdraw must emit exactly one stream event"
    );

    // Amounts: agent drains the full available balance.
    assert_eq!(
        paid, expected_withdrawable,
        "delegate_withdraw must return the full withdrawable amount"
    );

    // Token balances: new_recipient received the payout; old recipient did not.
    assert_eq!(
        h.balance(&new_recipient),
        new_recipient_before + expected_withdrawable,
        "new_recipient must receive the delegated payout"
    );
    assert_eq!(
        h.balance(&h.recipient),
        old_recipient_before,
        "old recipient must receive nothing from a post-transfer delegate withdrawal"
    );

    // Stream state: withdrawn advanced to 600 ONE.
    let stream_final = h.get(id);
    assert_eq!(stream_final.withdrawn, expected_withdrawable);
    assert_eq!(stream_final.deposited, 1_000 * ONE);
    assert_eq!(stream_final.recipient, new_recipient);
    assert_eq!(stream_final.status, StreamStatus::Active);

    // Event matches storage exactly: recipient topic is the *new* recipient.
    let expected_withdrawn_event = Withdrawn {
        stream_id: id,
        recipient: new_recipient.clone(),
        amount: expected_withdrawable,
        withdrawn: stream_final.withdrawn,
        deposited: stream_final.deposited,
        status: stream_final.status,
        sender: stream_final.sender.clone(),
        paused_at: stream_final.paused_at,
        paused_total: stream_final.paused_total,
    };
    assert_eq!(
        withdraw_events[0],
        expected_withdrawn_event.to_xdr(&h.env, &h.contract_id),
        "withdrawn event must name the new recipient and match storage"
    );

    // Conservation: pool holds exactly the remaining liability.
    h.assert_pool_exact();

    // Final state: 400 ONE remain unlocked for days 61–100.
    assert_eq!(
        h.client.refundable_of(&id),
        400 * ONE,
        "400 ONE still unvested, belongs to sender"
    );
}

// ---------------------------------------------------------------------------
// Funds routing — the delegate cannot divert to the old address
// ---------------------------------------------------------------------------

/// A delegate holding `WITHDRAW` cannot redirect the payout to an arbitrary
/// address.  `delegate_withdraw` takes no destination parameter; it always
/// pays `stream.recipient`.  After a transfer that field points to the new
/// holder, so the old address is structurally unreachable regardless of what
/// the delegate does.
///
/// This test forces the scenario where an agent granted by the *old* recipient
/// tries to act after the transfer and verifies:
/// * The call succeeds (the grant still exists).
/// * The tokens reach the *new* recipient.
/// * The old recipient's balance is unchanged.
#[test]
fn stale_delegate_cannot_divert_funds_to_old_recipient() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    let agent = Address::generate(&h.env);
    h.advance(30 * DAY);
    h.client
        .grant_delegate(&id, &h.recipient, &agent, &op::WITHDRAW, &None);

    let old_balance_before = h.balance(&h.recipient);

    // Transfer reassigns the stream; old recipient loses the payout destination.
    let new_recipient = Address::generate(&h.env);
    h.client.transfer_recipient(&id, &new_recipient);

    // Advance to accrue more value post-transfer.
    h.advance(10 * DAY);
    let withdrawable = h.client.withdrawable_of(&id);
    assert!(withdrawable > 0, "must have accrued value to withdraw");

    // Agent's grant is still alive — call must succeed.
    let paid = h.client.delegate_withdraw(&id, &agent, &None);
    assert_eq!(
        paid, withdrawable,
        "delegate_withdraw must drain the available balance"
    );

    // New recipient received the tokens.
    assert_eq!(
        h.balance(&new_recipient),
        withdrawable,
        "new_recipient must receive the payout"
    );

    // Old recipient received nothing from this withdrawal.
    assert_eq!(
        h.balance(&h.recipient),
        old_balance_before,
        "old recipient must not gain tokens after the transfer"
    );

    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Sender-side grants survive unchanged
// ---------------------------------------------------------------------------

/// Sender-issued grants (`CANCEL`, `PAUSE`, `RESUME`, `TOP_UP`) are attached to
/// the `(stream_id, delegate)` key, not to the recipient address.  A recipient
/// transfer does not change the sender and must not disturb these grants.
///
/// The test loops over every sender-side op bit, granting and then exercising
/// each after a transfer, confirming the contract does not mistake the
/// recipient change for a signal to clear sender-side grants.
#[test]
fn sender_grant_survives_recipient_transfer() {
    // Sender-side op bits.
    const SENDER_OPS: [u32; 4] = [op::CANCEL, op::PAUSE, op::RESUME, op::TOP_UP];

    for &op_bit in &SENDER_OPS {
        let h = Harness::new();
        let agent = Address::generate(&h.env);
        let id = h.create_simple(1_000 * ONE, 100 * DAY);

        // For RESUME we need the stream to be paused first.
        if op_bit == op::RESUME {
            h.client.pause(&id);
        }

        h.client
            .grant_delegate(&id, &h.sender, &agent, &op_bit, &None);

        // Transfer the stream.
        let new_recipient = Address::generate(&h.env);
        h.client.transfer_recipient(&id, &new_recipient);

        // The grant must still function — any `DelegateNotPermitted` here
        // would mean the transfer cleared a sender-side grant, which it must not.
        let result = match op_bit {
            op::CANCEL => h.client.try_delegate_cancel(&id, &agent).map(|_| ()),
            op::PAUSE => h.client.try_delegate_pause(&id, &agent).map(|_| ()),
            op::RESUME => h.client.try_delegate_resume(&id, &agent).map(|_| ()),
            op::TOP_UP => h
                .client
                .try_delegate_top_up(&id, &agent, &(100 * ONE))
                .map(|_| ()),
            other => panic!("unexpected op bit {other}"),
        };

        assert!(
            result.is_ok(),
            "op bit {op_bit}: sender-side grant must survive a recipient transfer; \
             got: {result:?}"
        );

        h.assert_pool_exact();
    }
}

// ---------------------------------------------------------------------------
// Authority over the survived grant follows the recipient slot
// ---------------------------------------------------------------------------

/// After a transfer the *new* recipient is the holder of the recipient slot and
/// therefore owns authority over any recipient-issued grants that survived.
/// They can revoke the agent's `WITHDRAW` grant immediately, and that revocation
/// takes effect on the very next call.
#[test]
fn new_recipient_can_revoke_surviving_withdraw_grant() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    let agent = Address::generate(&h.env);
    h.advance(20 * DAY);
    h.client
        .grant_delegate(&id, &h.recipient, &agent, &op::WITHDRAW, &None);

    let new_recipient = Address::generate(&h.env);
    h.client.transfer_recipient(&id, &new_recipient);

    // New recipient revokes the grant that survived.
    h.client.revoke_delegate(&id, &new_recipient, &agent);

    // Agent must now be rejected.
    let before = h.get(id);
    let err = h
        .client
        .try_delegate_withdraw(&id, &agent, &None)
        .unwrap_err()
        .unwrap();
    assert_eq!(
        err,
        Error::DelegateNotPermitted,
        "agent must be rejected after the new recipient revoked the grant"
    );

    // No state change from the rejected call.
    assert_eq!(
        h.get(id),
        before,
        "rejected delegate call must not mutate the stream"
    );

    h.assert_pool_exact();
}

/// The old recipient is no longer a party to the stream after the transfer.
/// Attempting to revoke the agent's grant from the old address must be rejected
/// with `Unauthorized`, and critically the grant must remain intact so the new
/// recipient can still use or revoke it.
#[test]
fn old_recipient_cannot_revoke_after_transfer() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    let agent = Address::generate(&h.env);
    h.advance(20 * DAY);
    h.client
        .grant_delegate(&id, &h.recipient, &agent, &op::WITHDRAW, &None);

    let new_recipient = Address::generate(&h.env);
    h.client.transfer_recipient(&id, &new_recipient);

    // Old recipient tries to revoke — must be rejected.
    let err = h
        .client
        .try_revoke_delegate(&id, &h.recipient, &agent)
        .unwrap_err()
        .unwrap();
    assert_eq!(
        err,
        Error::Unauthorized,
        "old recipient is no longer a party and must not revoke"
    );

    // The grant must still be alive — the rejected revocation must not have
    // cleared it as a side effect.
    let paid = h.client.delegate_withdraw(&id, &agent, &None);
    assert!(
        paid > 0,
        "grant must remain intact after rejected old-recipient revocation"
    );

    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Full conservation audit
// ---------------------------------------------------------------------------

/// Runs the complete sequence — create, partial withdraw, grant, transfer,
/// delegate_withdraw, remaining direct withdraw, final conservation — and
/// asserts the pool invariant and exact accounting after every step.
///
/// Ledger timeline (all relative to T0):
///
/// | day | event | pool change |
/// |-----|-------|-------------|
/// | 0   | create_stream(1 200 ONE, 120 days) | +1 200 ONE |
/// | 30  | recipient withdraws 200 ONE directly | −200 ONE |
/// | 30  | recipient grants WITHDRAW to agent | 0 |
/// | 30  | transfer_recipient to new_recipient | 0 |
/// | 60  | agent delegate_withdraw (max) | −300 ONE (days 31–60) |
/// | 90  | new_recipient direct withdraw (max) | −300 ONE (days 61–90) |
/// | 120 | new_recipient direct withdraw (max) | −400 ONE (days 91–120, remaining) |
/// | end | assert pool == 0 and stream Depleted | exact |
#[test]
fn funds_conservation_holds_across_transfer_and_delegate_withdraw() {
    let h = Harness::new();

    // 1 200 ONE over 120 days = 10 ONE/day exactly.
    let deposit = 1_200 * ONE;
    let duration = 120 * DAY;
    let id = h.create_simple(deposit, duration);
    h.assert_pool_exact();

    // Day 30: recipient withdraws 200 ONE (days 1–20 accrued so far, but wait —
    // we advance to day 30 first for a clean 300 ONE vested).
    h.advance(30 * DAY);
    assert_eq!(h.client.withdrawable_of(&id), 300 * ONE);
    let direct_1 = h.client.withdraw(&id, &Some(200 * ONE));
    assert_eq!(direct_1, 200 * ONE);
    h.assert_pool_exact();

    // Day 30: grant WITHDRAW to agent, then immediately transfer.
    let agent = Address::generate(&h.env);
    h.client
        .grant_delegate(&id, &h.recipient, &agent, &op::WITHDRAW, &None);

    let new_recipient = Address::generate(&h.env);
    h.client.transfer_recipient(&id, &new_recipient);
    h.assert_pool_exact();

    // Invariant check: conservation right after transfer.
    {
        let s = h.get(id);
        assert_eq!(
            s.deposited, deposit,
            "deposited must not change on transfer"
        );
        assert_eq!(s.withdrawn, 200 * ONE, "withdrawn carries forward");
        assert_eq!(
            h.client.vested_of(&id) + h.client.refundable_of(&id),
            deposit,
            "conservation: vested + refundable == deposited after transfer"
        );
    }

    // Day 60: agent drains the full accrued balance.
    // Accrued since day 30 = 300 ONE (days 31–60), plus 100 ONE left from before
    // direct_1 (we withdrew 200 of 300, leaving 100 still available).
    // Total withdrawable = 100 + 300 = 400 ONE.
    h.advance(30 * DAY);
    let expected_delegate_pay = 400 * ONE;
    assert_eq!(
        h.client.withdrawable_of(&id),
        expected_delegate_pay,
        "withdrawable before delegate_withdraw"
    );

    let delegate_paid = h.client.delegate_withdraw(&id, &agent, &None);
    assert_eq!(
        delegate_paid, expected_delegate_pay,
        "delegate_withdraw must drain the full available balance"
    );

    // Payout went to new_recipient, not old recipient.
    assert_eq!(
        h.balance(&new_recipient),
        expected_delegate_pay,
        "new_recipient must hold the delegated payout"
    );
    assert_eq!(
        h.balance(&h.recipient),
        200 * ONE,
        "old recipient retains only the earlier direct withdrawal"
    );
    h.assert_pool_exact();

    // Day 90: new_recipient withdraws directly (days 61–90 = 300 ONE).
    h.advance(30 * DAY);
    assert_eq!(h.client.withdrawable_of(&id), 300 * ONE);
    let direct_2 = h.client.withdraw(&id, &None);
    assert_eq!(direct_2, 300 * ONE);
    h.assert_pool_exact();

    // Day 120: new_recipient withdraws the final tranche (days 91–120 = 300 ONE).
    h.advance(30 * DAY);
    assert_eq!(h.client.withdrawable_of(&id), 300 * ONE);
    let direct_3 = h.client.withdraw(&id, &None);
    assert_eq!(direct_3, 300 * ONE);

    // Stream must now be Depleted.
    let final_stream = h.get(id);
    assert_eq!(
        final_stream.status,
        StreamStatus::Depleted,
        "stream must be Depleted after the full deposit is withdrawn"
    );
    assert_eq!(
        final_stream.withdrawn, deposit,
        "withdrawn must equal the full deposit"
    );
    assert_eq!(
        final_stream.deposited, deposit,
        "deposited is unchanged on a non-cancelled stream"
    );

    // Total tokens out = 200 (direct, old recipient) + 400 (delegate) + 300 + 300
    //                  = 1 200 ONE = deposit.  Pool must be exactly 0.
    assert_eq!(h.pool(), 0, "pool must be empty after full depletion");
    h.assert_pool_exact();

    // Final conservation: vested + refundable == deposited still holds on a
    // Depleted stream (vested == deposited, refundable == 0).
    assert_eq!(h.client.vested_of(&id), deposit);
    assert_eq!(h.client.refundable_of(&id), 0);
}
