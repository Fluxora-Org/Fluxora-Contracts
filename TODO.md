# TODO: Add Optional Cancellation Fee (Issue #434)

## Plan

1. **Add `cancellation_fee_bps` field to structs:**
   - `Stream`
   - `CreateStreamParams`
   - `CreateStreamRelativeParams`

2. **Bump `CONTRACT_VERSION`** from 2 → 3 (breaking ABI change)

3. **Update `validate_stream_params`** to validate `cancellation_fee_bps <= 10000`

4. **Update `persist_new_stream`** to accept and store `cancellation_fee_bps`

5. **Update `create_stream` signature** and propagate through:
   - `create_stream_relative`
   - `create_streams`
   - `create_streams_relative`

6. **Update `cancel_stream_internal`** to compute fee and apply to refund only

7. **Run `cargo test -p fluxora_stream`** and fix any issues

8. **Update documentation** if gaps found
