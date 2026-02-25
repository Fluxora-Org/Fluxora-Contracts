# Security

Notes for auditors and maintainers on security-relevant patterns used in the Fluxora stream contract.

## Checks–Effects–Interactions (CEI)

The contract follows the **Checks-Effects-Interactions** pattern to reduce reentrancy risk. State updates are performed **before** any external token transfers in the functions that move funds.

- **`withdraw`**  
  After all checks (auth, status, withdrawable amount), the contract updates `withdrawn_amount` and, when applicable, sets status to `Completed`, then persists the stream with `save_stream`. Only after that does it call the token contract to transfer tokens to the recipient.

- **`cancel_stream`** and **`cancel_stream_as_admin`**  
  After checks and computing the refund amount, the contract sets `stream.status = Cancelled` and calls `save_stream`. The refund transfer to the sender is performed only after the updated state is saved.

This ordering ensures that if a downstream token contract or hook re-enters the stream contract, the on-chain state (e.g. `withdrawn_amount`, `status`) already reflects the current operation, limiting reentrancy impact. For broader reentrancy mitigation, see [Issue #55](https://github.com/Fluxora-Org/Fluxora-Contracts/issues/55).
