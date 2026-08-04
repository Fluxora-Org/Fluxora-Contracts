# Event Schema Evolution Policy

> **Status:** Adopted | **Applies to:** `contracts/stream/src/events.rs`, `contracts/stream/src/types.rs`, `contracts/stream/src/lib.rs`  
> **Canonical reference:** [`docs/events.md`](./events.md) for the current schema catalogue  
> **Enforcement:** `tests/event_schema_consistency.rs` (additive-only checks), `tests/event_snapshots_suite.rs` (topic/field immutability)

## Table of contents

1. [Motivation](#1-motivation)
2. [Core rules](#2-core-rules)
3. [Topic symbol permanence](#3-topic-symbol-permanence)
4. [Additive-only field changes](#4-additive-only-field-changes)
5. [Deprecation process](#5-deprecation-process)
6. [Breaking changes (last resort)](#6-breaking-changes-last-resort)
7. [Worked examples](#7-worked-examples)
8. [Version bump checklist](#8-version-bump-checklist)
9. [Security and testing](#9-security-and-testing)
10. [FAQ](#10-faq)

---

## 1. Motivation

The Fluxora stream contract emits structured Soroban events that off-chain
indexers, dashboards, wallets, and treasury tooling consume. Once an event
schema has been deployed to any network (testnet, mainnet) and indexed,
changing its shape — field names, types, order, or topic symbols — silently
breaks every downstream consumer that depends on stable field ordering and
topic-based filtering.

This document codifies the **additive-only backward-compatibility policy** that
all contributors must follow when modifying event payloads. The policy
guarantees that:

- **No existing indexer ever breaks** from a contract upgrade.
- **New fields are always optional** (or carry safe defaults) so old parsers
  continue to decode the struct without error.
- **Topic symbols are permanent** once deployed to any non-local network.
- **Deprecation is signalled explicitly** in the payload itself before any
  field is removed.

### 1.1 Governance scope

This policy covers all event payload structs and topics defined in:

- `contracts/stream/src/events.rs` — Canonical topic definitions and emit helpers.
- `contracts/stream/src/types.rs` — Event payload structs (`#[contracttype]`).
- `contracts/stream/src/lib.rs` — Additional event types defined at the crate root.

It does **not** cover:

- Internal storage events (not indexed off-chain).
- Factory or governance contract events (each has its own separate event
  catalogue; see `docs/events.md` for the factory and governance sections).

### 1.2 Relationship to other policies

| Policy | Relationship |
|--------|-------------|
| [`docs/ABI_STABILITY.md`](./ABI_STABILITY.md) | Event fields are part of the contract ABI; ABI stability implies event-field stability. |
| [`docs/storage.md`](./storage.md) | Storage keys use a separate `DataKey` discriminant policy; events are emission-only and do not share that discriminant space. |
| [`docs/events.md`](./events.md) | Authoritative catalogue of every event, its topic(s), field types, and emission site. Updated in lockstep with this policy. |
| [`CONTRACT_VERSION`](../contracts/stream/src/lib.rs) | Breaking event changes MUST bump this constant (see §6). |
| `tests/event_schema_consistency.rs` | Programmatic cross-check that the code registry matches `docs/events.md`. |
| `tests/event_snapshots_suite.rs` | Snapshot tests that pin exact topic+payload shapes. |

---

## 2. Core rules

### Rule 1 — Topics are permanent

Every topic symbol published via `symbol_short!(...)` in `events.rs` is
**permanently reserved** once the contract has been deployed to any network
other than a local ephemeral instance.

> A topic symbol is the string literal inside `symbol_short!()` such as
> `"created"`, `"withdrew"`, `"cancelled"`, `"rate_upd"`, etc.

**What you may not do:**
- Rename an existing topic symbol.
- Reuse a retired topic symbol for a different event.
- Change the number of topic elements for an existing symbol.

**What you may do:**
- Add a **new** topic symbol for a new event (always additive).
- Annotate an existing topic as **deprecated** in documentation
  (`docs/events.md` and the Rust doc comment on the emit helper).

### Rule 2 — Fields are append-only

New fields may only be added at the **end** of an existing event payload struct.
Existing fields must never change type, name, or relative position.

> Soroban `#[contracttype]` structs are serialised by field declaration order.
> Inserting a field anywhere other than the end shifts every subsequent field's
> ordinal, breaking every deserialiser built against the old layout.

**What you may not do:**
- Insert a new field before or between existing fields.
- Reorder existing fields.
- Change the type of an existing field (e.g. `u64` → `i128`).
- Remove an existing field (see §5 deprecation process).
- Rename an existing field (field *names* are not part of the XDR encoding but
  are part of the documented contract; indexers that parse by name will break).

**What you may do:**
- Append new fields at the end of the struct.
- Add new `Option<T>` fields (preferred) or fields with safe zero-defaults
  (e.g. `u64::MIN`, `false`, `Address::default()`).
- Deprecate an existing field via the process in §5.

### Rule 3 — New fields must have safe defaults

Every new field appended to an existing event struct must be **self-describing**
or carry a **safe zero-value default** so that parsers built before the field
was added continue to function correctly.

| Type | Safe default | Notes |
|------|-------------|-------|
| `Option<T>` | `None` | **Preferred** — explicitly signals absence. |
| `bool` | `false` | Only when `false` is the safe backward-compatible state. |
| `u64`, `u32` | `0` | Only when `0` is semantically neutral (e.g. zero fee, zero balance impact). |
| `i128` | `0` | Same as `u64`. |
| `Address` | ❌ Not safe | Wrap in `Option<Address>` instead. |
| `Bytes`, `String` | ❌ Not safe | Wrap in `Option<Bytes>`, `Option<String>`. |
| `Map<K,V>` | ❌ Not safe | Wrap in `Option<Map<K,V>>`. |

### Rule 4 — Every event has one canonical emit helper

Each event is emitted by exactly one public helper function in `events.rs`.
The emit helper owns the `symbol_short!()` topic definition and the
`env.events().publish(...)` call.

> Rationale: Co-locating the topic symbol with the payload type makes ABI
> review trivial — every topic in the contract is visible by reading only
> `events.rs`. Splitting an event's emission across multiple sites risks
> inconsistency and silent topic drift.

If a new code path needs to emit an existing event, it must call the existing
emit helper — not re-publish the event inline.

### Rule 5 — Topic cardinality (number of elements) is fixed

The number of topic elements for a given topic symbol is part of the event's
identity. For example, `"created"` always has topics `["created", stream_id]`
(2 elements). Adding a third topic element would break parsers that filter by
`topics[1]` to extract the stream ID.

| Topic symbol | Current cardinality | Rationale |
|-------------|-------------------|-----------|
| `"created"` | 2 (`[symbol, stream_id]`) | Stream-level events use `stream_id` as second topic. |
| `"AdminUpd"` | 1 (`[symbol]`) | Contract-level events have no stream. |
| `"renewed"` | 3 (`[symbol, old_id, new_id]`) | Carries both stream IDs for correlation. |
| `"fct_init"` | 1 (`[symbol]`) | Factory init is a one-time event. |

---

## 3. Topic symbol permanence

### 3.1 Lifetime guarantee

Once a topic symbol (e.g. `"created"`, `"withdrew"`, `"rate_upd"`) appears in
a deployed contract's event output, it **must never be reassigned, renamed,
or removed** from the source tree. The symbol is part of the permanent event
namespace of the protocol.

Rationale:

- Indexers use `topics[0]` as the primary event-type discriminator. Changing
  it would cause all historical events of that type to be silently dropped.
- Off-chain analytics pipelines build type registries keyed by topic symbol.
- Wallet UIs filter event streams by topic symbol.

### 3.2 Reserved topic namespace

The following topic symbols are reserved for future use and must not be
assigned to a different event type:

| Reserved symbol | Reserved for |
|----------------|-------------|
| `"migrated"` | Future migration checkpoints. |
| `"tmpl_def"` | Template definition events. |

These symbols are documented in `docs/events.md` as "reserved" and are not
currently emitted by any function.

### 3.3 Adding a new topic symbol

To add a new topic symbol for a new event:

1. Choose a symbol ≤ 9 characters (`symbol_short!` constraint).
2. Follow the naming conventions from existing symbols:
   - Use `snake_case` abbreviations where possible (e.g. `rate_upd`, `end_shrt`, `ac_set`).
   - Avoid symbols that differ only in case (Soroban symbols are case-sensitive).
   - Use meaningful prefixes for related event families (e.g. `ac_set`, `ac_revoke`, `ac_trig` for auto-claim).
3. Add the emit helper to `events.rs` with the new `symbol_short!` call.
4. Document the new event in `docs/events.md`.
5. Add the new event struct to the test registry in `tests/event_schema_consistency.rs`.
6. Add a snapshot test in `tests/event_snapshots_suite.rs` that pins the exact topic+payload.

---

## 4. Additive-only field changes

### 4.1 Adding a field to an existing event

The only permitted mutation to an existing event payload struct is appending
one or more new fields at the end. Follow this checklist:

1. **Choose the type** — Prefer `Option<T>` so old parsers see `None`.
2. **Append the field** — Add the new field after all existing fields in the
   `#[contracttype]` struct definition.
3. **Update the emit helper** — If the new field requires a new parameter, add
   it to the emit helper function signature. The old call sites must compile
   unchanged — this means either:
   - Give the parameter a default (not possible in Rust without a second
     function or builder), **or**
   - Create a new emit helper with a different name and keep the old one
     (preferred for minor additions).
4. **Document the field** — Add it to the struct's doc comment and to
   `docs/events.md`.
5. **Update the test registry** — Add the field to the struct's entry in
   `tests/event_schema_consistency.rs`.
6. **Add snapshot coverage** — Update the snapshot test to include the new
   field in at least one fixture.
7. **Do NOT bump `CONTRACT_VERSION`** for purely additive field additions
   (they are backward-compatible by definition).

> **Exception:** If the new field changes the semantics of an existing field
> for downstream consumers (e.g., in V9, `Withdrawal.amount` now reports the
> *net* amount for `delegated_withdraw` calls instead of the gross), then
> `CONTRACT_VERSION` MUST be bumped. See §6.3 for the full policy.

### 4.2 Upgrade-aware emit helpers

When a field is added that only applies to streams created at or after a
certain `CONTRACT_VERSION`, the emit helper should document this:

```rust
/// Emit the `created` event for a new stream.
///
/// ## Versioning notes
///
/// - Since V1: `stream_id`, `sender`, `recipient`, `deposit_amount`,
///   `rate_per_second`, `start_time`, `cliff_time`, `end_time`,
///   `withdraw_dust_threshold`
/// - Since V2: `memo: Option<Bytes>` (appended).
/// - Since V3: `metadata: Option<Map<Bytes,Bytes>>` (appended).
pub(crate) fn emit_stream_created(env: &Env, stream_id: u64, payload: StreamCreated) {
    // ...
}
```

---

## 5. Deprecation process

When an event field is no longer relevant but cannot be removed without
breaking existing parsers, follow this deprecation process.

### 5.1 Deprecation lifecycle

```
Phase 1: Announce         Phase 2: Signal           Phase 3: Remove
  (docs only)              (in-band marker)           (major version)

      |                         |                          |
      v                         v                          v
  Notice period ─────────► Deprecation ──────────────► Removal
  (≥ 1 version bump)      field set to sentinel       after ≥ N versions
                            value for N versions       of deprecation
```

### 5.2 Phase 1 — Announce (notice period)

Duration: **At least one `CONTRACT_VERSION` bump** (i.e., the field is
announced as deprecated but still populated with real values).

Actions:

1. Update the field's doc comment in the Rust struct to include `@deprecated`
   in the doc comment.
2. Add a `#[deprecated]` Rust attribute if feasible (note: may not be possible
   on `#[contracttype]` struct fields; use doc comment only as fallback).
3. Update `docs/events.md` to mark the field as **DEPRECATED** with the
   version in which deprecation was announced and the planned removal version.
4. Bump `CONTRACT_VERSION` (deprecation is a user-facing change that indexers
   may act on).

### 5.3 Phase 2 — Signal (in-band marker)

Duration: **At least two full `CONTRACT_VERSION` bumps after Phase 1** (unless
an emergency requires faster removal).

Starting at version `V_deprecated + 1`, the field is set to its **sentinel
value** instead of a real value:

| Original type | Sentinel value | Rationale |
|-------------|---------------|-----------|
| `Option<T>` | `None` | Already the safe default. |
| `u64` | `0` | Indexers can check for sentinel. |
| `i128` | `0` | Same. |
| `bool` | `false` | Same. |
| `Address` | `Address::default()` (zero address) | Must switch to `Option<Address>` first. |
| `String` | Empty string `""` | Must switch to `Option<String>` first. |
| `Bytes` | Empty `Bytes` | Must switch to `Option<Bytes>` first. |

Actions:

1. Modify the emit helper to write the sentinel value.
2. Update the struct doc comment to state that the field is now sentinel-only.
3. Update `docs/events.md` to reflect the sentinel behaviour.
4. Add a test that confirms the sentinel is emitted correctly.

### 5.4 Phase 3 — Removal (major version)

After the field has been in sentinel mode for at least two version bumps,
it may be removed from the struct definition entirely.

> **Warning:** Removing a field from a `#[contracttype]` struct is a **binary
> breaking change** — old deserialisers will fail to decode the new struct.
> This should only be done when the protocol has an explicit mechanism for
> retiring old contract instances (e.g. a migration to a new contract ID).

Before removing:

1. Confirm that no indexed data on any live network still references the field.
2. Communicate the removal timeline to all known indexer operators.
3. Update `docs/events.md` to remove the field.
4. Bump `CONTRACT_VERSION`.
5. Remove the field from the Rust struct.
6. Remove the field from the test registry and snapshot tests.

### 5.5 Deprecation metadata payload

When deprecating a field, **do not** embed the deprecation signal in a
separate metadata field. The sentinel value in the field itself is the signal.
This keeps the event flat and parseable without secondary lookups.

---

## 6. Breaking changes (last resort)

A breaking change is any modification to an event payload that violates the
additive-only rules in §2. Breaking changes are strongly discouraged but may
be unavoidable in extreme circumstances (e.g., critical security fix that
requires changing a field type).

### 6.1 When breaking changes are permitted

- **Security vulnerability**: An existing field's type or semantics creates a
  security hole that cannot be fixed additively.
- **Protocol redesign**: A new major protocol version (e.g. V2 where V1 is
  fully deprecated) where old instances are no longer indexed.

### 6.2 Breaking change protocol

If a breaking change is absolutely necessary:

1. **Bump `CONTRACT_VERSION`** (required by the versioning policy in `lib.rs`).
2. **Update `docs/ABI_STABILITY.md`** with the full description of what changed
   and why.
3. **Update `docs/events.md`** to reflect the new schema.
4. **Annotate the old schema** in `docs/events.md` as a historical note with
   the last version it appeared in.
5. **Document the migration path** for indexers (e.g., "parse `amount` as net
   for delegated_withdraw paths from V9 onward; see `docs/events.md` §9").
6. **Add a version-sensitive snapshot test** that verifies the event shape
   differs by contract version.
7. **Communicate** the breaking change to all known downstream consumers at
   least two weeks before deployment to a non-local network.

### 6.3 The V9 precedent: `Withdrawal.amount` semantics change

The V9 release changed the semantics of `Withdrawal.amount` specifically for
the `delegated_withdraw` path — it now reports the **net** amount
(gross − relayer_fee) instead of the gross total. This was handled as follows:

| Step | Action | Location |
|------|--------|----------|
| 1 | `CONTRACT_VERSION` bumped from 8 to 9 | `lib.rs` |
| 2 | `docs/events.md` updated with a `v9 breaking event-payload change` callout | `docs/events.md` § Parsing recommendations |
| 3 | Legacy pre-V9 snapshots preserved as reference | `tests/event_snapshots_suite.rs` |
| 4 | Version-sensitive snapshot test added | `tests/event_snapshots_suite.rs` |

This remains the **only** permitted form of a "semantic" breaking change:
field *names*, *types*, and *order* are unchanged; only the documented
interpretation of an existing field's value changes for a specific code path.

---

## 7. Worked examples

### 7.1 Compliant change: Appending an optional field

**Scenario:** We want to add a `claim_deadline: Option<u64>` field to the
`StreamCreated` event so indexers know when a stream's initial claim expires.

**Current struct** (simplified):
```rust
pub struct StreamCreated {
    pub stream_id: u64,
    pub sender: Address,
    // ... other fields ...
    pub metadata: Option<Map<Bytes, Bytes>>,
}
```

**Compliant diff** ✅ — new field appended at end, safe default (`None`):
```diff
 pub struct StreamCreated {
     pub stream_id: u64,
     pub sender: Address,
     // ... other fields (unchanged) ...
     pub metadata: Option<Map<Bytes, Bytes>>,
+    /// Block number (or Unix timestamp) after which the initial claim
+    /// expires. `None` means no deadline.
+    /// Added in CONTRACT_VERSION 10.
+    pub claim_deadline: Option<u64>,
 }
```

**Why this is safe:** Old parsers that deserialise `StreamCreated` expecting
the old field count will still work because Soroban's XDR deserialisation
ignores trailing fields. New parsers see `None` for streams created before
this field existed (handled by the emit helper populating the default).

### 7.2 Non-compliant change: Inserting a field in the middle

**Scenario:** A developer wants to group all address fields together.

**Non-compliant diff** ❌ — field inserted between existing fields:
```diff
 pub struct StreamCreated {
     pub stream_id: u64,
     pub sender: Address,
+    pub claim_deadline: Option<u64>,   // INSERTED HERE — BREAKING!
     pub recipient: Address,
     // ...
 }
```

**Why this breaks:** Soroban XDR serialises struct fields by ordinal. The
recipient field moves from ordinal 2 to ordinal 3. Every indexer built
against the old layout now reads `recipient` data from the `claim_deadline`
slot and vice versa, producing corrupted data.

### 7.3 Non-compliant change: Changing a field type

**Non-compliant diff** ❌ — type changed:
```diff
 pub struct Withdrawal {
     pub stream_id: u64,
     pub recipient: Address,
-    pub amount: i128,
+    pub amount: u64,   // TYPE CHANGE — BREAKING!
 }
```

**Why this breaks:** The XDR wire format for `i128` and `u64` differs in both
size and encoding. Deserialisers expecting `i128` will read the wrong number
of bytes and fail to parse the rest of the struct.

### 7.4 Non-compliant change: Removing a field without deprecation

**Non-compliant diff** ❌ — field removed:
```diff
 pub struct WithdrawalTo {
     pub stream_id: u64,
     pub recipient: Address,
-    pub destination: Address,   // REMOVED — BREAKING!
     pub amount: i128,
 }
```

**Why this breaks:** All existing parsers expect `destination` at ordinal 2.
They will read `amount` data as `destination` and the first word of the next
struct element (if any) as `amount`.

### 7.5 Compliant change: Adding a new event

**Scenario:** Add a `StreamAudited` event.

**Compliant** ✅:
```rust
// 1. Add topic symbol and emit helper in events.rs.
pub(crate) fn emit_stream_audited(env: &Env, stream_id: u64, payload: StreamAudited) {
    env.events()
        .publish((symbol_short!("audited"), stream_id), payload);
}

// 2. Define the payload struct at the crate root (lib.rs or types.rs).
#[contracttype]
#[derive(Clone, Debug)]
pub struct StreamAudited {
    pub stream_id: u64,
    pub auditor: Address,
    pub passed: bool,
    pub report_hash: Option<BytesN<32>>,
}

// 3. Document in docs/events.md.
// 4. Add to test registry in event_schema_consistency.rs.
// 5. Add snapshot test in event_snapshots_suite.rs.
```

### 7.6 Compliant deprecation: Marking a field as deprecated

**Scenario:** The `memo` field on `StreamCreated` is no longer needed.

**Phase 1 — Announce (V10)**:
```diff
 pub struct StreamCreated {
     pub stream_id: u64,
     pub sender: Address,
     // ... other fields ...
+    /// @deprecated Since V10. Use the `metadata` field instead.
+    /// Will be removed in V12.
     pub memo: Option<Bytes>,
     pub metadata: Option<Map<Bytes, Bytes>>,
 }
```

**Phase 2 — Signal (V11)**:
```diff
 pub(crate) fn emit_stream_created(env: &Env, stream_id: u64, payload: StreamCreated) {
+    // V11+: memo is deprecated and always emitted as None.
+    // Remove the memo field from the struct in V12.
     env.events()
         .publish((symbol_short!("created"), stream_id), payload);
 }
```

**Phase 3 — Removal (V12)**:
```diff
 pub struct StreamCreated {
     pub stream_id: u64,
     pub sender: Address,
     // ... other fields ...
-    pub memo: Option<Bytes>,        // REMOVED
     pub metadata: Option<Map<Bytes, Bytes>>,
 }
```

---

## 8. Version bump checklist

When `CONTRACT_VERSION` is bumped, update the following in addition to the
constants and documents listed in `lib.rs`:

| Artifact | Action |
|----------|--------|
| `lib.rs` — `CONTRACT_VERSION` | Increment. |
| `lib.rs` — `const FROZEN_DISCRIMINANTS_V*` | Add the new version's enumerant if `DataKey` changed. |
| `docs/events.md` | Version header at the top: "Current version: V<N>". |
| `docs/events.md` — version callout | Add a **Version N** subsection if event shapes changed. |
| `docs/event-schema-evolution.md` | Update the version history table (§8.1). |
| `docs/ABI_STABILITY.md` | Update event catalogue reference. |
| `docs/upgrade.md` | Append version-history row. |
| `tests/event_schema_consistency.rs` | Update event registry if structs changed. |
| `tests/event_snapshots_suite.rs` | Add or update snapshot fixtures. |

### 8.1 Version history

| Version | Date | Breaking event change | Summary |
|---------|------|----------------------|---------|
| 1 | — | — | Initial event schema. |
| 3 | — | Yes (`StreamPaused` replaced `Paused(u64)`) | `"paused"` data changed from `StreamEvent::Paused(id)` to `StreamPaused { stream_id, reason }`. |
| 9 | — | Yes (semantic) | `Withdrawal.amount` reports net for `delegated_withdraw`. |
| 9+ | — | No | All future changes are additive-only per this policy. |

---

## 9. Security and testing

### 9.1 Security assumptions

1. **Event data is public.** Soroban events are visible to all network
   participants. No sensitive data should ever be placed in an event payload.
2. **Event emission is not a security control.** Events are informational;
   they cannot be forged by an attacker to alter contract state. However,
   events that leak information about stream health or finances should be
   reviewed for privacy implications.
3. **Deprecation sentinels must be verifiable.** Indexers that switch on
   sentinel values must be able to distinguish "not yet implemented" from
   "deprecated" — the version in which deprecation was announced is the
   discriminator.

### 9.2 Test requirements

Every change to an event schema (additive or otherwise) must include:

| Test | Location | What it verifies |
|------|----------|-----------------|
| Registry-vs-docs consistency | `tests/event_schema_consistency.rs` | Code struct fields match `docs/events.md`. |
| Topic immutability | `tests/event_schema_consistency.rs` (see `test_additive_only_evolution`) | No existing field was removed, reordered, or changed type. |
| Snapshot pinning | `tests/event_snapshots_suite.rs` | Exact topic+payload shape is pinned and does not change without explicit test update. |
| Sentinel emission | `tests/event_schema_consistency.rs` (deprecation tests) | Deprecated fields emit the correct sentinel value. |
| Version-sensitive shape | `tests/event_snapshots_suite.rs` | Event shape differs by contract version when a breaking change is intentional. |

### 9.3 CI enforcement

The following CI gates prevent accidental event-schema drift:

1. **`test_event_schemas_consistent_with_docs`** — Fails if the code registry
   and `docs/events.md` disagree on field names, types, or struct membership.
2. **`test_additive_only_evolution`** — Fails if an existing event struct's
   fields are reordered, removed (without going through the deprecation
   process), or changed in type.
3. **`event_snapshots_suite.rs` tests** — Fails if a snapshot fixture's
   event output differs from the pinned expected output.

---

## 10. FAQ

**Q: What if I need to add a required (non-Option) field?**

A: Don't. A required field with no safe default would cause old parsers to
either panic or produce undefined data. If the field is genuinely always
present, make it `Option<T>` and document that indexers should treat `None`
as "information not available for this stream" rather than a default.

**Q: Can I rename an event struct?**

A: No — the struct name is embedded in Soroban's XDR type metadata and is
used by SDK-generated clients. Renaming breaks those clients. Document the
old name as a deprecated alias instead.

**Q: What about the `StreamEvent` enum variants?**

A: The `StreamEvent` enum (with variants `Paused(u64)`, `Resumed(u64)`,
`StreamCancelled(u64)`, `StreamCompleted(u64)`, `StreamClosed(u64)`) is
part of the event schema. The same additive-only rules apply — new variants
may be appended, existing variants may not be reordered or removed.

> **Historical note:** The `Paused` variant was already replaced by the
> `StreamPaused` struct in V3. The `Paused(u64)` variant is retained in the
> enum for ABI compatibility but is no longer emitted.

**Q: Do snapshot tests need to be updated for additive changes?**

A: Yes. At least one snapshot fixture in `tests/event_snapshots_suite.rs`
must include the new field populated with a non-default value. This prevents
the field from being silently dropped by the serialiser.

**Q: What if two PRs add fields to the same struct concurrently?**

A: The second PR to merge will need to rebase and append its field after the
first PR's field. The CI test `test_event_schemas_consistent_with_docs` will
catch ordering mismatches between the registry and `docs/events.md`.

**Q: How does this policy interact with the `DataKey` storage evolution policy?**

A: The `DataKey` policy (documented in `types.rs` and `docs/storage.md`)
governs storage-key discriminant stability — an entirely separate concern.
Event payloads do not use `DataKey` discriminants and follow the rules in
this document instead. The only overlap is the shared requirement to bump
`CONTRACT_VERSION` for breaking changes.

---

## References

- [Event catalogue (`docs/events.md`)](./events.md) — Current schema for every event.
- [ABI stability policy (`docs/ABI_STABILITY.md`)](./ABI_STABILITY.md) — Broader ABI stability rules.
- [Storage key evolution (`docs/storage.md`)](./storage.md) — `DataKey` discriminant policy.
- [Manifest versioning (`docs/manifest-versioning.md`)](./manifest-versioning.md) — Versioning of the entire contract.
- [Event emission helpers (`contracts/stream/src/events.rs`)](../contracts/stream/src/events.rs) — Canonical emit call sites.
- [Event schema consistency tests (`contracts/stream/tests/event_schema_consistency.rs`)](../contracts/stream/tests/event_schema_consistency.rs) — Automated policy enforcement.
- [Event snapshot tests (`contracts/stream/tests/event_snapshots_suite.rs`)](../contracts/stream/tests/event_snapshots_suite.rs) — Topic+payload pinning tests.
