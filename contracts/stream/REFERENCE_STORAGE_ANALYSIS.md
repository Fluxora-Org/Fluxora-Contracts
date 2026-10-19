# Stream Reference Field Storage Cost Analysis

## Overview

The stream reference field is implemented as `Option<String>` with a maximum length of 64 characters (`MAX_REFERENCE_LENGTH = 64`). This document analyzes the storage cost contribution of this field.

## Storage Cost Breakdown

### Base Stream Structure
The existing Stream struct contains:
- 3 Address fields (sender, recipient, token): ~96 bytes
- 2 i128 amounts (deposited, withdrawn): ~32 bytes  
- 3 u64 timestamps (start_time, end_time, cliff_time): ~24 bytes
- 3 bool flags (cancellable, pausable, transferable): ~3 bytes
- Option<u64> paused_at: ~9 bytes
- u64 paused_total: ~8 bytes
- StreamStatus enum: ~1 byte

**Base struct size: ~173 bytes**

### Reference Field Addition
The `Option<String>` reference field adds:

| Reference Value | Additional Storage | Total Overhead |
|----------------|------------------|---------------|
| `None` | 1 byte (discriminant) | +1 byte |
| `Some("")` (empty) | 1 + 8 bytes (discriminant + string overhead) | +9 bytes |
| `Some("a")` (1 char) | 1 + 8 + 1 bytes | +10 bytes |
| `Some("payroll-001")` (11 chars) | 1 + 8 + 11 bytes | +20 bytes |
| `Some(64 chars)` (maximum) | 1 + 8 + 64 bytes | +73 bytes |

### Storage Model Impact

#### Persistent Storage
- Each stream is stored as a separate persistent storage entry under `DataKey::Stream(id)`
- Reference field is part of the Stream struct, so no additional storage keys needed
- Each persistent storage entry has independent TTL management

#### Ledger Entry Limits
- Individual ledger entry size limit: 64 KB
- A single Stream entry with maximum reference: ~246 bytes (173 + 73)
- Theoretical streams per 64 KB: ~267 streams (if all stored in one entry, but they're not)
- In practice: Each stream gets its own persistent storage entry, so no limit concerns

## Cost Analysis

### Storage Rent Costs
Based on Soroban's storage pricing model:
- Persistent storage cost scales with entry size and TTL duration
- Reference field adds 1-73 bytes per stream (0.1%-30% overhead)
- For typical 20-character references: ~29 bytes overhead (~17% increase)

### Network Bandwidth
- StreamCreated events include the reference field
- Event size increase: 1-73 bytes per creation event
- Minimal impact on transaction size

## Recommendations

1. **Optimal Reference Length**: 8-32 characters for most use cases
   - Balances utility with storage efficiency
   - Common patterns: "payroll-001", "grant-2024-q1", "salary-john-doe"

2. **Cost Budgeting**: 
   - Budget ~30 bytes additional storage per stream for typical references
   - Maximum 73 bytes for worst-case planning

3. **Indexing Strategy**:
   - Off-chain indexers can use references for stream organization
   - No additional on-chain indexing structures needed

## Validation

The implementation includes length validation:
- Empty references (None or empty string) are allowed
- References exceeding 64 characters are rejected with `InvalidReferenceLength`
- UTF-8 encoding is handled by Soroban's String type

## Comparison with Alternatives

| Alternative | Storage Cost | Pros | Cons |
|------------|--------------|------|------|
| Symbol (up to 32 chars) | ~8 bytes fixed | Smaller size | Limited character set (a-z, A-Z, 0-9, _) |
| BytesN<32> | 32 bytes fixed | Fixed size | Wastes space for short references |
| String (unbounded) | Variable | Full flexibility | DoS risk without bounds |
| **Option<String> with 64-char limit** | **1-73 bytes** | **Good balance** | **Chosen implementation** |

## Testing

The storage cost impact is validated through:
- Unit tests with various reference lengths
- Verification of stream creation and retrieval
- Event emission validation