//! Issue #1804 — calibrate `MAX_BATCH_SIZE` against more than one token
//! implementation.
//!
//! `docs/KNOWN-LIMITATIONS.md` §3 observes that `MAX_BATCH_SIZE = 16`
//! (`contracts/stream/src/lib.rs`) was calibrated against a single token
//! implementation: the Stellar Asset Contract. The *binding* transaction
//! resource for a batch is the contract event budget (16,384 bytes), because
//! every paid-out stream emits a `withdrawn` event **and** drives one
//! `transfer` event inside its token contract. A token whose transfer event is
//! heavier therefore spends the same budget faster, and a 16-element batch that
//! is comfortable against the SAC can be impossible against a chattier token.
//!
//! This module re-derives the ceiling by measurement across four token
//! implementations of deliberately different per-transfer cost:
//!
//! | profile  | token                    | per-transfer event payload |
//! |----------|--------------------------|----------------------------|
//! | `Sparse` | test-only                | none                       |
//! | `Sac`    | Stellar Asset Contract   | baseline                   |
//! | `Chatty` | test-only                | ~256 bytes                 |
//! | `Bloated`| test-only                | ~2,048 bytes               |
//!
//! # Calibration method
//!
//! For each profile, on a fresh harness:
//!
//! 1. **Isolate the token's own event cost.** Call `transfer` directly through
//!    the token client and read `contract_events_size_bytes` from
//!    `env.cost_estimate().resources()`. This is the per-transfer event cost
//!    with no stream-contract event mixed in.
//! 2. **Run a full-cap batch.** Create [`MAX_BATCH_SIZE`] streams against that
//!    token, advance the clock so every stream has something to withdraw, and
//!    call `batch_withdraw` once. The resource vector now covers the whole
//!    transaction, token sub-invocations included.
//! 3. **Derive the implied ceiling.** Every stream in a batch costs the same
//!    fixed number of event bytes, so
//!    `ceiling = EVENT_BYTES_LIMIT / (at_cap_event_bytes / MAX_BATCH_SIZE)`.
//!    The same ratio over the entry footprint gives the footprint ceiling.
//! 4. **Assert against the enforced limits.** The SDK test host enforces the
//!    Stellar mainnet `InvocationResourceLimits` on every invocation by
//!    default, so a batch that would not fit on-network cannot even be
//!    measured. Where a profile is *expected* to overflow (the `Bloated` one),
//!    enforcement is switched off for that single measurement and the measured
//!    figure is asserted to exceed the real limit — the defect is measured and
//!    documented, not hidden.
//!
//! The figures this module prints (`cargo test ... -- --nocapture`) are the
//! calibration record.
//!
//! # What the calibration shows
//!
//! * The SAC baseline keeps the documented 2x headroom on every enforced limit
//!   at the cap, and the event budget alone admits at least `2 * 16` streams.
//! * `Sparse` and `Chatty` also fit at the cap; their implied ceilings are
//!   strictly different, which is the whole point of the issue.
//! * `Bloated` does **not** fit at the cap: its implied ceiling is a smaller,
//!   still-useful chunk size. The stream contract cannot detect this — the cost
//!   lives inside the token's own event — so the over-cap rejection
//!   (`BatchTooLarge`, discriminant 19) is what the contract *can* do, and it
//!   does it before the token is touched. That is asserted explicitly.

use soroban_sdk::testutils::Events as _;
use soroban_sdk::token::Client as TokenClient;
use soroban_sdk::{contract, contractevent, contractimpl, Address, Bytes, Env, MuxedAddress};

use super::common::*;
use crate::{Error, MAX_BATCH_SIZE};

// ---------------------------------------------------------------------------
// Enforced protocol limits (Stellar mainnet snapshot, protocol 27)
// ---------------------------------------------------------------------------

/// Maximum total transaction footprint: disk reads + memory reads + writes.
const LEDGER_ENTRY_LIMIT: u32 = 400;
/// Maximum entries one transaction may write.
const WRITE_ENTRY_LIMIT: u32 = 200;
/// Maximum total size of emitted contract events, in bytes.
const EVENT_BYTES_LIMIT: u32 = 16_384;
/// Maximum modelled CPU instructions per invocation.
const INSTRUCTION_LIMIT: i64 = 400_000_000;

/// Tokens minted to the sender for every test-only token.
const FUNDING: i128 = 1_000_000 * ONE;
/// One stream's deposit in every calibration fixture.
const DEPOSIT: i128 = 100 * ONE;
/// Stream duration, so [`ELAPSED`] of advance vests exactly `30 * ONE`.
const DURATION: u64 = 100 * DAY;
/// Seconds of accrual before a batch is run.
const ELAPSED: u64 = 30 * DAY;

// ---------------------------------------------------------------------------
// Test-only token contracts
// ---------------------------------------------------------------------------

/// Persistent balance store shared by every calibrating token.
fn token_balance(env: &Env, id: &Address) -> i128 {
    env.storage().persistent().get(id).unwrap_or(0)
}

/// Persistent balance store shared by every calibrating token.
fn set_token_balance(env: &Env, id: &Address, amount: i128) {
    env.storage().persistent().set(id, &amount);
}

/// The transfer event every calibrating token emits.
///
/// Declared with `#[contractevent]` so the test token exports a schematised
/// event exactly like a real SEP-41 token, rather than using the deprecated raw
/// `Events::publish`. `payload` carries a caller-chosen number of bytes, which
/// is what makes the four profiles cost different amounts of the event budget.
#[contractevent]
pub struct CalibratedTransfer {
    #[topic]
    pub from: Address,
    pub payload: Bytes,
}

/// Publish one [`CalibratedTransfer`] event whose data payload is `bytes`.
///
/// An empty payload emits nothing at all — that is the `Sparse` profile, and it
/// is deliberately not an empty event, which would still cost event bytes.
fn publish_sized_event(env: &Env, from: &Address, bytes: &[u8]) {
    if bytes.is_empty() {
        return;
    }
    CalibratedTransfer {
        from: from.clone(),
        payload: Bytes::from_slice(env, bytes),
    }
    .publish(env);
}

/// Define a SEP-41-shaped token whose `transfer` moves balances between
/// persistent entries and emits one event carrying `$payload` bytes.
///
/// Only `transfer` and `balance` are implemented: those are the two calls the
/// stream contract makes (`balance` from `pull_deposit`, `transfer` from
/// `apply_withdrawal`), and a minimal surface keeps the footprint comparison
/// against the SAC honest.
macro_rules! calibrating_token {
    ($name:ident, $payload:literal, $doc:literal) => {
        #[doc = $doc]
        #[contract]
        pub struct $name;

        #[contractimpl]
        impl $name {
            /// Test-only mint, bypassing transfer semantics entirely.
            pub fn mint(env: Env, to: Address, amount: i128) {
                let balance = token_balance(&env, &to);
                set_token_balance(&env, &to, balance + amount);
            }

            pub fn balance(env: Env, id: Address) -> i128 {
                token_balance(&env, &id)
            }

            pub fn transfer(env: Env, from: Address, to: MuxedAddress, amount: i128) {
                from.require_auth();
                let to = to.address();
                let from_balance = token_balance(&env, &from);
                assert!(
                    from_balance >= amount,
                    "calibrating token: insufficient balance",
                );
                set_token_balance(&env, &from, from_balance - amount);
                let to_balance = token_balance(&env, &to);
                set_token_balance(&env, &to, to_balance + amount);

                publish_sized_event(&env, &from, &[0u8; $payload]);
            }
        }
    };
}

calibrating_token!(
    SparseToken,
    0,
    "A token whose `transfer` emits no event at all — the cheapest profile."
);
calibrating_token!(
    ChattyToken,
    256,
    "A token whose `transfer` emits a ~256 byte event payload."
);
calibrating_token!(
    BloatedToken,
    2_048,
    "A token whose `transfer` emits a ~2 KB event payload, enough to overrun \
     the 16,384-byte event budget at the batch cap."
);

// ---------------------------------------------------------------------------
// Profiles and measurement
// ---------------------------------------------------------------------------

/// The token implementations the ceiling is calibrated against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Profile {
    Sparse,
    Sac,
    Chatty,
    Bloated,
}

impl Profile {
    const ALL: [Profile; 4] = [
        Profile::Sparse,
        Profile::Sac,
        Profile::Chatty,
        Profile::Bloated,
    ];

    fn label(self) -> &'static str {
        match self {
            Profile::Sparse => "sparse (no transfer event)",
            Profile::Sac => "stellar asset contract (baseline)",
            Profile::Chatty => "chatty (~256 B event)",
            Profile::Bloated => "bloated (~2 KB event)",
        }
    }

    /// The event payload the profile's token is built to emit, in bytes. The
    /// SAC is not ours to size, so its nominal payload is reported as zero and
    /// only the *measured* figure is used for it.
    fn nominal_event_payload(self) -> u32 {
        match self {
            Profile::Sparse | Profile::Sac => 0,
            Profile::Chatty => 256,
            Profile::Bloated => 2_048,
        }
    }
}

/// The resource dimensions a batch is judged on, read from the host's own
/// metering of the last top-level invocation.
#[derive(Debug, Clone, Copy)]
struct Cost {
    /// Total footprint: disk reads + memory reads + writes.
    footprint: u32,
    /// Entries written.
    writes: u32,
    /// In-memory ledger entries read.
    memory: u32,
    /// Modelled CPU instructions.
    instructions: i64,
    /// Total size of every contract event emitted, token sub-calls included.
    event_bytes: u32,
}

/// Snapshot the last invocation's measured resources.
fn cost_of(h: &Harness) -> Cost {
    let r = h.env.cost_estimate().resources();
    Cost {
        footprint: r.disk_read_entries + r.memory_read_entries + r.write_entries,
        writes: r.write_entries,
        memory: r.memory_read_entries,
        instructions: r.instructions,
        event_bytes: r.contract_events_size_bytes,
    }
}

/// One profile's calibration record.
#[derive(Debug, Clone, Copy)]
struct ProfileCost {
    profile: Profile,
    /// Event bytes attributable to a single token `transfer`, measured by
    /// calling the token in isolation.
    per_transfer_event_bytes: u32,
    /// The measured resources of a full-[`MAX_BATCH_SIZE`] batch.
    at_cap: Cost,
}

impl ProfileCost {
    /// Event bytes one paid-out stream contributes to a batch: the token's own
    /// transfer event plus the stream contract's `withdrawn` event.
    fn per_stream_event_bytes(self) -> u32 {
        self.at_cap.event_bytes / MAX_BATCH_SIZE
    }

    /// Entries one paid-out stream contributes to the footprint.
    fn per_stream_footprint(self) -> u32 {
        (self.at_cap.footprint / MAX_BATCH_SIZE).max(1)
    }

    /// How many streams the event budget alone would admit at this profile's
    /// per-stream cost.
    fn event_ceiling(self) -> u32 {
        EVENT_BYTES_LIMIT / self.per_stream_event_bytes().max(1)
    }

    /// How many streams the entry footprint alone would admit.
    fn footprint_ceiling(self) -> u32 {
        LEDGER_ENTRY_LIMIT / self.per_stream_footprint()
    }

    /// Whether the measured full-cap batch sits inside every enforced limit.
    fn fits_at_cap(self) -> bool {
        self.at_cap.footprint <= LEDGER_ENTRY_LIMIT
            && self.at_cap.writes <= WRITE_ENTRY_LIMIT
            && self.at_cap.memory <= LEDGER_ENTRY_LIMIT
            && self.at_cap.instructions <= INSTRUCTION_LIMIT
            && self.at_cap.event_bytes <= EVENT_BYTES_LIMIT
    }

    /// Print the calibration record for one profile.
    fn report(self) {
        std::println!(
            "{:<28} per-transfer events={:<6} per-stream events={:<6} \
             at-cap events={}/{} footprint={}/{} writes={}/{} mem={} \
             instructions={} implied ceiling={} (footprint ceiling={})",
            self.profile.label(),
            self.per_transfer_event_bytes,
            self.per_stream_event_bytes(),
            self.at_cap.event_bytes,
            EVENT_BYTES_LIMIT,
            self.at_cap.footprint,
            LEDGER_ENTRY_LIMIT,
            self.at_cap.writes,
            WRITE_ENTRY_LIMIT,
            self.at_cap.memory,
            self.at_cap.instructions,
            self.event_ceiling(),
            self.footprint_ceiling(),
        );
    }
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// Register (and, for the test-only tokens, fund) the profile's token.
///
/// The SAC profile reuses the harness's own asset, which is already funded.
fn register_token(h: &Harness, profile: Profile) -> Address {
    match profile {
        Profile::Sac => h.token.clone(),
        Profile::Sparse => {
            let token = h.env.register(SparseToken, ());
            SparseTokenClient::new(&h.env, &token).mint(&h.sender, &FUNDING);
            token
        }
        Profile::Chatty => {
            let token = h.env.register(ChattyToken, ());
            ChattyTokenClient::new(&h.env, &token).mint(&h.sender, &FUNDING);
            token
        }
        Profile::Bloated => {
            let token = h.env.register(BloatedToken, ());
            BloatedTokenClient::new(&h.env, &token).mint(&h.sender, &FUNDING);
            token
        }
    }
}

/// Create `count` identical streams denominated in `token`.
fn create_streams(h: &Harness, token: &Address, count: u32) -> std::vec::Vec<u64> {
    let start = h.now();
    (0..count)
        .map(|_| {
            h.client.create_stream(
                &h.sender,
                &h.recipient,
                token,
                &DEPOSIT,
                &start,
                &(start + DURATION),
                &start,
                &true,
                &true,
                &true,
                &None,
            )
        })
        .collect()
}

/// The balance of `who` in `token`, via the generic token interface.
fn balance_of(h: &Harness, token: &Address, who: &Address) -> i128 {
    TokenClient::new(&h.env, token).balance(who)
}

/// Measure the event cost of a single `transfer` by calling the token directly,
/// with no stream contract involved.
fn measure_single_transfer(h: &Harness, profile: Profile, token: &Address) -> u32 {
    let to = MuxedAddress::from(h.recipient.clone());
    match profile {
        Profile::Sac => TokenClient::new(&h.env, token).transfer(&h.sender, &to, &ONE),
        Profile::Sparse => SparseTokenClient::new(&h.env, token).transfer(&h.sender, &to, &ONE),
        Profile::Chatty => ChattyTokenClient::new(&h.env, token).transfer(&h.sender, &to, &ONE),
        Profile::Bloated => BloatedTokenClient::new(&h.env, token).transfer(&h.sender, &to, &ONE),
    }
    cost_of(h).event_bytes
}

/// Full calibration run for one profile: isolate the token's event cost, then
/// measure a full-cap batch.
///
/// The `Bloated` profile is expected to overflow the event budget at the cap,
/// so the enforced limits are switched off for that one measurement; the
/// measured number is then asserted against the real limit by the caller.
fn calibrate(profile: Profile) -> ProfileCost {
    let h = Harness::new();
    let token = register_token(&h, profile);
    let per_transfer_event_bytes = measure_single_transfer(&h, profile, &token);

    let ids = create_streams(&h, &token, MAX_BATCH_SIZE);
    h.advance(ELAPSED);

    if profile == Profile::Bloated {
        h.env.cost_estimate().disable_resource_limits();
    }

    h.client.batch_withdraw(&h.recipient, &h.ids(&ids));
    let at_cap = cost_of(&h);

    ProfileCost {
        profile,
        per_transfer_event_bytes,
        at_cap,
    }
}

/// Assert the documented 2x headroom on every enforced limit, mirroring
/// `test::resource_limits::assert_has_headroom`.
fn assert_two_x_headroom(label: &str, cost: Cost) {
    assert!(
        cost.footprint * 2 <= LEDGER_ENTRY_LIMIT,
        "{label}: footprint {} lacks 2x headroom under {LEDGER_ENTRY_LIMIT}",
        cost.footprint,
    );
    assert!(
        cost.writes * 2 <= WRITE_ENTRY_LIMIT,
        "{label}: {} writes lack 2x headroom under {WRITE_ENTRY_LIMIT}",
        cost.writes,
    );
    assert!(
        cost.memory * 2 <= LEDGER_ENTRY_LIMIT,
        "{label}: {} in-memory reads lack 2x headroom under {LEDGER_ENTRY_LIMIT}",
        cost.memory,
    );
    assert!(
        cost.instructions <= INSTRUCTION_LIMIT,
        "{label}: {} instructions exceed {INSTRUCTION_LIMIT}",
        cost.instructions,
    );
    assert!(
        cost.event_bytes * 2 <= EVENT_BYTES_LIMIT,
        "{label}: {} event bytes lack 2x headroom under {EVENT_BYTES_LIMIT}",
        cost.event_bytes,
    );
}

// ---------------------------------------------------------------------------
// 1. Baseline
// ---------------------------------------------------------------------------

/// The Stellar Asset Contract baseline — the token `MAX_BATCH_SIZE` was
/// originally calibrated against — keeps the documented 2x headroom on every
/// enforced limit, and the event budget alone still admits at least
/// `2 * MAX_BATCH_SIZE` streams.
#[test]
fn the_baseline_token_keeps_the_documented_two_x_headroom() {
    let baseline = calibrate(Profile::Sac);
    baseline.report();

    assert_two_x_headroom("SAC baseline at the cap", baseline.at_cap);

    // docs/ABI.md: "Sixteen is the measured ceiling with a 2x safety factor."
    // That is a claim about the *event* budget, so assert it directly.
    let ceiling = baseline.event_ceiling();
    assert!(
        ceiling >= 2 * MAX_BATCH_SIZE,
        "documented 2x headroom requires an event ceiling of at least {} streams; \
         measured ceiling is {ceiling} ({} event bytes per stream)",
        2 * MAX_BATCH_SIZE,
        baseline.per_stream_event_bytes(),
    );
    assert_eq!(MAX_BATCH_SIZE, 16, "the committed cap is 16");

    // The measured per-transfer event cost is the token's share of that.
    assert!(
        baseline.per_stream_event_bytes() > baseline.per_transfer_event_bytes,
        "a batch stream must cost the token's transfer event plus the stream's \
         own withdrawn event",
    );
}

// ---------------------------------------------------------------------------
// 2. Fits at the cap
// ---------------------------------------------------------------------------

/// Every token implementation whose implied ceiling admits the committed cap
/// really runs a full [`MAX_BATCH_SIZE`] batch inside every enforced limit.
///
/// The `Bloated` profile is the deliberate exception: it cannot fit, and it is
/// handled explicitly by `an_over_budget_token_is_handled_explicitly`. Its
/// measured over-budget figure is asserted here too, so the exclusion is
/// evidence-backed rather than a skipped case.
#[test]
fn a_full_cap_batch_fits_on_every_custom_token() {
    let mut fitting = 0;

    for profile in Profile::ALL {
        let measured = calibrate(profile);
        measured.report();

        // Whatever the profile, a stream must cost at least the token's own
        // transfer event, and the token must emit at least its nominal payload.
        assert!(
            measured.per_stream_event_bytes() >= measured.per_transfer_event_bytes,
            "{}: per-stream event cost {} below the measured transfer cost {}",
            profile.label(),
            measured.per_stream_event_bytes(),
            measured.per_transfer_event_bytes,
        );
        assert!(
            measured.per_transfer_event_bytes >= profile.nominal_event_payload(),
            "{}: measured {} event bytes below the {} byte payload it emits",
            profile.label(),
            measured.per_transfer_event_bytes,
            profile.nominal_event_payload(),
        );

        if measured.event_ceiling() >= MAX_BATCH_SIZE {
            assert!(
                measured.fits_at_cap(),
                "{}: a full-cap batch must sit inside every enforced limit — {:?}",
                profile.label(),
                measured.at_cap,
            );
            fitting += 1;
        } else {
            assert_eq!(
                profile,
                Profile::Bloated,
                "{}: a profile whose ceiling is below the cap is only expected for \
                 the deliberately over-budget token",
                profile.label(),
            );
            assert!(
                measured.at_cap.event_bytes > EVENT_BYTES_LIMIT,
                "{}: ceiling {} is below the cap, so the measured full-cap batch \
                 must genuinely exceed the {EVENT_BYTES_LIMIT} byte event budget \
                 (measured {})",
                profile.label(),
                measured.event_ceiling(),
                measured.at_cap.event_bytes,
            );
        }
    }

    assert!(
        fitting >= 3,
        "the calibration must cover at least three implementations that fit at \
         the cap; only {fitting} did",
    );
}

// ---------------------------------------------------------------------------
// 3. Which budget binds
// ---------------------------------------------------------------------------

/// For the heavier tokens the event budget is exhausted before the entry
/// footprint budget, which is the premise of the whole calibration. Also
/// asserts the per-transfer cost is strictly increasing across the profiles, so
/// the four implementations really are of differing cost.
#[test]
fn heavier_tokens_spend_the_event_budget_before_the_footprint_budget() {
    let measurements: std::vec::Vec<ProfileCost> = Profile::ALL
        .iter()
        .map(|&profile| calibrate(profile))
        .collect();

    for measured in &measurements {
        measured.report();

        // Every profile stays inside the entry footprint at the cap; the
        // question is only which budget runs out first.
        assert!(
            measured.at_cap.footprint <= LEDGER_ENTRY_LIMIT,
            "{}: footprint {} exceeds {LEDGER_ENTRY_LIMIT} at the cap",
            measured.profile.label(),
            measured.at_cap.footprint,
        );
    }

    for pair in measurements.windows(2) {
        let (lighter, heavier) = (pair[0], pair[1]);
        assert!(
            heavier.per_transfer_event_bytes > lighter.per_transfer_event_bytes,
            "{} must be a costlier token than {}: {} vs {} event bytes per transfer",
            heavier.profile.label(),
            lighter.profile.label(),
            heavier.per_transfer_event_bytes,
            lighter.per_transfer_event_bytes,
        );
        assert!(
            heavier.per_stream_event_bytes() > lighter.per_stream_event_bytes(),
            "{} must cost more event bytes per stream than {}",
            heavier.profile.label(),
            lighter.profile.label(),
        );
    }

    for measured in &measurements {
        if !matches!(measured.profile, Profile::Chatty | Profile::Bloated) {
            continue;
        }

        let event_ceiling = measured.event_ceiling();
        let footprint_ceiling = measured.footprint_ceiling();
        assert!(
            event_ceiling < footprint_ceiling,
            "{}: the event budget must bind before the footprint budget \
             (event ceiling {event_ceiling} vs footprint ceiling {footprint_ceiling})",
            measured.profile.label(),
        );

        // The same statement in comparable budget units: fraction of the event
        // budget used exceeds fraction of the footprint budget used.
        let event_used = measured.at_cap.event_bytes as u64 * LEDGER_ENTRY_LIMIT as u64;
        let footprint_used = measured.at_cap.footprint as u64 * EVENT_BYTES_LIMIT as u64;
        assert!(
            event_used > footprint_used,
            "{}: a full-cap batch spends a larger share of the event budget than \
             of the footprint budget",
            measured.profile.label(),
        );
    }

    // The over-budget profile proves the point in the limit: events exhausted,
    // footprint nowhere near its ceiling.
    let bloated = measurements[3];
    assert_eq!(bloated.profile, Profile::Bloated);
    assert!(
        bloated.at_cap.event_bytes > EVENT_BYTES_LIMIT,
        "bloated: full-cap event bytes {} must exceed {EVENT_BYTES_LIMIT}",
        bloated.at_cap.event_bytes,
    );
    assert!(
        bloated.at_cap.footprint < LEDGER_ENTRY_LIMIT,
        "bloated: footprint {} should stay well inside {LEDGER_ENTRY_LIMIT}",
        bloated.at_cap.footprint,
    );
}

// ---------------------------------------------------------------------------
// 4. The ceiling is token-dependent
// ---------------------------------------------------------------------------

/// The implied event ceiling is a property of the token implementation, not a
/// constant: it falls strictly as the token's transfer event grows, and the
/// committed cap sits inside the ceiling of every profile that can fit.
#[test]
fn the_implied_event_ceiling_differs_per_token_implementation() {
    let measurements: std::vec::Vec<ProfileCost> = Profile::ALL
        .iter()
        .map(|&profile| calibrate(profile))
        .collect();

    let ceilings: std::vec::Vec<u32> = measurements.iter().map(|m| m.event_ceiling()).collect();

    std::println!("implied per-token event ceilings: {ceilings:?}");

    for pair in ceilings.windows(2) {
        assert!(
            pair[1] < pair[0],
            "implied ceilings must fall as the token event grows: {ceilings:?}",
        );
    }

    for i in 0..ceilings.len() {
        for j in (i + 1)..ceilings.len() {
            assert_ne!(
                ceilings[i], ceilings[j],
                "every token implementation must imply a different ceiling: {ceilings:?}",
            );
        }
    }

    for (measured, ceiling) in measurements.iter().zip(ceilings.iter()) {
        if measured.fits_at_cap() {
            assert!(
                *ceiling >= MAX_BATCH_SIZE,
                "{}: ceiling {ceiling} must admit the committed cap {MAX_BATCH_SIZE}",
                measured.profile.label(),
            );
        } else {
            assert!(
                *ceiling < MAX_BATCH_SIZE,
                "{}: ceiling {ceiling} must be below the cap that does not fit",
                measured.profile.label(),
            );
        }
    }

    // Concretely: the baseline is comfortably above the cap, and the heaviest
    // token is comfortably below it.
    assert!(ceilings[1] > MAX_BATCH_SIZE, "SAC must clear the cap");
    assert!(
        ceilings[3] < MAX_BATCH_SIZE,
        "bloated must not clear the cap"
    );
}

// ---------------------------------------------------------------------------
// 5. An over-budget token
// ---------------------------------------------------------------------------

/// A token whose transfer event is heavy enough that a full-cap batch cannot
/// fit is handled explicitly, in both halves of the story:
///
/// * anything **above** the cap is rejected with `BatchTooLarge`
///   (discriminant 19) *before the token is touched* — no transfer, no event,
///   no balance movement; and
/// * the **derived** ceiling is a real, smaller chunk size that does fit inside
///   every enforced limit, so the token remains usable in chunks.
#[test]
fn an_over_budget_token_is_handled_explicitly() {
    // -- Measurement: the full-cap batch overruns the event budget. ---------
    let bloated = calibrate(Profile::Bloated);
    bloated.report();

    let per_stream = bloated.per_stream_event_bytes();
    let ceiling = bloated.event_ceiling();

    assert!(
        bloated.at_cap.event_bytes > EVENT_BYTES_LIMIT,
        "a full-cap batch on the bloated token must exceed the event budget: \
         measured {} vs limit {EVENT_BYTES_LIMIT}",
        bloated.at_cap.event_bytes,
    );
    assert!(
        (1..MAX_BATCH_SIZE).contains(&ceiling),
        "the derived ceiling {ceiling} must be a real, smaller chunk size \
         (per-stream cost {per_stream} event bytes)",
    );
    assert!(
        (ceiling + 1) * per_stream > EVENT_BYTES_LIMIT,
        "the derived ceiling {ceiling} must be tight: one more stream would \
         exceed the event budget",
    );

    // -- Half 1: above the cap is rejected before the token is touched. -----
    let h = Harness::new();
    let token = register_token(&h, Profile::Bloated);
    let ids = create_streams(&h, &token, MAX_BATCH_SIZE + 1);
    h.advance(ELAPSED);

    let err = h
        .client
        .try_batch_withdraw(&h.recipient, &h.ids(&ids))
        .unwrap_err()
        .unwrap();

    assert_eq!(err, Error::BatchTooLarge);
    assert_eq!(Error::BatchTooLarge as u32, 19, "frozen ABI discriminant");
    assert_eq!(
        balance_of(&h, &token, &h.recipient),
        0,
        "no payout may move when the batch is rejected",
    );
    assert_eq!(
        h.env.events().all().events().len(),
        0,
        "a rejected batch must not touch the token, so it emits nothing",
    );
    for id in &ids {
        assert_eq!(h.get(*id).withdrawn, 0, "stream {id} must be untouched");
    }

    // -- Half 2: the derived chunk size really fits. ------------------------
    let h = Harness::new();
    let token = register_token(&h, Profile::Bloated);
    let ids = create_streams(&h, &token, MAX_BATCH_SIZE);
    h.advance(ELAPSED);

    let chunk = ceiling as usize;
    let paid = h.client.batch_withdraw(&h.recipient, &h.ids(&ids[..chunk]));
    let measured = cost_of(&h);
    std::println!(
        "bloated chunk of {chunk} streams: events={}/{} footprint={}/{} \
         writes={}/{} mem={}/{} instructions={}",
        measured.event_bytes,
        EVENT_BYTES_LIMIT,
        measured.footprint,
        LEDGER_ENTRY_LIMIT,
        measured.writes,
        WRITE_ENTRY_LIMIT,
        measured.memory,
        LEDGER_ENTRY_LIMIT,
        measured.instructions,
    );

    assert_eq!(
        paid,
        chunk as i128 * 30 * ONE,
        "the chunk must pay every stream it names",
    );
    assert!(
        measured.event_bytes <= EVENT_BYTES_LIMIT,
        "chunk of {chunk} must fit the event budget: measured {} vs {EVENT_BYTES_LIMIT}",
        measured.event_bytes,
    );
    assert!(
        measured.footprint <= LEDGER_ENTRY_LIMIT,
        "chunk of {chunk} must fit the footprint: measured {} vs {LEDGER_ENTRY_LIMIT}",
        measured.footprint,
    );
    assert!(
        measured.writes <= WRITE_ENTRY_LIMIT,
        "chunk of {chunk} must fit the write budget: measured {} vs {WRITE_ENTRY_LIMIT}",
        measured.writes,
    );
    assert!(
        measured.memory <= LEDGER_ENTRY_LIMIT,
        "chunk of {chunk} must fit the memory budget: measured {} vs {LEDGER_ENTRY_LIMIT}",
        measured.memory,
    );
    assert!(
        measured.instructions <= INSTRUCTION_LIMIT,
        "chunk of {chunk} must fit the instruction budget: measured {} vs {INSTRUCTION_LIMIT}",
        measured.instructions,
    );
    assert_eq!(
        balance_of(&h, &token, &h.recipient),
        chunk as i128 * 30 * ONE,
        "the recipient must receive exactly the chunk's payouts",
    );
}

// ---------------------------------------------------------------------------
// 6. One past the cap, for every token
// ---------------------------------------------------------------------------

/// `MAX_BATCH_SIZE + 1` is rejected with `BatchTooLarge` (discriminant 19) for
/// **every** token implementation — the cap is a structural check on the id
/// vector, independent of which token a stream uses. Nothing is paid, no event
/// is emitted, and no stream is marked drawn.
#[test]
fn one_past_the_cap_is_rejected_for_every_token_implementation() {
    for profile in Profile::ALL {
        let h = Harness::new();
        let token = register_token(&h, profile);
        let ids = create_streams(&h, &token, MAX_BATCH_SIZE + 1);
        h.advance(ELAPSED);

        let err = h
            .client
            .try_batch_withdraw(&h.recipient, &h.ids(&ids))
            .unwrap_err()
            .unwrap();

        assert_eq!(
            err,
            Error::BatchTooLarge,
            "{}: one past the cap must be rejected",
            profile.label(),
        );
        assert_eq!(Error::BatchTooLarge as u32, 19, "frozen ABI discriminant");
        assert_eq!(
            balance_of(&h, &token, &h.recipient),
            0,
            "{}: nothing may be paid out",
            profile.label(),
        );
        assert_eq!(
            h.env.events().all().events().len(),
            0,
            "{}: the rejection precedes any token call, so nothing is emitted",
            profile.label(),
        );
        for id in &ids {
            assert_eq!(
                h.get(*id).withdrawn,
                0,
                "{}: stream {id} must be untouched",
                profile.label(),
            );
        }
    }
}
