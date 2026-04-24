# Mainnet Deployment Checklist Alignment

## Purpose

This document ensures mainnet deployment procedures align with protocol semantics and provide externally visible assurances. Treasury operators, recipient applications, and auditors must be able to verify deployment correctness using only on-chain observables and published documentation.

## Scope

Everything materially related to mainnet deployment: initialization parameters, authorization boundaries, state verification, time boundaries, numeric ranges, and failure modes. Intentionally excluded: gas optimization, network infrastructure, key management tooling (documented separately with rationale).

## Verification Status

✅ **Complete alignment verified** between deployment procedures and protocol semantics as of 2026-03-27.

---

## Deployment Roles and Authorization

### Role Definitions

| Role                | Responsibility                 | Authorization Required     | Verification Method            |
| ------------------- | ------------------------------ | -------------------------- | ------------------------------ |
| **Deployer**        | Upload WASM, deploy contract   | Deployer secret key        | Transaction signature          |
| **Bootstrap Admin** | Call `init` once               | Admin secret key           | `admin.require_auth()` in init |
| **Token Contract**  | Provide streaming token        | N/A (address verification) | On-chain contract existence    |
| **Verifier**        | Confirm deployment correctness | None (read-only)           | Public RPC queries             |

### Authorization Boundaries

**Deployer**:

- ✅ Can upload WASM to network
- ✅ Can deploy contract instance
- ❌ Cannot initialize contract (requires admin auth)
- ❌ Cannot modify contract after deployment (immutable)

**Bootstrap Admin**:

- ✅ Can call `init` exactly once
- ✅ Becomes contract admin after init
- ❌ Cannot re-initialize (blocked by `AlreadyInitialised`)
- ❌ Cannot change token address after init (immutable)

**Token Contract**:

- Must exist on-chain before init
- Must implement SEP-41 token interface
- Address is immutable after init

---

## Initialization Semantics

### Success Semantics (Crisp)

**Preconditions**:

1. Contract deployed to mainnet
2. Token contract exists at specified address
3. Admin address is valid Stellar address
4. Admin authorizes the init transaction
5. Init has not been called before

**Observable Effects**:

1. **Storage**: `Config { token, admin }` persisted to instance storage
2. **Storage**: `NextStreamId` initialized to `0`
3. **Storage**: TTL extended (17,280 ledgers threshold, 120,960 bump)
4. **Events**: None (init does not emit events)
5. **Return**: `Ok(())` on success

**Verification**:

```bash
stellar contract invoke \
  --id <CONTRACT_ID> \
  --network mainnet \
  -- get_config
```

Expected output:

```json
{
  "token": "<TOKEN_ADDRESS>",
  "admin": "<ADMIN_ADDRESS>"
}
```

**Code Location**: `contracts/stream/src/lib.rs:665-678`
**Doc Reference**: `docs/streaming.md` §4 Access Control

### Failure Semantics (Crisp)

| Failure Condition        | Error                                 | Side Effects                          | Verification               |
| ------------------------ | ------------------------------------- | ------------------------------------- | -------------------------- |
| Admin does not authorize | Auth failure                          | None                                  | Transaction rejected       |
| Init called twice        | `AlreadyInitialised`                  | None                                  | `try_init` returns error   |
| Invalid token address    | None (address validation is external) | Config persisted with invalid address | Subsequent operations fail |
| Invalid admin address    | None (address validation is external) | Config persisted with invalid address | Admin operations fail      |

**No Silent Drift**:

- Failed init leaves no storage changes
- Failed init emits no events
- Failed init transfers no tokens
- Failed auth is observable in transaction result

---

## Time Boundary Verification

### Deployment Time Constraints

**No time-based constraints on deployment**:

- Contract can be deployed at any time
- Init can be called at any time after deployment
- No expiration on deployment window

**First stream creation constraints**:

- `start_time >= current_ledger_timestamp` (enforced)
- `start_time < end_time` (enforced)
- `cliff_time` in `[start_time, end_time]` (enforced)

**Verification after deployment**:

```bash
# Get current ledger timestamp
stellar contract invoke \
  --id <CONTRACT_ID> \
  --network mainnet \
  -- create_stream \
    --sender <SENDER> \
    --recipient <RECIPIENT> \
    --deposit_amount 1000 \
    --rate_per_second 1 \
    --start_time <FUTURE_TIMESTAMP> \
    --cliff_time <FUTURE_TIMESTAMP> \
    --end_time <FUTURE_TIMESTAMP>
```

**Expected behavior**:

- ✅ Future start_time: Success
- ❌ Past start_time: `StartTimeInPast` error
- ❌ start_time >= end_time: `InvalidParams` error

**Code Location**: `contracts/stream/src/lib.rs:547-549`
**Doc Reference**: `docs/streaming.md` §3 Start Time Boundary

### Cliff and End Time Boundaries

**Cliff behavior** (post-deployment verification):

- Before cliff: `calculate_accrued` returns `0`
- At cliff: `calculate_accrued` returns `(cliff_time - start_time) × rate`
- After cliff: Accrual continues linearly

**End time behavior** (post-deployment verification):

- Before end: Accrual grows linearly
- At end: Accrual capped at `min(rate × duration, deposit_amount)`
- After end: Accrual remains capped (no further growth)

**Verification**:

```bash
# Create stream with known parameters
# Query accrual at different times
stellar contract invoke \
  --id <CONTRACT_ID> \
  --network mainnet \
  -- calculate_accrued \
    --stream_id 0
```

**Code Location**: `contracts/stream/src/accrual.rs:14-42`
**Doc Reference**: `docs/streaming.md` §2 Accrual Formula

---

## Numeric Range Verification

### Deposit Amount Constraints

**Validation rules**:

- `deposit_amount > 0` (enforced)
- `deposit_amount <= i128::MAX` (type system)
- `deposit_amount >= rate_per_second × (end_time - start_time)` (enforced)

**Verification after deployment**:

```bash
# Test with zero deposit (should fail)
stellar contract invoke \
  --id <CONTRACT_ID> \
  --network mainnet \
  -- create_stream \
    --deposit_amount 0 \
    ...
# Expected: InvalidParams error

# Test with insufficient deposit (should fail)
stellar contract invoke \
  --id <CONTRACT_ID> \
  --network mainnet \
  -- create_stream \
    --deposit_amount 100 \
    --rate_per_second 10 \
    --start_time <NOW> \
    --end_time <NOW + 20> \
    ...
# Expected: InsufficientDeposit error (100 < 10 × 20)
```

**Code Location**: `contracts/stream/src/lib.rs:534-556`
**Doc Reference**: `docs/streaming.md` §3 Deposit Validation

### Rate Per Second Constraints

**Validation rules**:

- `rate_per_second > 0` (enforced)
- `rate_per_second <= i128::MAX` (type system)
- `rate_per_second × duration` must not overflow (checked)

**Verification after deployment**:

```bash
# Test with zero rate (should fail)
stellar contract invoke \
  --id <CONTRACT_ID> \
  --network mainnet \
  -- create_stream \
    --rate_per_second 0 \
    ...
# Expected: InvalidParams error

# Test with overflow (should fail)
stellar contract invoke \
  --id <CONTRACT_ID> \
  --network mainnet \
  -- create_stream \
    --rate_per_second <i128::MAX> \
    --start_time 0 \
    --end_time <u64::MAX> \
    ...
# Expected: InvalidParams error (overflow in rate × duration)
```

**Code Location**: `contracts/stream/src/lib.rs:558-563`
**Doc Reference**: `docs/streaming.md` §3 Limits Policy

---

## State Verification Post-Deployment

### Initialization State

**Immediately after init**:

```bash
# Verify config
stellar contract invoke --id <CONTRACT_ID> --network mainnet -- get_config
# Expected: {"token": "<TOKEN>", "admin": "<ADMIN>"}

# Verify stream count
stellar contract invoke --id <CONTRACT_ID> --network mainnet -- get_stream_count
# Expected: 0

# Verify contract version
stellar contract invoke --id <CONTRACT_ID> --network mainnet -- version
# Expected: 1
```

**Code Location**: `contracts/stream/src/lib.rs:665-678, 1493, 1889`
**Doc Reference**: `docs/streaming.md` §4 Access Control

### First Stream Creation

**After creating first stream**:

```bash
# Create stream
STREAM_ID=$(stellar contract invoke \
  --id <CONTRACT_ID> \
  --network mainnet \
  -- create_stream \
    --sender <SENDER> \
    --recipient <RECIPIENT> \
    --deposit_amount 1000 \
    --rate_per_second 1 \
    --start_time <FUTURE> \
    --cliff_time <FUTURE> \
    --end_time <FUTURE>)

# Verify stream state
stellar contract invoke \
  --id <CONTRACT_ID> \
  --network mainnet \
  -- get_stream_state \
    --stream_id $STREAM_ID

# Expected output:
# {
#   "stream_id": 0,
#   "sender": "<SENDER>",
#   "recipient": "<RECIPIENT>",
#   "deposit_amount": 1000,
#   "rate_per_second": 1,
#   "start_time": <FUTURE>,
#   "cliff_time": <FUTURE>,
#   "end_time": <FUTURE>,
#   "withdrawn_amount": 0,
#   "status": "Active",
#   "cancelled_at": null
# }

# Verify stream count incremented
stellar contract invoke --id <CONTRACT_ID> --network mainnet -- get_stream_count
# Expected: 1

# Verify recipient index
stellar contract invoke \
  --id <CONTRACT_ID> \
  --network mainnet \
  -- get_recipient_streams \
    --recipient <RECIPIENT>
# Expected: [0]
```

**Code Location**: `contracts/stream/src/lib.rs:754-819, 1485, 1493, 1920`
**Doc Reference**: `docs/streaming.md` §1 Stream Lifecycle

---

## Event Emission Verification

### StreamCreated Event

**After first stream creation**:

```bash
# Query events from transaction
stellar events --id <TX_HASH> --network mainnet

# Expected event:
# Topic: ("created", 0)
# Payload: StreamCreated {
#   stream_id: 0,
#   sender: "<SENDER>",
#   recipient: "<RECIPIENT>",
#   deposit_amount: 1000,
#   rate_per_second: 1,
#   start_time: <FUTURE>,
#   cliff_time: <FUTURE>,
#   end_time: <FUTURE>
# }
```

**Code Location**: `contracts/stream/src/lib.rs:809-819`
**Doc Reference**: `docs/streaming.md` §5 Events

### No Events on Init

**Init does not emit events**:

- Rationale: Init is a one-time bootstrap operation
- Verification: Query events from init transaction (should be empty)

**Code Location**: `contracts/stream/src/lib.rs:665-678`
**Doc Reference**: `docs/streaming.md` §5 Events (init not listed)

---

## Token Contract Verification

### Token Address Validation

**Pre-deployment checks**:

1. Token contract exists on mainnet
2. Token implements SEP-41 interface
3. Token address is correct (triple-check)

**Post-deployment verification**:

```bash
# Verify token contract exists
stellar contract info --id <TOKEN_ADDRESS> --network mainnet

# Verify token interface (attempt transfer)
stellar contract invoke \
  --id <TOKEN_ADDRESS> \
  --network mainnet \
  -- balance \
    --id <TEST_ADDRESS>
# Expected: Balance returned (confirms SEP-41 compliance)

# Verify streaming contract can interact with token
# (Create stream with small amount)
stellar contract invoke \
  --id <CONTRACT_ID> \
  --network mainnet \
  -- create_stream \
    --sender <FUNDED_SENDER> \
    --recipient <RECIPIENT> \
    --deposit_amount 100 \
    --rate_per_second 1 \
    --start_time <FUTURE> \
    --cliff_time <FUTURE> \
    --end_time <FUTURE>
# Expected: Success (confirms token transfer works)
```

**Failure modes**:

- Invalid token address → Stream creation fails (token transfer fails)
- Non-SEP-41 token → Stream creation fails (interface mismatch)
- Token with transfer restrictions → Stream creation may fail

**Code Location**: `contracts/stream/src/lib.rs:428-442 (pull_token)`
**Doc Reference**: `docs/streaming.md` §1 Scope boundary and exclusions

---

## Admin Key Verification

### Admin Authorization

**Post-deployment verification**:

```bash
# Verify admin can perform admin operations
stellar contract invoke \
  --id <CONTRACT_ID> \
  --network mainnet \
  --source <ADMIN_SECRET> \
  -- set_contract_paused \
    --paused true
# Expected: Success

# Verify non-admin cannot perform admin operations
stellar contract invoke \
  --id <CONTRACT_ID> \
  --network mainnet \
  --source <NON_ADMIN_SECRET> \
  -- set_contract_paused \
    --paused false
# Expected: Auth failure

# Verify admin can rotate admin key
stellar contract invoke \
  --id <CONTRACT_ID> \
  --network mainnet \
  --source <ADMIN_SECRET> \
  -- set_admin \
    --new_admin <NEW_ADMIN_ADDRESS>
# Expected: Success

# Verify new admin works
stellar contract invoke \
  --id <CONTRACT_ID> \
  --network mainnet \
  --source <NEW_ADMIN_SECRET> \
  -- set_contract_paused \
    --paused false
# Expected: Success
```

**Code Location**: `contracts/stream/src/lib.rs:1437-1458, 1461-1468`
**Doc Reference**: `docs/streaming.md` §4 Access Control

---

## Deployment Checklist Alignment

### Pre-Deployment Verification

| Check                      | Protocol Requirement        | Verification Method                         | Doc Reference                             |
| -------------------------- | --------------------------- | ------------------------------------------- | ----------------------------------------- |
| All tests pass             | 95% coverage minimum        | `cargo test -p fluxora_stream`              | CONTRIBUTING.md                           |
| Snapshot tests pass        | No behavior drift           | `cargo test -p fluxora_stream`              | docs/snapshot-tests.md                    |
| Protocol narrative aligned | Zero contradictions         | Review protocol-narrative-code-alignment.md | docs/protocol-narrative-code-alignment.md |
| Token address verified     | SEP-41 compliance           | Query token contract on mainnet             | docs/streaming.md §1                      |
| Admin key secured          | Hardware wallet or MPC      | Key management audit                        | docs/mainnet.md §Risk Summary             |
| Audit completed            | Professional security audit | Audit report review                         | docs/audit.md                             |

### Deployment Execution

| Step                | Protocol Requirement  | Verification Method       | Failure Mode          |
| ------------------- | --------------------- | ------------------------- | --------------------- |
| Build WASM          | Reproducible build    | Compare SHA256 hash       | Build mismatch        |
| Upload WASM         | Correct binary        | Verify WASM hash on-chain | Wrong binary deployed |
| Deploy contract     | Correct network       | Verify contract address   | Wrong network         |
| Initialize contract | Correct parameters    | `get_config` query        | Wrong config          |
| Verify token        | SEP-41 compliance     | Test token transfer       | Token incompatible    |
| Verify admin        | Correct authorization | Test admin operation      | Wrong admin           |

### Post-Deployment Verification

| Check                   | Expected Behavior       | Verification Method      | Failure Indicator         |
| ----------------------- | ----------------------- | ------------------------ | ------------------------- |
| Config readable         | Returns token and admin | `get_config`             | Error or wrong values     |
| Stream count zero       | Returns 0               | `get_stream_count`       | Non-zero value            |
| Version correct         | Returns 1               | `version`                | Wrong version             |
| Create stream works     | Returns stream_id 0     | `create_stream`          | Error or wrong ID         |
| Stream state correct    | Matches input params    | `get_stream_state`       | Mismatched values         |
| Recipient index updated | Contains stream_id      | `get_recipient_streams`  | Empty or wrong IDs        |
| Event emitted           | StreamCreated event     | Query transaction events | No event or wrong payload |
| Token transferred       | Balance decreased       | Query token balance      | No transfer               |

---

## Edge Case Verification

### Time Boundary Edge Cases

**Test immediately after deployment**:

1. **Start time in past**:

```bash
stellar contract invoke \
  --id <CONTRACT_ID> \
  --network mainnet \
  -- create_stream \
    --start_time <PAST_TIMESTAMP> \
    ...
# Expected: StartTimeInPast error
```

2. **Start time equals end time**:

```bash
stellar contract invoke \
  --id <CONTRACT_ID> \
  --network mainnet \
  -- create_stream \
    --start_time 1000 \
    --end_time 1000 \
    ...
# Expected: InvalidParams error
```

3. **Cliff before start**:

```bash
stellar contract invoke \
  --id <CONTRACT_ID> \
  --network mainnet \
  -- create_stream \
    --start_time 1000 \
    --cliff_time 999 \
    --end_time 2000 \
    ...
# Expected: InvalidParams error
```

4. **Cliff after end**:

```bash
stellar contract invoke \
  --id <CONTRACT_ID> \
  --network mainnet \
  -- create_stream \
    --start_time 1000 \
    --cliff_time 2001 \
    --end_time 2000 \
    ...
# Expected: InvalidParams error
```

**Code Location**: `contracts/stream/src/lib.rs:547-556`
**Doc Reference**: `docs/streaming.md` §3 Cliff and end_time Behavior

### Numeric Range Edge Cases

**Test immediately after deployment**:

1. **Zero deposit**:

```bash
stellar contract invoke \
  --id <CONTRACT_ID> \
  --network mainnet \
  -- create_stream \
    --deposit_amount 0 \
    ...
# Expected: InvalidParams error
```

2. **Zero rate**:

```bash
stellar contract invoke \
  --id <CONTRACT_ID> \
  --network mainnet \
  -- create_stream \
    --rate_per_second 0 \
    ...
# Expected: InvalidParams error
```

3. **Insufficient deposit**:

```bash
stellar contract invoke \
  --id <CONTRACT_ID> \
  --network mainnet \
  -- create_stream \
    --deposit_amount 100 \
    --rate_per_second 10 \
    --start_time <NOW> \
    --end_time <NOW + 20> \
    ...
# Expected: InsufficientDeposit error
```

4. **Overflow in rate × duration**:

```bash
stellar contract invoke \
  --id <CONTRACT_ID> \
  --network mainnet \
  -- create_stream \
    --rate_per_second <i128::MAX> \
    --start_time 0 \
    --end_time <u64::MAX> \
    ...
# Expected: InvalidParams error
```

**Code Location**: `contracts/stream/src/lib.rs:534-563`
**Doc Reference**: `docs/streaming.md` §3 Deposit Validation

### Authorization Edge Cases

**Test immediately after deployment**:

1. **Non-admin init attempt**:

```bash
stellar contract invoke \
  --id <CONTRACT_ID> \
  --network mainnet \
  --source <NON_ADMIN_SECRET> \
  -- init \
    --token <TOKEN> \
    --admin <ADMIN>
# Expected: Auth failure
```

2. **Double init attempt**:

```bash
stellar contract invoke \
  --id <CONTRACT_ID> \
  --network mainnet \
  --source <ADMIN_SECRET> \
  -- init \
    --token <TOKEN> \
    --admin <ADMIN>
# Expected: AlreadyInitialised error
```

3. **Non-sender pause attempt**:

```bash
# Create stream as sender A
# Attempt pause as sender B
stellar contract invoke \
  --id <CONTRACT_ID> \
  --network mainnet \
  --source <SENDER_B_SECRET> \
  -- pause_stream \
    --stream_id 0
# Expected: Auth failure
```

**Code Location**: `contracts/stream/src/lib.rs:665-678, 906-925`
**Doc Reference**: `docs/streaming.md` §4 Access Control

---

## Residual Risks (Explicitly Excluded)

### Out of Scope for Deployment Checklist

1. **Gas cost optimization**:
   - Rationale: Gas costs vary by network congestion
   - Mitigation: Test on testnet, monitor mainnet costs
   - Doc: Not in deployment checklist scope

2. **Key management tooling**:
   - Rationale: Organization-specific infrastructure
   - Mitigation: Hardware wallets, MPC, multi-sig
   - Doc: docs/mainnet.md §Security Best Practices

3. **Network infrastructure**:
   - Rationale: Stellar network responsibility
   - Mitigation: Use reliable RPC providers
   - Doc: docs/DEPLOYMENT.md §Network Details

4. **Token contract bugs**:
   - Rationale: External dependency
   - Mitigation: Verify SEP-41 compliance, test transfers
   - Doc: docs/streaming.md §1 Scope boundary

5. **Regulatory compliance**:
   - Rationale: Jurisdiction-specific requirements
   - Mitigation: Legal review before deployment
   - Doc: Not in technical documentation scope

✅ **All exclusions documented with rationale**

---

## Integrator Assurances

### For Treasury Operators

Post-deployment, you can verify:

- ✅ Token address is correct (`get_config`)
- ✅ Admin address is correct (`get_config`)
- ✅ Stream creation works (test stream)
- ✅ Authorization boundaries enforced (test non-admin operations)
- ✅ Time boundaries enforced (test past start_time)
- ✅ Numeric boundaries enforced (test zero deposit)

### For Recipient Applications

Post-deployment, you can verify:

- ✅ Accrual formula is correct (test `calculate_accrued`)
- ✅ Withdrawal works (test `withdraw`)
- ✅ Events are emitted (query transaction events)
- ✅ Recipient index works (`get_recipient_streams`)

### For Auditors

Post-deployment, you can verify:

- ✅ All protocol semantics match documentation
- ✅ No hidden state transitions
- ✅ Error classifications are correct
- ✅ Event emissions are complete
- ✅ Authorization boundaries are enforced

### For Indexers

Post-deployment, you can verify:

- ✅ Event schemas match documentation
- ✅ Event ordering is deterministic
- ✅ Status transitions are observable
- ✅ No silent state changes

---

## Maintenance

When deploying to mainnet:

1. Follow this checklist in order
2. Verify each step before proceeding
3. Document all verification results
4. Save all transaction hashes
5. Monitor contract for 24-48 hours post-deployment

When updating documentation:

1. Update this alignment document
2. Update docs/mainnet.md
3. Update docs/DEPLOYMENT.md
4. Run verification tests
5. Document changes in PR

Last verified: 2026-03-27
