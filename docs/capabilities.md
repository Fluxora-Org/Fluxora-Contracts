# Delegated Capability Tokens

This contract supports short-lived, revocable capability tokens for fine-grained delegation.

## Why capabilities

Role/admin authorization is broad. Capabilities allow an owner to delegate a specific action to a specific holder with bounded scope and expiry.

## Capability structure

Each capability stores:

- `owner`: account that owns the underlying authority
- `holder`: delegate account allowed to use the capability
- `action`: `Claim`, `Release`, or `RefundOnce`
- `amount_limit`: maximum delegated amount (or `1` for `RefundOnce`)
- `remaining_amount`: remaining delegated quota
- `expiry`: unix timestamp after which use is rejected
- `revoked`: explicit revocation flag
- `stream_id`: stream this capability is bound to

## Entry points

- `issue_capability(stream_id, holder, action, amount_limit, expiry) -> capability_id`
- `revoke_capability(capability_id)`
- `get_capability(capability_id) -> Capability`
- `delegated_release(stream_id, capability_id, holder, amount) -> released_amount`
- `delegated_refund(stream_id, capability_id, holder)`

## Security invariants

- Only the true authority can issue:
  - recipient for `Claim`/`Release`
  - sender for `RefundOnce`
- Capability use is holder-bound, stream-bound, expiry-bound, and revocable.
- `Release`/`Claim` cannot be issued above recipient's remaining stream authority.
- `RefundOnce` can only be issued with `amount_limit = 1`.
- Delegated release still pays the stream recipient (never the holder).

## Usage examples

### 1) Delegate release up to 300 tokens for 1 hour

1. Recipient issues:
   - `issue_capability(stream_id, bot, Release, 300, now + 3600)`
2. Bot calls as needed:
   - `delegated_release(stream_id, capability_id, bot, 120)`
   - `delegated_release(stream_id, capability_id, bot, 180)`
3. Capability is exhausted (`remaining_amount = 0`) and auto-marked revoked.

### 2) Delegate one-time refund/cancel

1. Sender issues:
   - `issue_capability(stream_id, ops, RefundOnce, 1, now + 600)`
2. Ops calls:
   - `delegated_refund(stream_id, capability_id, ops)`
3. Capability cannot be reused.

### 3) Emergency revocation

1. Owner calls:
   - `revoke_capability(capability_id)`
2. Any subsequent delegated use fails with `capability revoked`.
