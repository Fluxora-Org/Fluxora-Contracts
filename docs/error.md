# Error Code Reference Table

This table lists all possible errors in the FluxoraStream contract, including both formal `ContractError` variants and runtime errors/panics that can occur. It enables integrators and frontends to handle errors correctly.

| Error Code / Panic Message                       | Description                                                                 | Functions Returning It |
|-------------------------------------------------|-----------------------------------------------------------------------------|----------------------|
| `StreamNotFound`                                | The specified stream does not exist                                          | `pause_stream`, `resume_stream`, `cancel_stream`, `withdraw`, `calculate_accrued`, `get_stream_state`, `cancel_stream_as_admin`, `pause_stream_as_admin`, `resume_stream_as_admin` |
| `deposit_amount must be positive`               | Deposit amount must be greater than zero                                     | `create_stream` |
| `rate_per_second must be positive`              | Stream rate must be greater than zero                                        | `create_stream` |
| `sender and recipient must be different`       | Sender cannot stream to themselves                                           | `create_stream` |
| `start_time must be before end_time`           | Stream start time must be less than end time                                  | `create_stream` |
| `cliff_time must be within [start_time, end_time]` | Vesting cliff must be within the stream duration                          | `create_stream` |
| `deposit_amount must cover total streamable amount (rate * duration)` | Deposit must be sufficient to cover total streaming                          | `create_stream` |
| `already initialised`                           | Contract has already been initialized                                        | `init` |
| `stream must be active to pause`                | Cannot pause a stream unless it is active                                     | `pause_stream`, `pause_stream_as_admin` |
| `stream is already paused`                      | Stream is already paused                                                     | `pause_stream` |
| `stream is active, not paused`                 | Cannot resume a stream that is already active                                 | `resume_stream` |
| `stream is completed`                           | Cannot resume or withdraw from a completed stream                             | `resume_stream`, `withdraw` |
| `stream is cancelled`                           | Cannot resume a cancelled stream                                             | `resume_stream` |
| `stream must be active or paused to cancel`    | Can only cancel active or paused streams                                     | `cancel_stream`, `cancel_stream_as_admin` |
| `stream already completed`                      | Cannot withdraw from a completed stream                                      | `withdraw` |
| `cannot withdraw from paused stream`           | Cannot withdraw while stream is paused                                        | `withdraw` |
| `nothing to withdraw`                            | No withdrawable tokens available                                             | `withdraw` |
| `stream must be active`                         | Admin cannot pause a stream that is not active                                 | `pause_stream_as_admin` |
| `stream is not paused`                          | Admin cannot resume a stream that is not paused                                | `resume_stream_as_admin` |
| `Unauthorized`                                  | Caller is not authorized to perform this operation                             | `set_admin`, `require_sender_or_admin` (internal checks) |
| `InsufficientBalance`                           | Token transfer failed due to insufficient balance or allowance                 | `create_stream`, `cancel_stream`, `cancel_stream_as_admin`, `withdraw` |
| `Overflow calculating total streamable amount` | Overflow occurred when calculating total streamable tokens                     | `create_stream` |
| `contract not initialised: missing config`     | Contract storage not initialized before access                                  | `get_config`, `get_token`, `get_admin` |
| `InvalidState`                                  | Operation attempted on a stream in an invalid state (Paused, Completed, Cancelled) | `pause_stream`, `resume_stream`, `cancel_stream`, `withdraw` |
| `InvalidParams`                                 | Function input parameters are invalid (generic catch-all for asserts)          | `create_stream` |
