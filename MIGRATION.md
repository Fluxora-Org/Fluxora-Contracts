# Migration: `main` → `v1-rewrite`

Deletion audit for the v1 rewrite. Two questions, answered in order:

1. What did the disabled tests cover, and is any of it now *silently* missing
   rather than deliberately dropped?
2. Do `Fluxora-Backend` or `Fluxora-Frontend` call into the deleted surface?

**Short answers.** (1) Nothing is silently missing — but the premise needs
correcting first: `main` does not compile, so none of its tests ran, disabled or
otherwise. (2) The backend is unaffected. **The frontend breaks completely**:
all four of its contract calls fail against v1.

Audit performed 2026-08-12 against `main` @ `75b15ca`, `Fluxora-Backend`
`origin/main` @ `59d3538`, `Fluxora-Frontend` `origin/main` @ `86795ee`.

---

## 0. Correction to an earlier claim

I previously reported "~30 tests disabled via `test = false`". That was wrong.
The accurate figures from `main:contracts/stream/Cargo.toml`:

| | count |
|---|---|
| `[[test]]` blocks total | 41 |
| `test = false` (disabled) | **9** |
| `test = true` (explicitly enabled) | 28 |
| `required-features` gated | 7 |
| test files on disk across all contracts | 83 |

So 9 disabled, not 30. The larger number came from miscounting the block of
`[[test]]` entries added under the "known-broken" comment headers, most of which
were left *enabled*. The stage-1 commit message has been corrected accordingly.

---

## 1. `main` does not compile

Before asking what the tests covered, it is worth establishing that they did not
run.

```
$ cargo build -p fluxora_stream        # on main @ 75b15ca, toolchain 1.94.1 as pinned
error[E0425]: cannot find type `CliffStatus` in module `accrual`
    --> contracts/stream/src/lib.rs:4696:74
error[E0425]: cannot find function `cliff_status` in module `accrual`
    --> contracts/stream/src/lib.rs:4705:21
error: could not compile `fluxora_stream` (lib) due to 2 previous errors
```

This is the **library**, not a test — the contract itself does not build.
`cargo test --workspace` additionally fails to compile
`fluxora_governance`'s `signer_index_proptest` (16 errors, including calls to
`try_add_signer`/`try_remove_signer` methods that no longer exist).

How it happened: PR #1508 (`30f5ed7`, cliff close-time skew status) added
`get_cliff_status` to `lib.rs` and `cliff_status` to `accrual.rs` in the same
change. That commit is an ancestor of `main`, but `main`'s `accrual.rs` contains
no `cliff_status` — a subsequent merge (#1506/#1507) kept the caller and dropped
the callee. `main` has been broken since 2026-07-30.

**Consequence for this audit.** "Did the rewrite lose test coverage?" is not
answerable by comparing pass counts, because the old suite's pass count at HEAD
is zero — nothing compiles, so nothing executes. The comparison below is
therefore about *behaviour covered by test source*, not about behaviour that was
actually being verified in CI.

This also means the previous CI was not gating anything meaningful on `main`.

---

## 2. The nine disabled tests

For each: what it covered, and where that behaviour stands in v1.

| test file | tests | covered | v1 status |
|---|---|---|---|
| `chaos.rs` | 2 | permutations of withdraw/cancel/update_rate/pause/resume against one stream; post-conditions hold regardless of order | **Covered, more strongly.** `test::invariants` drives randomized operation sequences and re-checks every invariant after *every* operation, seeded deterministically for replay. |
| `cliff_only_variant.rs` | 14 | a `CliffOnly` stream *kind* interacting with `keeper_cancel`, `bulk_cancel_streams`, `cancel_stream` | **Split.** Cliff behaviour is covered by `test::cliff` (7 tests) plus a `cliff_gates_but_does_not_delay` property. There is no `StreamKind` enum in v1 — a cliff is a timestamp on every stream, and `cliff_time == end_time` expresses the lump-sum case. `keeper_cancel` and `bulk_cancel_streams` are **deliberately dropped** (see §3). |
| `create_stream_relative.rs` | 21 | `create_stream_relative(start_delay, duration, …)` and `create_streams_relative` batch creation | **Deliberately dropped.** Relative→absolute time conversion is a client concern; putting it on chain duplicates validation and widens the audit surface for no protocol benefit. Batch creation is separately dropped (§3). Belongs in `fluxora-sdk`. |
| `id_monotonicity_upgrade.rs` | 2 | id counter stays monotonic across a contract upgrade | **Partly covered, remainder deliberately dropped.** Monotonicity itself is covered by `stream_ids_are_monotonic_and_never_reused` and `stream_ids_never_collide_after_a_restore`. The upgrade dimension is moot: v1 has no upgrade path (§6 non-goal). |
| `id_reservation.rs` | 33 | `reserve_stream_ids`, `get_id_reservation`, `release_id_reservation`, `reclaim_expired_id_reservation`, reservation/counter-gap semantics | **Deliberately dropped.** Pre-allocating ids is an off-chain coordination feature; it adds four entrypoints and a second id allocator to the audit surface. Not in the spec's function surface (§2.7). |
| `withdrawal_frequency.rs` | 32 | minimum interval between withdrawals, lookback windows, per-stream rate limiting incl. inside `batch_withdraw`, delegated (signed-message) withdrawal | **Deliberately dropped — the largest genuine behaviour removal.** See §3. |
| `governance_proptest.rs` | 13 | below-quorum, timelock and signer-set invariants of `FluxoraGovernance` | **Deliberately dropped** with the whole governance contract (§6: no admin key, no upgradeability). |
| `governance_integration.rs` | 49 | governance propose/approve/execute lifecycle end to end | **Deliberately dropped**, as above. |
| `governance_ttl.rs` | 11 | TTL/archival of `Proposal(id)` and `QuorumReachedAt(id)` entries | **Deliberately dropped** with governance. The *technique* — on-write and on-read TTL bumping near the archival threshold — is carried forward and generalised in `storage.rs` and `test::ttl`. |

**Nothing in this table is silently missing.** Every item is either covered at
least as well in v1, or dropped for a reason traceable to a v1 non-goal.

---

## 3. Behaviour deliberately removed

The old contract set exposed **145 entrypoints** (100 stream, 16 factory, 29
governance). v1 exposes **16**. Grouped by why:

**Contradicts §6 (no admin, no upgradeability, no fees, no global pause)**
`init`, `set_admin`, `upgrade`, `version`, `pause_protocol`, `resume_protocol`,
`global_resume`, `set_global_emergency_paused`, `get_global_emergency_paused`,
`set_contract_paused`, `is_paused`, `cancel_stream_as_admin`,
`pause_stream_as_admin`, `resume_stream_as_admin`, `bulk_resume_streams_as_admin`,
`set_stream_decommissioned`, `sweep_excess`, `get_protocol_fees_accrued`,
`get_keeper_fee_split`, `set_max_rate_per_second`, plus the entire `factory`
(16) and `governance` (29) contracts.

**Contradicts §2.3 (no on-chain stream discovery)**
`get_recipient_streams`, `get_recipient_streams_paginated`,
`get_recipient_stream_count`, `get_streams_by_id_range`, `get_sender_portfolio_health`,
`get_paused_stream_count`, `get_total_liabilities`,
`get_factory_streams_paginated`. These are exactly the per-user index the spec
warns kills these projects — the old contract had it. `test::resource_limits::cost_is_independent_of_how_many_streams_exist`
is the v1 guarantee that replaces them.

**Out of v1 scope, deferred to the SDK or a later version**
`create_stream_relative`, `create_streams_relative`, `create_streams`,
`create_streams_partial`, `reserve_stream_ids` (+3 reservation fns),
`clone_stream`, `create_stream_from_template` (+2 template fns),
`create_pooled_stream`, `withdraw_from_pool`, `create_stream_offer` (+3 offer
fns), `set_auto_claim` (+3 auto-claim fns), `set_auto_renew`, `renew_stream`,
`delegate_recipient_share`, `witnessed_cancel_stream`,
`transfer_claim_ownership`, `get_stream_metadata`, `get_stream_memo`.

**Contradicts the immutability guarantee (§2.2)**
`update_rate`, `update_rate_per_second`, `decrease_rate_per_second`,
`extend_stream_end_time`, `shorten_stream_end_time`. A stream whose rate or end
date the sender can move is not a guarantee the recipient can rely on. v1's only
schedule mutation is `top_up`, which extends duration at a fixed rate and can
never accelerate or dilute an existing schedule.

**Three removals worth calling out individually**, because they are defensible
choices rather than obvious cleanups:

1. **Withdrawal rate limiting** (`withdrawal_frequency.rs`, `create_stream_with_lookback`,
   `set_lookback_window`, `get_lookback_window`). The old contract enforced a
   minimum one-ledger interval between withdrawals and an optional lookback
   window capping each claim. v1 has neither: a recipient may withdraw every
   ledger if they wish. The protocol is not harmed — they pay their own fees and
   conservation still holds exactly — but senders who wanted claim-size caps
   have lost that. If it is wanted back, it belongs behind a separate policy
   contract wrapping `withdraw`, not in the core.

2. **Delegated withdrawal / cancellation** (`delegated_withdraw`,
   `delegated_cancel`, `get_delegated_nonce`, `get_delegated_cancel_nonce`,
   `batch_withdraw_to`, `withdraw_to`). A third party could act for the
   recipient with a signed message, and payouts could be routed to an address
   other than the recipient. v1 pays the recipient and only the recipient.
   Smart accounts (`__check_auth`) cover the legitimate delegation cases without
   a bespoke nonce-and-signature scheme in the contract.

3. **Keeper cancellation** (`keeper_cancel`, `bulk_cancel_streams`,
   `close_completed_stream`, `close_cancelled_stream`). A keeper could cancel or
   close streams, earning a fee split. In v1 the only permissionless keeper
   action is `extend_stream_ttl`, which can only *pay* rent — it cannot move
   funds or change stream state. That is a deliberate reduction in what an
   unauthenticated caller can do.

---

## 4. Renames — the part most likely to bite

Functions that still exist but under different names or signatures. These break
callers **silently at the ABI boundary** (wrong function name → invocation
failure), which is worse than a deletion of something nobody called.

| old | v1 | change |
|---|---|---|
| `cancel_stream(sender, id)` | `cancel(id)` | renamed; `sender` dropped (read from the stream) |
| `pause_stream(sender, id)` | `pause(id)` | renamed; `sender` dropped |
| `resume_stream(sender, id)` | `resume(id)` | renamed; `sender` dropped |
| `top_up_stream(...)` | `top_up(id, amount)` | renamed; semantics changed (extends duration, never the rate) |
| `update_recipient` / `accept_recipient_update` | `transfer_recipient(id, new)` | renamed; two-step propose/accept collapsed to one step, gated by the new immutable `transferable` flag |
| `get_stream_state` | `get_stream(id)` | renamed; `Stream` struct reshaped |
| `get_stream_count` | `stream_count()` | renamed |
| `get_withdrawable` | `withdrawable_of(id)` | renamed |
| `calculate_accrued` | `vested_of(id)` | renamed |
| `create_stream(sender, recipient, amount, start, end, cliff)` | `create_stream(sender, recipient, **token**, deposit, start, end, cliff, **cancellable**, **pausable**, **transferable**)` | 6 args → 10 |
| `withdraw(recipient, id, amount)` | `withdraw(id, amount: Option<i128>)` | 3 args → 2; `recipient` dropped; `None` means "withdraw max" |

Two structural changes behind those signatures:

* **The token is now per stream.** The old contract had a single configured
  token set at `init` and read via `get_config`. v1 has no `init` and no config
  entry; every stream names its own SEP-41 token. Callers must supply it.
* **Amounts are `i128`, not `u64`.** The SEP-41 interface uses `i128`; the
  frontend currently encodes amounts with `encodeU64`.

---

## 5. Downstream impact

### Fluxora-Backend — **not affected**

* **Zero** references to `factory`, `governance`, `timelock` or `proposal`
  anywhere in `*.ts` / `*.sql` / `*.yaml`. Dropping both contracts is safe.
* `src/config/stellarContracts.ts` pins **one** contract address per network
  under a single `contract` kind (plus a `token`). It has no factory or
  governance address, so it already assumes the one-contract model v1 has.
* The indexer is **event-schema-generic**. `ContractEventRecord` is
  `{ eventId, topic: string, ledger, … }` — it ingests, de-duplicates, handles
  forks and replays, but *nothing decodes Fluxora event payloads*. A repo-wide
  search for `stream_created`, `topics[0]` or `scValToNative` returns no files.
  The event renames and reshapes therefore break nothing that exists today.

Two things to carry into stage 5 rather than bugs to fix now:

* **`streams.status` CHECK constraint** allows `('active','paused','completed','cancelled')`.
  v1 emits `Active | Paused | Cancelled | Depleted`. `Depleted` needs mapping to
  `completed`, or the constraint needs widening. Note `Cancelled` is sticky in
  v1 — a cancelled stream drained to zero stays `Cancelled`, it does not become
  `Depleted`.
* **`streams.rate_per_second`** is a stored column. v1 stores no rate; it is
  derived as `deposited / duration` and changes meaning after a `top_up`
  (deposit and duration both grow, rate is held constant). The projection should
  compute it rather than expect it in an event.

The `streams` table is currently populated through the REST API
(`src/routes/streams.ts` → `streamRepository`), not from chain events. The
indexer→projection path is unbuilt, which is precisely stage 5's job.

### Fluxora-Frontend — **breaks completely**

`src/lib/stellar/tx.ts` is the only file that invokes the contract. All four
calls fail against v1:

| call site | invokes | fails because |
|---|---|---|
| `createStream()` :375 | `create_stream` | 6 args vs 10 — no `token`, no capability flags; amount encoded as `u64`, v1 wants `i128` |
| `withdraw()` :395 | `withdraw` | 3 args vs 2 — passes `recipient`, which v1 reads from the stream; amount is `u64` and not `Option` |
| `pauseStream()` :412 | `pause_stream` | function does not exist; renamed to `pause`, and `sender` is dropped |
| `cancelStream()` :429 | `cancel_stream` | function does not exist; renamed to `cancel`, and `sender` is dropped |

`docs/soroban-contract-abi.md` documents the same four old signatures and needs
rewriting. No factory or governance references anywhere in the frontend.

**This is contained.** One file, four functions, plus one doc. Because §6 stage 6
puts the reference UI last and stage 5 introduces `fluxora-sdk`, the right fix is
to have the frontend consume the SDK rather than hand-roll `nativeToScVal`
encoding — which also removes the `u64`/`i128` class of bug permanently.

---

## 6. Actions

| # | action | stage |
|---|---|---|
| 1 | Publish the v1 contract ID and generated bindings so downstream repos have something to target | 4 |
| 2 | Widen or map `streams.status` for `Depleted`; make `rate_per_second` derived | 5 |
| 3 | Build the indexer→`streams` projection against v1's event schemas (codegen from the deployed spec, do not hand-roll topic parsers) | 5 |
| 4 | Replace `src/lib/stellar/tx.ts` with `fluxora-sdk` calls; rewrite `docs/soroban-contract-abi.md` | 6 |
| 5 | Reinstate `delegated_withdraw` only — see the ruling below | v1.1, post-audit |

Nothing here blocks stage 4.

---

## 7. Rulings on the three judgement calls

Settled 2026-08-12. §3 lists these as defensible choices rather than obvious
cleanups; this is the decision on each and the reasoning that produced it.

### Delegated withdrawal — **reinstate, but as v1.1 after audit**

Scope: `delegated_withdraw` only. `delegated_cancel`, `withdraw_to` and
`batch_withdraw_to` stay out — routing a payout to an address that is not the
recipient is a separate and larger risk than letting a relayer pay the gas.

Reinstating it is justified: gasless recipient UX and keeper-driven withdrawal
are real, and they are load-bearing for the smart-account composability story.
But the cost is the largest of the three. It is a *second authorization system*
living beside `require_auth` — bespoke nonce storage, deadline handling, message
construction and ed25519 verification — and a replay window, a malleable-message
bug or a nonce-reuse bug in that code is a direct fund-loss path. An auditor has
to review it as a novel scheme rather than as standard Soroban auth.

Note that smart accounts already cover much of the ground: a recipient whose
address is a contract with `__check_auth` can accept a relayer-submitted signed
intent without the core contract knowing anything about it, and that path is
already tested. The genuine gap is gasless UX for a recipient who is a plain
keypair.

So it lands in v1.1 with its own threat model and its own audit pass, not folded
into the frozen v1 ABI. See [docs/ABI.md](docs/ABI.md) — adding an entry point
means a new deployment at a new address, so this is a deliberate v2 boundary
rather than something to squeeze in.

### Withdrawal rate limiting — **stays out**

The old behaviour enforced a minimum one-ledger interval between withdrawals
plus an optional lookback window capping each claim. Keeping it would cost a
`last_withdraw_ledger` field on every stream, a per-stream lookback setting, and
three more entry points — but the disqualifying cost is behavioural: it makes
`withdraw` able to **fail for a recipient who is genuinely owed money**, which
is a new denial vector in a primitive whose entire promise is that earned funds
are always claimable.

The stated rationale was preventing excessive ledger I/O, but the recipient pays
their own transaction fees, so the protocol is not the party being harmed.
Conservation holds exactly regardless of withdrawal frequency, and a single
withdraw measures at 13/400 footprint entries. There is no protocol-level
problem being solved. The one real use case — a sender capping how much can be
claimed per period — is policy, and policy belongs in a contract wrapping
`withdraw`.

### Keeper cancellation — **stays out**

What it actually did: `keeper_cancel(stream_id, keeper)` was permissionless once
a stream was at least a 7-day grace period past its `end_time`. It force-settled
an abandoned stream — pushed the recipient's accrued balance to them, computed
the sender's refund of the unstreamed portion, took `KEEPER_FEE_BPS` of *that
refund* as the keeper's fee, and sent the remainder to the sender. The purpose
was stopping unclaimed deposits sitting in storage indefinitely.

Two reasons it stays out. First, it is the only path in the contract where an
unauthenticated third party moves other people's money and pays itself from the
proceeds — a categorically different risk class from `extend_stream_ttl`, where
the caller can only spend their *own* funds on rent.

Second, and decisively, the economics are backwards. The fee is a cut of the
sender's unstreamed remainder, and past `end_time` that remainder is zero by
definition. So on a normally-completed stream the keeper fee is zero and nobody
runs it; it only pays out on cancelled-early or dust-remainder streams. The
incentive is both narrow and adversarially shaped.

The problem it solves is also not real in v1: an unwithdrawn stream costs the
contract nothing, TTL is handled by the permissionless rent path, and the
recipient's claim never expires.
