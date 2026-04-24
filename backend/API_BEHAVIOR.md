# API Behavior Reference

## Idempotency Keys

### Overview

Stream creation (`POST /api/v1/streams`) supports idempotency keys so clients
can safely retry requests without creating duplicate streams.

### How to use

Add the `Idempotency-Key` header with a UUID v4 value to any `POST /api/v1/streams` request:

```http
POST /api/v1/streams HTTP/1.1
Authorization: Bearer <jwt>
Idempotency-Key: 550e8400-e29b-41d4-a716-446655440000
Content-Type: application/json

{
  "recipientId": "a1b2c3d4-e5f6-4789-abcd-ef0123456789",
  "depositAmount": "1000000",
  "ratePerSecond": "100",
  "startTime": 1700000000,
  "endTime": 1700010000
}
```

### Behavior matrix

| Scenario | Status | Body |
|---|---|---|
| First request with key | `201 Created` | Stream object |
| Replay (same key, same user, within 24 h) | `200 OK` (original status) | Original response body |
| Concurrent duplicate (in-flight) | `409 Conflict` | Error envelope |
| Missing header | `201 Created` | Stream object (no idempotency protection) |
| Malformed key (not UUID v4) | `400 Bad Request` | Error envelope |
| Key from different user | Treated as a new key (scoped per user) | Normal processing |

### Key scoping and security

- Keys are scoped to the **authenticated user**. User A's key `abc` and User B's
  key `abc` are independent — there is no cross-user replay risk.
- The raw key value is **never logged**. Only a truncated SHA-256 hash is
  written to logs so secrets embedded in keys cannot leak into log aggregators.
- Keys must be **UUID v4** format. Any other value is rejected with `400`.

### TTL

Idempotency results are cached for **24 hours**. After expiry the key can be
reused and will be treated as a new request.

### In-flight detection

If two requests with the same key arrive simultaneously (before the first
completes), the second receives `409 Conflict`. The client should wait briefly
and retry — the first request will complete and subsequent replays will return
the cached result.

The in-flight sentinel has a **30-second TTL** so a crashed process cannot
permanently block a key.

---

## Amount field serialization

All on-chain / API amount fields are serialized as **decimal strings**:

| Field | Type in JSON | Example |
|---|---|---|
| `depositAmount` | `string` | `"1000000"` |
| `ratePerSecond` | `string` | `"100"` |

This guarantees precision for values that exceed JavaScript's
`Number.MAX_SAFE_INTEGER` (2^53 − 1 ≈ 9 × 10^15). Clients must treat these
fields as opaque decimal strings and use a big-integer library when performing
arithmetic.

---

## Error codes

| Code | HTTP Status | Meaning |
|---|---|---|
| `VALIDATION_ERROR` | 400 | Request body failed validation |
| `INVALID_IDEMPOTENCY_KEY` | 400 | `Idempotency-Key` header is not a UUID v4 |
| `UNAUTHORIZED` | 401 | Missing or invalid JWT |
| `FORBIDDEN` | 403 | Insufficient permissions |
| `NOT_FOUND` | 404 | Resource not found |
| `IDEMPOTENCY_CONFLICT` | 409 | Concurrent duplicate request |
| `CONFLICT` | 409 | General conflict |
| `RATE_LIMIT_EXCEEDED` | 429 | Too many requests |
| `INTERNAL_ERROR` | 500 | Unexpected server error |

---

## Correlation IDs

Include `X-Correlation-Id: <value>` in any request to propagate a trace ID
through all log entries for that request. If omitted, the server generates one
internally. The value appears in error response bodies as `correlationId` and
in all structured log entries for that request.

---

## Rate limits

| Endpoint | Window | Max requests |
|---|---|---|
| `POST /api/v1/streams` | 15 minutes | 30 |
| Standard user endpoints | 15 minutes | 100 |
| Sensitive endpoints (delete) | 15 minutes | 10 |
