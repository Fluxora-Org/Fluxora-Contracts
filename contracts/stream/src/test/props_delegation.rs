//! Property: a delegation grant never widens through any entry point (#1861).
//!
//! A grant is a bitmask scoped to one `(stream_id, delegate)` pair. The
//! security claim the whole delegation surface rests on is that the mask can
//! only ever be *written* by the party that owns the bits, and can only ever
//! *shrink* as a result of anything else. A single entry point that adds a bit
//! turns every delegate into a potential holder of every permission, so this is
//! asserted as a property over randomized call sequences rather than as a
//! handful of examples.
//!
//! # Model-based, not example-based
//!
//! The test drives a random sequence of calls to *every* entry point that can
//! touch a grant or use one, while maintaining a model of what storage must
//! contain: the mask of the last successful grant written by an owning party,
//! or "no grant" after a revocation.
//!
//! After **every** step the model is compared against the stored grant. That
//! comparison is what carries the property:
//!
//! * a grant written by a caller that does not own the bits (a delegate,
//!   an outsider, the *other* party) shows up as a mask the model never
//!   authorised;
//! * a mixed sender/recipient mask would likewise appear out of nowhere;
//! * a zero-bit grant that was supposed to be a documented no-op would leave a
//!   `Some(0)` where the model says `None`.
//!
//! It also asserts the *shape* of every mask ever stored — a subset of the
//! eight delegated bits, and never a mix of sender-side and recipient-side bits
//! (which `grant_delegate` rejects) — and, in the negative direction, that a
//! delegate call whose bit the mask does not carry is always rejected with
//! [`Error::DelegateNotPermitted`], so a removed bit check fails the property
//! instead of passing silently.
//!
//! # Why a deterministic PRNG rather than `proptest!`
//!
//! Every step is a host invocation, which costs orders of magnitude more than
//! the pure accrual functions the rest of this module drives, so the
//! `proptest!` macro's generator and shrinker machinery buys nothing here. This
//! follows `test::lifecycle_proptest` — the repository's existing convention for
//! host-driven randomized suites — with a value-dumped xorshift64\* PRNG,
//! deterministic per-case seeds, and `PROPTEST_CASES` read from the environment
//! so the existing proptest CI job's budget controls how many sequences run.
//! A failure reports the seed and step, and [`regression_seeds`] replays the
//! seeds that have ever failed.
//!
//! # Fixture
//!
//! One long-lived stream: not cancellable (so a granted `CANCEL` can never
//! settle the stream out from under the sequence) and never close to maturity,
//! so `grant_delegate`'s terminal-status guard is never what a case is
//! measuring. Expiry handling is covered by `test::delegation`; grants here are
//! issued without an expiry so a case cannot fail merely because time passed.

use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;

use super::super::common::*;
use crate::{op, storage, DelegateGrant, Error};

/// Bits only the sender may grant.
const SENDER_OPS: u32 = op::CANCEL | op::PAUSE | op::RESUME | op::TOP_UP;
/// Bits only the recipient may grant.
const RECIPIENT_OPS: u32 = op::WITHDRAW | op::TRANSFER_RECIPIENT;
/// Every delegated bit.
const ALL_OPS: u32 = SENDER_OPS | RECIPIENT_OPS;

const DURATION: u64 = 1_000 * DAY;
/// Steps per sequence. Bounded so that the fixture stream never matures and the
/// case count from `PROPTEST_CASES` stays affordable.
const STEPS: u32 = 16;

// ---------------------------------------------------------------------------
// PRNG — value-dumped xorshift64*, so a seed reproduces a sequence exactly.
// ---------------------------------------------------------------------------

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    /// A random subset of `mask`'s bits (possibly empty).
    fn submask(&mut self, mask: u32) -> u32 {
        let mut out = 0;
        let mut bit = 1u32;
        while bit != 0 {
            if mask & bit != 0 && self.below(2) == 1 {
                out |= bit;
            }
            bit <<= 1;
        }
        out
    }

    /// A random non-empty subset of `mask`'s bits.
    fn nonempty_submask(&mut self, mask: u32) -> u32 {
        let out = self.submask(mask);
        if out != 0 {
            out
        } else {
            // `mask` is never zero at the call sites, so this cannot shift out.
            1 << mask.trailing_zeros()
        }
    }

    /// One of the delegated bits, held by the grant or not.
    fn pick_bit(&mut self) -> u32 {
        const BITS: [u32; 6] = [
            op::WITHDRAW,
            op::CANCEL,
            op::PAUSE,
            op::RESUME,
            op::TOP_UP,
            op::TRANSFER_RECIPIENT,
        ];
        BITS[self.below(BITS.len() as u64) as usize]
    }
}

// ---------------------------------------------------------------------------
// Grant observation / call harness
// ---------------------------------------------------------------------------

/// The stored grant for `(stream_id, delegate)`, read from inside the contract
/// frame as the contract itself would read it.
fn stored_grant(h: &Harness, stream_id: u64, delegate: &Address) -> Option<DelegateGrant> {
    h.env.as_contract(&h.contract_id, || {
        storage::load_delegate(&h.env, stream_id, delegate)
    })
}

fn stored_ops(h: &Harness, id: u64, delegate: &Address) -> Option<u32> {
    stored_grant(h, id, delegate).map(|g| g.ops)
}

/// Collapse a generated `try_*` result to `Result<(), Error>`, panicking on the
/// two outcomes that would mean the delegation surface is not behaving: an
/// untyped host error escaping, or a return value that fails to decode.
fn collapse<T: core::fmt::Debug, D: core::fmt::Debug, I: core::fmt::Debug>(
    result: Result<Result<T, D>, Result<Error, I>>,
    what: &str,
) -> Result<(), Error> {
    match result {
        Err(Ok(err)) => Err(err),
        Ok(Ok(_)) => Ok(()),
        Ok(Err(decode)) => panic!("{what}: return value failed to decode: {decode:?}"),
        Err(Err(invoke)) => panic!("{what}: escaped the contract untyped: {invoke:?}"),
    }
}

/// Perform one grant entry point call.
fn grant_call(
    h: &Harness,
    id: u64,
    grantor: &Address,
    delegate: &Address,
    ops: u32,
) -> Result<(), Error> {
    collapse(
        h.client
            .try_grant_delegate(&id, grantor, delegate, &ops, &None),
        "grant_delegate",
    )
}

fn revoke_call(h: &Harness, id: u64, grantor: &Address, delegate: &Address) -> Result<(), Error> {
    collapse(
        h.client.try_revoke_delegate(&id, grantor, delegate),
        "revoke_delegate",
    )
}

/// Perform the delegated entry point gated on `bit`.
fn delegate_call(
    h: &Harness,
    id: u64,
    delegate: &Address,
    bit: u32,
    new_recipient: &Address,
) -> Result<(), Error> {
    match bit {
        op::WITHDRAW => collapse(
            h.client.try_delegate_withdraw(&id, delegate, &None),
            "delegate_withdraw",
        ),
        op::CANCEL => collapse(
            h.client.try_delegate_cancel(&id, delegate),
            "delegate_cancel",
        ),
        op::PAUSE => collapse(h.client.try_delegate_pause(&id, delegate), "delegate_pause"),
        op::RESUME => collapse(
            h.client.try_delegate_resume(&id, delegate),
            "delegate_resume",
        ),
        op::TOP_UP => collapse(
            h.client.try_delegate_top_up(&id, delegate, &(ONE)),
            "delegate_top_up",
        ),
        op::TRANSFER_RECIPIENT => collapse(
            h.client
                .try_delegate_transfer_recipient(&id, delegate, new_recipient),
            "delegate_transfer_recipient",
        ),
        other => panic!("unhandled op bit {other}"),
    }
}

// ---------------------------------------------------------------------------
// The property
// ---------------------------------------------------------------------------

/// The state the model expects storage to hold after each step.
struct Model {
    /// The mask storage must hold, or `None` for "no grant".
    ops: Option<u32>,
    /// The current recipient — moves on a successful recipient transfer, and is
    /// the grantor for recipient-side bits.
    recipient: Address,
}

fn check(h: &Harness, id: u64, delegate: &Address, model: &Model, seed: u64, step: u32) {
    let stored = stored_grant(h, id, delegate);
    let observed = stored.as_ref().map(|g| g.ops);

    assert_eq!(
        observed, model.ops,
        "seed {seed}, step {step}: stored grant {stored:?} does not match the \
         model {:?} — a grant was written or widened through a path that does \
         not own the bits",
        model.ops,
    );

    if let Some(ops) = observed {
        assert_eq!(
            ops & !ALL_OPS,
            0,
            "seed {seed}, step {step}: grant carries bits outside the delegated set",
        );
        assert!(
            !(ops & SENDER_OPS != 0 && ops & RECIPIENT_OPS != 0),
            "seed {seed}, step {step}: grant mixes sender-side and recipient-side bits",
        );
    }
}

/// Run one randomized sequence. Panics with the offending seed and step.
fn run_sequence(seed: u64, steps: u32) {
    let h = Harness::new();
    let id = h.create(
        1_000 * ONE,
        h.now(),
        h.now() + DURATION,
        h.now(),
        false,
        true,
        true,
    );
    let delegate = Address::generate(&h.env);
    let outsider = Address::generate(&h.env);

    let mut model = Model {
        ops: None,
        recipient: h.recipient.clone(),
    };
    let mut rng = Rng(seed);

    for step in 0..steps {
        let recipient = model.recipient.clone();

        match rng.below(13) {
            // 0 — the sender grants a subset of what it owns.
            0 => {
                let ops = rng.nonempty_submask(SENDER_OPS);
                assert!(
                    grant_call(&h, id, &h.sender, &delegate, ops).is_ok(),
                    "seed {seed}, step {step}: the sender may always grant its own bits",
                );
                model.ops = Some(ops);
            }
            // 1 — the recipient grants a subset of what it owns.
            1 => {
                let ops = rng.nonempty_submask(RECIPIENT_OPS);
                assert!(grant_call(&h, id, &recipient, &delegate, ops).is_ok());
                model.ops = Some(ops);
            }
            // 2 — a party tries to grant bits it does not own.
            2 => {
                let (grantor, stolen) = if rng.below(2) == 0 {
                    (h.sender.clone(), RECIPIENT_OPS)
                } else {
                    (recipient.clone(), SENDER_OPS)
                };
                let ops = rng.nonempty_submask(stolen);
                assert_eq!(
                    grant_call(&h, id, &grantor, &delegate, ops),
                    Err(Error::Unauthorized),
                    "seed {seed}, step {step}: a party must not grant the other \
                     party's bits",
                );
            }
            // 3 — a mixed grant (both sides' bits at once) is rejected.
            3 => {
                let ops = rng.nonempty_submask(SENDER_OPS) | rng.nonempty_submask(RECIPIENT_OPS);
                let grantor = if rng.below(2) == 0 {
                    h.sender.clone()
                } else {
                    recipient.clone()
                };
                assert_eq!(
                    grant_call(&h, id, &grantor, &delegate, ops),
                    Err(Error::Unauthorized),
                    "seed {seed}, step {step}: mixed grants must be rejected",
                );
            }
            // 4 — an address that is not a party at all cannot grant.
            4 => {
                let ops = rng.nonempty_submask(ALL_OPS);
                assert_eq!(
                    grant_call(&h, id, &outsider, &delegate, ops),
                    Err(Error::Unauthorized),
                    "seed {seed}, step {step}: a non-party must not grant",
                );
            }
            // 5 — the delegate tries to widen its own grant.
            5 => {
                assert_eq!(
                    grant_call(&h, id, &delegate, &delegate, ALL_OPS),
                    Err(Error::Unauthorized),
                    "seed {seed}, step {step}: a delegate must not grant to itself",
                );
            }
            // 6 — a zero-bit grant is the documented no-op: Ok, nothing written.
            6 => {
                let grantor = if rng.below(2) == 0 {
                    h.sender.clone()
                } else {
                    recipient.clone()
                };
                assert!(grant_call(&h, id, &grantor, &delegate, 0).is_ok());
            }
            // 7 — either party may revoke, at any time.
            7 => {
                let grantor = if rng.below(2) == 0 {
                    h.sender.clone()
                } else {
                    recipient.clone()
                };
                assert!(revoke_call(&h, id, &grantor, &delegate).is_ok());
                model.ops = None;
            }
            // 8 — a non-party cannot revoke.
            8 => {
                assert_eq!(
                    revoke_call(&h, id, &outsider, &delegate),
                    Err(Error::Unauthorized),
                    "seed {seed}, step {step}: a non-party must not revoke",
                );
            }
            // 9 — the delegate uses one of the six delegated entry points. It
            //     may succeed or fail for many reasons, but it must never
            //     change the grant, and it must be rejected outright when the
            //     mask does not carry the bit.
            9 => {
                let bit = rng.pick_bit();
                let new_recipient = Address::generate(&h.env);
                let before = stored_ops(&h, id, &delegate);
                let result = delegate_call(&h, id, &delegate, bit, &new_recipient);
                if model.ops.is_none_or(|ops| ops & bit == 0) {
                    assert_eq!(
                        result,
                        Err(Error::DelegateNotPermitted),
                        "seed {seed}, step {step}: bit {bit} is not held, so the \
                         delegate call must be rejected",
                    );
                }
                assert_eq!(
                    stored_ops(&h, id, &delegate),
                    before,
                    "seed {seed}, step {step}: using a grant must not modify it",
                );
                // A delegated recipient transfer moves the party that owns the
                // recipient-side bits, exactly as the owner path does.
                if bit == op::TRANSFER_RECIPIENT && result.is_ok() {
                    model.recipient = new_recipient;
                }
            }
            // 10 — the owner paths never touch a grant.
            10 => {
                let before = stored_ops(&h, id, &delegate);
                match rng.below(3) {
                    0 => {
                        let _ = h.client.try_withdraw(&id, &None);
                    }
                    1 => {
                        let _ = h.client.try_pause(&id);
                    }
                    _ => {
                        let _ = h.client.try_resume(&id);
                    }
                }
                assert_eq!(
                    stored_ops(&h, id, &delegate),
                    before,
                    "seed {seed}, step {step}: an owner path must not touch a grant",
                );
            }
            // 11 — a recipient transfer: grants survive, and authority over
            //      recipient-side bits moves with the party.
            11 => {
                let new_recipient = Address::generate(&h.env);
                let before = stored_ops(&h, id, &delegate);
                assert!(h.client.try_transfer_recipient(&id, &new_recipient).is_ok());
                assert_eq!(
                    stored_ops(&h, id, &delegate),
                    before,
                    "seed {seed}, step {step}: a recipient transfer must not \
                     change a grant",
                );
                model.recipient = new_recipient;
            }
            // 12 — the delegate calls views: no state may change at all.
            _ => {
                let before = stored_ops(&h, id, &delegate);
                let _ = h.client.try_withdrawable_of(&id);
                let _ = h.client.try_get_stream(&id);
                assert_eq!(stored_ops(&h, id, &delegate), before);
            }
        }

        check(&h, id, &delegate, &model, seed, step);

        // Keep the fixture alive without maturing it: the sequence runs at most
        // `steps * 5` days against a 1000-day stream.
        h.advance(1 + rng.below(5 * DAY));
    }

    // Final state, after the last clock movement.
    check(&h, id, &delegate, &model, seed, steps);
}

/// `PROPTEST_CASES` is the switch the existing proptest CI job already sets;
/// honouring it keeps this suite inside that job's budget.
fn case_count(default: u64) -> u64 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Property: across randomized sequences of grant, revoke, delegate and owner
/// calls, the stored grant always equals the mask the owning parties have
/// authorised — never a bit more.
#[test]
fn delegation_grants_never_widen_through_any_entry_point() {
    let cases = case_count(24);
    for i in 0..cases {
        let seed = 0xA24B_AED4_963E_E407u64
            .wrapping_mul(i.wrapping_add(1))
            .wrapping_add(0x9E37_79B9_7F4A_7C15);
        run_sequence(seed, STEPS);
    }
}

/// Seeds that have failed in the past, replayed verbatim. Add a seed here (with
/// the step count it failed at) whenever this property reports one, so the
/// failure stays covered even if the case budget drops.
#[test]
fn regression_seeds() {
    let seeds: [(u64, u32); 1] = [(0x6B5A_5A6B_5A6B_5A6Bu64, STEPS)];
    for (seed, steps) in seeds {
        run_sequence(seed, steps);
    }
}
