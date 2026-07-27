# Stream Template Lifecycle Guide

## Overview
The Fluxora Stream contract provides a **template system** that lets operators pre‑define reusable schedule parameters for payroll‑style streams. A template stores three `u64` values:
- `start_delay` – seconds after the caller’s `create_stream_from_template` invocation before the stream starts.
- `cliff_delay` – additional seconds before tokens become withdrawable.
- `duration` – total length of the stream after the start time.

Using a template reduces calldata size dramatically (three `u64` values vs the full schedule payload) and ensures consistent schedule semantics across multiple streams.

## Entry‑points
| Entry‑point | Auth / Caller | Description | Errors |
|------------|---------------|-------------|--------|
| `register_stream_template` | `owner.require_auth()` – the address that will own the template | Creates a new template and returns its `template_id`. | `TemplateLimitExceeded` (per‑owner or global cap) |
| `delete_stream_template` | `owner.require_auth()` | Deletes a template owned by the caller. | `TemplateNotFound`, `TemplateUnauthorized` |
| `create_stream_from_template` | `sender.require_auth()` – the stream sender | Instantiates a stream using a previously registered template. Internally calls `create_stream_relative`/`create_stream`. | `TemplateNotFound`, `TemplateUnauthorized` (if sender isn’t the registered owner), `InvalidParams` |
| `get_stream_template` | Public (no auth) | Returns the stored `StreamScheduleTemplate`. | `TemplateNotFound` |

## Auth & Errors Detail
- **Register** – Only the *owner* address supplied as the first arg can register a template. The call records `owner` in storage and enforces `MAX_TEMPLATES_PER_OWNER` (default 50) and the global `MAX_GLOBAL_TEMPLATES` (default 1 000). If a cap is exceeded, `TemplateLimitExceeded` is returned.
- **Delete** – The caller must match the stored `owner`. If the caller is different, the contract returns `TemplateUnauthorized`. Deleting a non‑existent template yields `TemplateNotFound`.
- **Create from template** – The caller (stream **sender**) must be authorized, but the template’s `owner` does **not** need to match the sender. The only validation is that the `template_id` exists; otherwise `TemplateNotFound` is raised.

## Field Mapping from Template to Stream
When `create_stream_from_template(sender, template_id, recipient, deposit, rate, memo, metadata)` is called, the contract performs the following transformations (see `contracts/stream/src/lib.rs::create_stream_from_template`):
1. Retrieve `StreamScheduleTemplate { start_delay, cliff_delay, duration }`.
2. Compute absolute times based on the current ledger timestamp `now`:
   ```rust
   let start_time = now + start_delay;
   let cliff_time = start_time + cliff_delay;
   let end_time   = start_time + duration;
   ```
3. Pass these values to the underlying `create_stream` (or the relative helper) along with the supplied `deposit`, `rate`, `memo`, and `metadata`.
4. The resulting `StreamCreated` event contains the derived times, making the linkage explicit for indexers.

## Calldata / Gas Savings
| Approach | Bytes sent (approx.) |
|----------|---------------------|
| Full schedule (explicit `start_time`, `cliff_time`, `end_time`) | 24 bytes (3 × `u64`) per stream + other params |
| Template + `create_stream_from_template` | **3 bytes** for the `template_id` (a `u64` still) but the schedule fields are omitted from the call payload. The contract reads them from storage, saving ~24 bytes per stream and reducing the transaction’s gas consumption.

> **Why it matters** – When creating hundreds or thousands of payroll streams, the calldata reduction translates into lower fees and less risk of hitting the Soroban per‑transaction byte limit.

## Limits & Safety
- **Per‑owner limit** – `MAX_TEMPLATES_PER_OWNER` (default 50). Once reached, further `register_stream_template` calls by that owner return `TemplateLimitExceeded`.
- **Global limit** – `MAX_GLOBAL_TEMPLATES` (default 1 000). When the total number of stored templates across all owners reaches this cap, any additional registration fails with `TemplateLimitExceeded`.
- **Deletion** – Frees both the owner‑slot and the global slot, allowing new registrations.
- **Security** – Only the owner can delete a template, preventing malicious actors from removing schedules that other services rely on.

## Reference Tests
The contract’s test suite validates the entire lifecycle:
- `template_register_create_delete_happy_path` – registers, creates a stream, then deletes.
- `delete_template_rejects_wrong_owner` – ensures only the owner can delete.
- `per_owner_template_cap_enforced` – demonstrates the per‑owner cap.
- `test_global_template_cap_exceeded` – shows the global cap behavior.

All these tests live in `contracts/stream/tests/stream_templates.rs`.

## Usage Example (Rust client)
```rust
let tid = client.register_stream_template(&owner, &0u64, &0u64, &3600u64);
let stream_id = client.create_stream_from_template(
    &sender,
    &tid,
    &recipient,
    &deposit_amount,
    &rate_per_second,
    &0u64, // optional memo
    &None,
);
```

## Further Reading
- Entry‑point list in `README.md` – see the `register_stream_template` / `delete_stream_template` rows.
- Full contract source in `contracts/stream/src/lib.rs` for the implementation details.
- Security notes in `docs/security.md` for auth handling.

---
*This document is intended for integrators, auditors, and operators building payroll or subscription systems on Fluxora.*
