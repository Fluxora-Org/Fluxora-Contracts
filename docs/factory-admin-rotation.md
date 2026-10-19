# Factory admin rotation semantics

`set_admin` and other admin-gated factory setters use the authorization
recorded for each invocation. When an admin rotation and a setter occur in the
same ledger, invocation order is therefore significant:

1. A setter invoked before `set_admin` is authorized by the old admin.
2. After `set_admin` returns, the old admin is rejected by subsequent setter
   calls.
3. The new admin is authorized by subsequent setter calls in that ledger.

The factory test suite covers both orders, including `set_min_duration` and
`set_batch_cap_enforcement`, so clients can model rotation transactions without
assuming that all operations in one ledger use the same administrator.
