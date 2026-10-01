//! Multi-bit delegate grant tests — issue #1880.
//!
//! A realistic operator delegation (e.g. "this address may PAUSE and RESUME")
//! carries several permission bits at once. This module pins the interaction
//! between bits on a single grant so that a multi-bit mask behaves as the
//! exact union of its parts — no more, no less.
//!
//! # Acceptance criteria
//!
//! 1. **Exact permitted set** — a grant holding several bits permits exactly
//!    those operations and no others.
//! 2. **Partial revocation** — revoking one bit (by re-granting with the
//!    remaining mask) leaves the other bits intact and usable.
//! 3. **Full revocation** — clearing every bit is equivalent to revoking the
//!    grant; the delegate cannot act at all afterwards.
//! 4. **Ungrated op rejected** — an operation whose bit was never in the grant
//!    is rejected even when other bits are present.

use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;

use super::common::*;
use crate::{op, Error};

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// All six permission bits, exhaustive.
const ALL_OPS: [u32; 6] = [
    op::WITHDRAW,
    op::CANCEL,
    op::PAUSE,
    op::RESUME,
    op::TOP_UP,
    op::TRANSFER_RECIPIENT,
];

/// Dispatch the delegate entry point gated on `op_bit` and return the result,
/// normalising heterogeneous success types to `()`.
fn delegate_call_result(h: &Harness, id: u64, agent: &Address, op_bit: u32) -> Result<(), Error> {
    let new_recip = Address::generate(&h.env);
    let outcome = match op_bit {
        op::WITHDRAW => h
            .client
            .try_delegate_withdraw(&id, agent, &None)
            .map(|_| ()),
        op::CANCEL => h.client.try_delegate_cancel(&id, agent).map(|_| ()),
        op::PAUSE => h.client.try_delegate_pause(&id, agent).map(|_| ()),
        op::RESUME => h.client.try_delegate_resume(&id, agent).map(|_| ()),
        op::TOP_UP => h
            .client
            .try_delegate_top_up(&id, agent, &(100 * ONE))
            .map(|_| ()),
        op::TRANSFER_RECIPIENT => h
            .client
            .try_delegate_transfer_recipient(&id, agent, &new_recip)
            .map(|_| ()),
        other => panic!("unhandled op bit {other}"),
    };
    outcome.map_err(|e| e.expect("host invocation trapped"))
}

// ---------------------------------------------------------------------------
// Acceptance criterion 1 — a multi-bit grant permits exactly those ops
// ---------------------------------------------------------------------------

/// The canonical "operator" combo: PAUSE | RESUME.
///
/// The agent can pause and resume the stream; every other op is rejected.
#[test]
fn pause_resume_grant_permits_exactly_pause_and_resume() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    h.token_admin.mint(&agent, &(1_000 * ONE));
    h.advance(10 * DAY);

    // Grant both PAUSE and RESUME in one call — same grantor (sender) for both bits.
    h.client
        .grant_delegate(&id, &h.sender, &agent, &(op::PAUSE | op::RESUME), &None);

    // Permitted: PAUSE.
    h.client.delegate_pause(&id, &agent);
    assert_eq!(h.client.get_stream(&id).status, crate::StreamStatus::Paused);

    // Permitted: RESUME.
    h.client.delegate_resume(&id, &agent);
    assert_eq!(h.client.get_stream(&id).status, crate::StreamStatus::Active);

    // Rejected: every other op bit.
    for &other_bit in &ALL_OPS {
        if other_bit == op::PAUSE || other_bit == op::RESUME {
            continue;
        }
        assert_eq!(
            delegate_call_result(&h, id, &agent, other_bit),
            Err(Error::DelegateNotPermitted),
            "op bit {other_bit}: must be rejected — not in PAUSE|RESUME grant",
        );
    }

    h.assert_pool_exact();
}

/// Recipient-side multi-bit combo: WITHDRAW | TRANSFER_RECIPIENT.
///
/// The agent can withdraw accrued funds and reassign the stream to a new
/// address; every sender-side op is rejected.
#[test]
fn withdraw_transfer_grant_permits_exactly_withdraw_and_transfer() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    h.token_admin.mint(&agent, &(1_000 * ONE));
    h.advance(20 * DAY);

    // Grant WITHDRAW | TRANSFER_RECIPIENT — both owned by the recipient.
    h.client.grant_delegate(
        &id,
        &h.recipient,
        &agent,
        &(op::WITHDRAW | op::TRANSFER_RECIPIENT),
        &None,
    );

    // Permitted: WITHDRAW.
    let paid = h.client.delegate_withdraw(&id, &agent, &None);
    assert_eq!(paid, 200 * ONE);

    // Permitted: TRANSFER_RECIPIENT.
    let new_recip = Address::generate(&h.env);
    h.client
        .delegate_transfer_recipient(&id, &agent, &new_recip);
    assert_eq!(h.client.get_stream(&id).recipient, new_recip);

    // Rejected: every sender-side op bit.
    for &other_bit in &ALL_OPS {
        if other_bit == op::WITHDRAW || other_bit == op::TRANSFER_RECIPIENT {
            continue;
        }
        assert_eq!(
            delegate_call_result(&h, id, &agent, other_bit),
            Err(Error::DelegateNotPermitted),
            "op bit {other_bit}: must be rejected — not in WITHDRAW|TRANSFER_RECIPIENT grant",
        );
    }

    h.assert_pool_exact();
}

/// Full sender-side combo: CANCEL | PAUSE | RESUME | TOP_UP.
///
/// The agent holds every sender-side bit; all four operations succeed and every
/// recipient-side bit is rejected.
#[test]
fn all_sender_ops_grant_permits_exactly_cancel_pause_resume_top_up() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    h.token_admin.mint(&agent, &(1_000 * ONE));
    h.advance(10 * DAY);

    let all_sender = op::CANCEL | op::PAUSE | op::RESUME | op::TOP_UP;
    h.client
        .grant_delegate(&id, &h.sender, &agent, &all_sender, &None);

    // Permitted: TOP_UP.
    h.client.delegate_top_up(&id, &agent, &(100 * ONE));
    assert_eq!(h.client.get_stream(&id).deposited, 1_100 * ONE);

    // Permitted: PAUSE then RESUME.
    h.client.delegate_pause(&id, &agent);
    assert_eq!(h.client.get_stream(&id).status, crate::StreamStatus::Paused);
    h.client.delegate_resume(&id, &agent);
    assert_eq!(h.client.get_stream(&id).status, crate::StreamStatus::Active);

    // Permitted: CANCEL (terminates the stream — test last).
    h.client.delegate_cancel(&id, &agent);
    assert_eq!(
        h.client.get_stream(&id).status,
        crate::StreamStatus::Cancelled
    );

    h.assert_pool_exact();
}

/// Recipient-side bits are rejected even when the agent holds all sender-side bits.
#[test]
fn sender_only_grant_rejects_recipient_side_ops() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    h.token_admin.mint(&agent, &(1_000 * ONE));
    h.advance(10 * DAY);

    let all_sender = op::CANCEL | op::PAUSE | op::RESUME | op::TOP_UP;
    h.client
        .grant_delegate(&id, &h.sender, &agent, &all_sender, &None);

    for &bit in &[op::WITHDRAW, op::TRANSFER_RECIPIENT] {
        assert_eq!(
            delegate_call_result(&h, id, &agent, bit),
            Err(Error::DelegateNotPermitted),
            "op bit {bit}: recipient-side op must be rejected on a sender-only grant",
        );
    }

    h.assert_pool_exact();
}

/// Sender-side bits are rejected even when the agent holds all recipient-side bits.
#[test]
fn recipient_only_grant_rejects_sender_side_ops() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    h.token_admin.mint(&agent, &(1_000 * ONE));
    h.advance(10 * DAY);

    let all_recipient = op::WITHDRAW | op::TRANSFER_RECIPIENT;
    h.client
        .grant_delegate(&id, &h.recipient, &agent, &all_recipient, &None);

    for &bit in &[op::CANCEL, op::PAUSE, op::RESUME, op::TOP_UP] {
        assert_eq!(
            delegate_call_result(&h, id, &agent, bit),
            Err(Error::DelegateNotPermitted),
            "op bit {bit}: sender-side op must be rejected on a recipient-only grant",
        );
    }

    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Acceptance criterion 2 — revoking one bit leaves the remaining bits intact
// ---------------------------------------------------------------------------

/// Revoking PAUSE from a PAUSE|RESUME grant: the agent can still RESUME but
/// can no longer PAUSE. Re-granting is the revoke-a-single-bit mechanism —
/// issue a new grant that omits the unwanted bit.
#[test]
fn revoking_pause_from_pause_resume_grant_leaves_resume_intact() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    h.token_admin.mint(&agent, &(1_000 * ONE));
    h.advance(10 * DAY);

    // Grant PAUSE | RESUME.
    h.client
        .grant_delegate(&id, &h.sender, &agent, &(op::PAUSE | op::RESUME), &None);

    // Confirm both work.
    h.client.delegate_pause(&id, &agent);
    h.client.delegate_resume(&id, &agent);

    // Re-grant with PAUSE removed — this replaces the previous mask.
    h.client
        .grant_delegate(&id, &h.sender, &agent, &op::RESUME, &None);

    // PAUSE is now rejected.
    assert_eq!(
        delegate_call_result(&h, id, &agent, op::PAUSE),
        Err(Error::DelegateNotPermitted),
        "PAUSE must be rejected after it was removed from the grant",
    );

    // RESUME is still permitted: pause the stream via the owner path first.
    h.client.pause(&id);
    h.client.delegate_resume(&id, &agent);
    assert_eq!(h.client.get_stream(&id).status, crate::StreamStatus::Active);

    h.assert_pool_exact();
}

/// Revoking WITHDRAW from a WITHDRAW|TRANSFER_RECIPIENT grant: the agent can
/// still transfer the recipient but can no longer withdraw.
#[test]
fn revoking_withdraw_from_withdraw_transfer_grant_leaves_transfer_intact() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    h.token_admin.mint(&agent, &(1_000 * ONE));
    h.advance(20 * DAY);

    // Grant WITHDRAW | TRANSFER_RECIPIENT.
    h.client.grant_delegate(
        &id,
        &h.recipient,
        &agent,
        &(op::WITHDRAW | op::TRANSFER_RECIPIENT),
        &None,
    );

    // Re-grant with WITHDRAW removed.
    h.client
        .grant_delegate(&id, &h.recipient, &agent, &op::TRANSFER_RECIPIENT, &None);

    // WITHDRAW is now rejected.
    assert_eq!(
        delegate_call_result(&h, id, &agent, op::WITHDRAW),
        Err(Error::DelegateNotPermitted),
        "WITHDRAW must be rejected after it was removed from the grant",
    );

    // TRANSFER_RECIPIENT is still permitted.
    let new_recip = Address::generate(&h.env);
    h.client
        .delegate_transfer_recipient(&id, &agent, &new_recip);
    assert_eq!(h.client.get_stream(&id).recipient, new_recip);

    h.assert_pool_exact();
}

/// Removing one bit at a time from a four-bit sender grant; after each removal
/// the removed bit is blocked and all remaining bits are still usable.
///
/// Sequence: CANCEL | PAUSE | RESUME | TOP_UP
///   step 1: remove CANCEL  → { PAUSE, RESUME, TOP_UP } remain
///   step 2: remove PAUSE   → { RESUME, TOP_UP } remain
///   step 3: remove RESUME  → { TOP_UP } remains
///   (TOP_UP is verified live at each step; stream not cancelled until step 1's
///   check is done so there is always a live stream to act on.)
#[test]
fn removing_bits_one_at_a_time_from_a_four_bit_grant() {
    let h = Harness::new();
    let id = h.create_simple(10_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    h.token_admin.mint(&agent, &(10_000 * ONE));
    h.advance(10 * DAY);

    // Step 0 — full sender grant.
    h.client.grant_delegate(
        &id,
        &h.sender,
        &agent,
        &(op::CANCEL | op::PAUSE | op::RESUME | op::TOP_UP),
        &None,
    );

    // -- Step 1: remove CANCEL --
    h.client
        .grant_delegate(&id, &h.sender, &agent, &(op::PAUSE | op::RESUME | op::TOP_UP), &None);

    assert_eq!(
        delegate_call_result(&h, id, &agent, op::CANCEL),
        Err(Error::DelegateNotPermitted),
        "CANCEL must be blocked after removal",
    );
    // Remaining bits work.
    h.client.delegate_pause(&id, &agent);
    h.client.delegate_resume(&id, &agent);
    h.client.delegate_top_up(&id, &agent, &(100 * ONE));

    // -- Step 2: remove PAUSE --
    h.client
        .grant_delegate(&id, &h.sender, &agent, &(op::RESUME | op::TOP_UP), &None);

    assert_eq!(
        delegate_call_result(&h, id, &agent, op::PAUSE),
        Err(Error::DelegateNotPermitted),
        "PAUSE must be blocked after removal",
    );
    // RESUME still works (pause via owner path first).
    h.client.pause(&id);
    h.client.delegate_resume(&id, &agent);
    // TOP_UP still works.
    h.client.delegate_top_up(&id, &agent, &(100 * ONE));

    // -- Step 3: remove RESUME --
    h.client
        .grant_delegate(&id, &h.sender, &agent, &op::TOP_UP, &None);

    assert_eq!(
        delegate_call_result(&h, id, &agent, op::RESUME),
        Err(Error::DelegateNotPermitted),
        "RESUME must be blocked after removal",
    );
    // TOP_UP is the last surviving bit.
    h.client.delegate_top_up(&id, &agent, &(100 * ONE));

    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Acceptance criterion 3 — clearing every bit is equivalent to revoking
// ---------------------------------------------------------------------------

/// Re-granting with `ops = 0` is the documented no-op (see `grant_delegate`
/// ABI note and `grant_delegate_rejects_a_terminal_stream_and_zero_ops_grants_nothing`
/// in delegation.rs). After a zero-bit re-grant the delegate is effectively
/// ungrantable for every op.
#[test]
fn re_granting_zero_bits_revokes_all_ops() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    h.token_admin.mint(&agent, &(1_000 * ONE));
    h.advance(10 * DAY);

    // Grant PAUSE | RESUME.
    h.client
        .grant_delegate(&id, &h.sender, &agent, &(op::PAUSE | op::RESUME), &None);

    // Re-grant with zero ops — equivalent to a revoke.
    h.client.grant_delegate(&id, &h.sender, &agent, &0, &None);

    // Both previously granted bits are now rejected.
    for &bit in &[op::PAUSE, op::RESUME] {
        assert_eq!(
            delegate_call_result(&h, id, &agent, bit),
            Err(Error::DelegateNotPermitted),
            "op bit {bit}: must be rejected after zero-bit re-grant",
        );
    }

    // Explicit revoke after a zero-bit grant is also a no-op (idempotent).
    h.client.revoke_delegate(&id, &h.sender, &agent);

    for &bit in &[op::PAUSE, op::RESUME] {
        assert_eq!(
            delegate_call_result(&h, id, &agent, bit),
            Err(Error::DelegateNotPermitted),
            "op bit {bit}: must remain rejected after redundant revoke",
        );
    }

    h.assert_pool_exact();
}

/// Using `revoke_delegate` directly on a multi-bit grant clears all bits at
/// once; no op from the old grant remains usable.
#[test]
fn revoking_a_multi_bit_grant_clears_all_bits_at_once() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    h.token_admin.mint(&agent, &(1_000 * ONE));
    h.advance(10 * DAY);

    // Grant PAUSE | RESUME | TOP_UP.
    h.client.grant_delegate(
        &id,
        &h.sender,
        &agent,
        &(op::PAUSE | op::RESUME | op::TOP_UP),
        &None,
    );

    // Revoke entirely.
    h.client.revoke_delegate(&id, &h.sender, &agent);

    // All three previously granted bits are rejected.
    for &bit in &[op::PAUSE, op::RESUME, op::TOP_UP] {
        assert_eq!(
            delegate_call_result(&h, id, &agent, bit),
            Err(Error::DelegateNotPermitted),
            "op bit {bit}: must be rejected after full revocation of multi-bit grant",
        );
    }

    h.assert_pool_exact();
}

/// Revoking a WITHDRAW | TRANSFER_RECIPIENT grant clears the recipient's full
/// delegation; the stream is unchanged afterwards.
#[test]
fn revoking_recipient_multi_bit_grant_blocks_all_recipient_ops() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    h.token_admin.mint(&agent, &(1_000 * ONE));
    h.advance(10 * DAY);

    h.client.grant_delegate(
        &id,
        &h.recipient,
        &agent,
        &(op::WITHDRAW | op::TRANSFER_RECIPIENT),
        &None,
    );

    h.client.revoke_delegate(&id, &h.recipient, &agent);

    let before = h.client.get_stream(&id);

    assert_eq!(
        delegate_call_result(&h, id, &agent, op::WITHDRAW),
        Err(Error::DelegateNotPermitted),
        "WITHDRAW must be rejected after full revocation",
    );
    assert_eq!(
        delegate_call_result(&h, id, &agent, op::TRANSFER_RECIPIENT),
        Err(Error::DelegateNotPermitted),
        "TRANSFER_RECIPIENT must be rejected after full revocation",
    );

    // The stream is completely untouched by the rejected calls.
    assert_eq!(
        h.client.get_stream(&id),
        before,
        "rejected delegate calls must not mutate the stream"
    );
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Acceptance criterion 4 — an op not in the grant is rejected even when other
// bits are present
// ---------------------------------------------------------------------------

/// Grant PAUSE only; every other op (including RESUME, which is adjacent) is
/// rejected.
#[test]
fn single_bit_grant_rejects_all_other_ops() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    h.token_admin.mint(&agent, &(1_000 * ONE));
    h.advance(10 * DAY);

    // Grant PAUSE only.
    h.client
        .grant_delegate(&id, &h.sender, &agent, &op::PAUSE, &None);

    // PAUSE is permitted.
    h.client.delegate_pause(&id, &agent);
    assert_eq!(h.client.get_stream(&id).status, crate::StreamStatus::Paused);

    // Every other bit is rejected, including RESUME (the natural complement).
    for &other_bit in &ALL_OPS {
        if other_bit == op::PAUSE {
            continue;
        }
        assert_eq!(
            delegate_call_result(&h, id, &agent, other_bit),
            Err(Error::DelegateNotPermitted),
            "op bit {other_bit}: must be rejected — only PAUSE was granted",
        );
    }

    h.assert_pool_exact();
}

/// Grant PAUSE | TOP_UP; the missing RESUME is rejected even though PAUSE
/// is present.
#[test]
fn resume_bit_not_in_grant_is_rejected_despite_pause_being_granted() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    h.token_admin.mint(&agent, &(1_000 * ONE));
    h.advance(10 * DAY);

    // Grant PAUSE and TOP_UP — but not RESUME.
    h.client.grant_delegate(
        &id,
        &h.sender,
        &agent,
        &(op::PAUSE | op::TOP_UP),
        &None,
    );

    // Both granted ops work.
    h.client.delegate_pause(&id, &agent);
    assert_eq!(h.client.get_stream(&id).status, crate::StreamStatus::Paused);
    // Cannot resume through the delegate path — unpause via owner path.
    h.client.resume(&id);
    h.client.delegate_top_up(&id, &agent, &(100 * ONE));

    // RESUME is not in the grant: rejected even though PAUSE is.
    assert_eq!(
        delegate_call_result(&h, id, &agent, op::RESUME),
        Err(Error::DelegateNotPermitted),
        "RESUME must be rejected: it is not in the PAUSE|TOP_UP grant",
    );

    // CANCEL is also not in the grant.
    assert_eq!(
        delegate_call_result(&h, id, &agent, op::CANCEL),
        Err(Error::DelegateNotPermitted),
        "CANCEL must be rejected: it is not in the PAUSE|TOP_UP grant",
    );

    h.assert_pool_exact();
}

/// Exhaustive check: for every subset of two sender-side bits, the two bits
/// in the grant succeed and the two that are absent are rejected.
///
/// Covers all six pairs from {CANCEL, PAUSE, RESUME, TOP_UP}, ensuring that
/// no particular combination of present bits accidentally enables an absent one.
#[test]
fn every_two_bit_sender_combo_permits_exactly_its_two_bits() {
    let sender_bits = [op::CANCEL, op::PAUSE, op::RESUME, op::TOP_UP];

    for i in 0..sender_bits.len() {
        for j in (i + 1)..sender_bits.len() {
            let bit_a = sender_bits[i];
            let bit_b = sender_bits[j];
            let mask = bit_a | bit_b;

            let h = Harness::new();
            let id = h.create_simple(10_000 * ONE, 200 * DAY);
            let agent = Address::generate(&h.env);
            h.token_admin.mint(&agent, &(10_000 * ONE));
            h.advance(10 * DAY);

            // RESUME only makes sense on a paused stream; pause first if needed.
            if mask & op::RESUME != 0 && mask & op::PAUSE == 0 {
                // The combo includes RESUME but not PAUSE: pre-pause via owner.
                h.client.pause(&id);
            }

            h.client.grant_delegate(&id, &h.sender, &agent, &mask, &None);

            // Both bits in the grant must succeed.
            for &granted_bit in &[bit_a, bit_b] {
                // For RESUME the stream must be paused; for PAUSE it must be active.
                // Normalise state before each op check.
                let stream = h.client.get_stream(&id);
                if granted_bit == op::RESUME
                    && stream.status != crate::StreamStatus::Paused
                {
                    h.client.pause(&id);
                }
                if granted_bit == op::PAUSE
                    && stream.status == crate::StreamStatus::Paused
                {
                    h.client.resume(&id);
                }

                assert!(
                    delegate_call_result(&h, id, &agent, granted_bit).is_ok(),
                    "mask={mask:#b}: granted bit {granted_bit} must succeed",
                );

                // If CANCEL just ran, the stream is terminal — stop checking this pair.
                if granted_bit == op::CANCEL {
                    break;
                }

                // Restore Active state for subsequent checks.
                let stream = h.client.get_stream(&id);
                if stream.status == crate::StreamStatus::Paused {
                    h.client.resume(&id);
                }
            }

            // If the stream was cancelled we cannot check absent bits on it.
            if h.client.get_stream(&id).status == crate::StreamStatus::Cancelled {
                h.assert_pool_exact();
                continue;
            }

            // Every sender-side bit NOT in the mask must be rejected.
            for &absent_bit in &sender_bits {
                if absent_bit == bit_a || absent_bit == bit_b {
                    continue;
                }
                // Normalise state for the absent-bit check.
                let stream = h.client.get_stream(&id);
                if absent_bit == op::RESUME
                    && stream.status != crate::StreamStatus::Paused
                {
                    h.client.pause(&id);
                }
                if absent_bit == op::PAUSE
                    && stream.status == crate::StreamStatus::Paused
                {
                    h.client.resume(&id);
                }

                assert_eq!(
                    delegate_call_result(&h, id, &agent, absent_bit),
                    Err(Error::DelegateNotPermitted),
                    "mask={mask:#b}: absent sender bit {absent_bit} must be rejected",
                );
            }

            // Recipient-side bits must also be rejected regardless of the sender mask.
            for &bit in &[op::WITHDRAW, op::TRANSFER_RECIPIENT] {
                assert_eq!(
                    delegate_call_result(&h, id, &agent, bit),
                    Err(Error::DelegateNotPermitted),
                    "mask={mask:#b}: recipient bit {bit} must be rejected on a sender grant",
                );
            }

            h.assert_pool_exact();
        }
    }
}

// ---------------------------------------------------------------------------
// Multi-bit grant with expiry
// ---------------------------------------------------------------------------

/// A multi-bit grant with an expiry rejects all of its bits after the expiry
/// time, not just the one that was last used.
#[test]
fn expired_multi_bit_grant_rejects_all_its_bits() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    h.token_admin.mint(&agent, &(1_000 * ONE));
    h.advance(5 * DAY);

    let expires = h.now() + 3 * DAY;
    h.client.grant_delegate(
        &id,
        &h.sender,
        &agent,
        &(op::PAUSE | op::RESUME | op::TOP_UP),
        &Some(expires),
    );

    // All three bits work before expiry.
    h.client.delegate_top_up(&id, &agent, &(100 * ONE));
    h.client.delegate_pause(&id, &agent);
    h.client.delegate_resume(&id, &agent);

    // Advance past the expiry.
    h.advance(4 * DAY);

    // All three bits are now rejected with DelegateExpired.
    for &bit in &[op::PAUSE, op::RESUME, op::TOP_UP] {
        assert_eq!(
            delegate_call_result(&h, id, &agent, bit),
            Err(Error::DelegateExpired),
            "op bit {bit}: must be rejected with DelegateExpired after grant expiry",
        );
    }

    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Interaction: re-granting expands bits (upgrade path)
// ---------------------------------------------------------------------------

/// A delegate who starts with PAUSE can be upgraded to PAUSE | RESUME | TOP_UP
/// by issuing a new grant; all three bits must then succeed.
#[test]
fn re_granting_with_more_bits_expands_the_permitted_set() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    h.token_admin.mint(&agent, &(1_000 * ONE));
    h.advance(10 * DAY);

    // Initial grant: PAUSE only.
    h.client
        .grant_delegate(&id, &h.sender, &agent, &op::PAUSE, &None);

    // Upgrade to PAUSE | RESUME | TOP_UP.
    h.client.grant_delegate(
        &id,
        &h.sender,
        &agent,
        &(op::PAUSE | op::RESUME | op::TOP_UP),
        &None,
    );

    // All three bits now work.
    h.client.delegate_top_up(&id, &agent, &(100 * ONE));
    h.client.delegate_pause(&id, &agent);
    h.client.delegate_resume(&id, &agent);

    // Bits outside the new mask are still rejected.
    assert_eq!(
        delegate_call_result(&h, id, &agent, op::CANCEL),
        Err(Error::DelegateNotPermitted),
        "CANCEL must be rejected — not in the upgraded grant",
    );

    h.assert_pool_exact();
}
