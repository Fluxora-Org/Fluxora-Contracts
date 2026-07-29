# Keeper-cancel conservation property

`contracts/stream/tests/keeper_cancel.rs` contains an integration-level
property test for `keeper_cancel`. It generates linear streams with varying
deposit amounts, rates, durations, and optional prior recipient withdrawals.
Each case advances the ledger beyond `KEEPER_GRACE_PERIOD_SECONDS` and calls
the real contract entry point with the full authentication, storage, token,
and liability bookkeeping paths enabled.

For every successful cancellation, the test observes the actual token balance
deltas and proves:

```text
recipient_amount + sender_refund + keeper_fee
    == deposit_amount - withdrawn_amount_before_cancel
```

It also asserts that `TotalLiabilities` and the contract token balance each
decrease by exactly that outstanding balance. The keeper fee is checked to be
non-negative, bounded by the gross sender refund, and reconciled with the
`KeeperCancelled` event. These checks ensure the three real `push_token`
transfers conserve escrowed funds without relying on the isolated fee-split
helper.

Run the focused suite with:

```bash
cargo test -p fluxora_stream --features testutils --test keeper_cancel
```
