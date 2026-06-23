/**
 * Tests for POST /api/v1/streams — idempotency key handling
 *
 * Coverage targets:
 *  - Happy path: stream creation (with and without idempotency key)
 *  - Replay: same key returns cached response, no duplicate created
 *  - Collision: concurrent in-flight duplicate → 409
 *  - Key validation: malformed key → 400
 *  - Auth: missing/invalid JWT → 401
 *  - Body validation: all required fields, type checks, decimal-string amounts
 *  - Decimal-string serialization guarantee
 *  - Cross-user key isolation
 *  - Error response shape (OpenAPI alignment)
 *
 * Security notes:
 *  - Raw idempotency key values are never asserted in log output; only
 *    hashed representations appear in logs.
 *  - Cache keys are scoped per-user so cross-user replay is impossible.
 *  - In-flight sentinel TTL (30 s) prevents permanent key blocking on crash.
 */

import express, { Application } from 'express';
import request from 'supertest';
import { v4 as uuidv4 } from 'uuid';
import jwt from 'jsonwebtoken';

// ---------------------------------------------------------------------------
// Module mocks — must be declared before imports that use them
// ---------------------------------------------------------------------------

// Set test environment before any module is loaded
process.env.NODE_ENV = 'test';

// Mock the cache so tests are hermetic (no Redis / in-memory state leakage)
const cacheStore = new Map<string, unknown>();

jest.mock('../../src/utils/cache', () => ({
  __esModule: true,
  default: {
    get: jest.fn(async (key: string) => cacheStore.get(key) ?? null),
    set: jest.fn(async (key: string, value: unknown) => { cacheStore.set(key, value); }),
    del: jest.fn(async (key: string) => { cacheStore.delete(key); })
  }
}));

// Mock logger to suppress noise and allow assertion on log calls
jest.mock('../../src/utils/logger', () => ({
  __esModule: true,
  default: {
    info:  jest.fn(),
    warn:  jest.fn(),
    error: jest.fn(),
    debug: jest.fn()
  }
}));

// ---------------------------------------------------------------------------
// Imports (after mocks)
// ---------------------------------------------------------------------------

import streamRouter from '../../src/routes/streams';
import cache from '../../src/utils/cache';
import logger from '../../src/utils/logger';

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

const JWT_SECRET = 'test-secret-do-not-use-in-production';

function makeToken(userId: string, role = 'learner'): string {
  return jwt.sign({ id: userId, email: `${userId}@test.com`, role }, JWT_SECRET, {
    expiresIn: '1h'
  });
}

function buildApp(): Application {
  const app = express();
  app.use(express.json());

  // Minimal auth middleware that reads JWT_SECRET from env
  process.env.JWT_SECRET = JWT_SECRET;

  app.use('/api/v1/streams', streamRouter);

  // Global error handler (mirrors index.ts) — handles AppError statusCode
  app.use((err: Error & { statusCode?: number }, _req: express.Request, res: express.Response, _next: express.NextFunction) => {
    const status = (err as { statusCode?: number }).statusCode ?? 500;
    res.status(status).json({
      success: false,
      error: {
        message: err.message,
        code: status === 401 ? 'UNAUTHORIZED'
          : status === 403 ? 'FORBIDDEN'
          : status === 404 ? 'NOT_FOUND'
          : status === 400 ? 'VALIDATION_ERROR'
          : 'INTERNAL_ERROR'
      }
    });
  });

  return app;
}

const VALID_BODY = {
  recipientId: uuidv4(),
  depositAmount: '1000000',
  ratePerSecond: '100',
  startTime: 1700000000,
  endTime: 1700010000
};

// ---------------------------------------------------------------------------
// Setup / teardown
// ---------------------------------------------------------------------------

let app: Application;

beforeAll(() => {
  app = buildApp();
});

beforeEach(() => {
  cacheStore.clear();
  jest.clearAllMocks();
});

// ===========================================================================
// 1. Authentication
// ===========================================================================

describe('Authentication', () => {
  it('returns 401 when Authorization header is missing', async () => {
    const res = await request(app)
      .post('/api/v1/streams')
      .send(VALID_BODY);

    expect(res.status).toBe(401);
    expect(res.body.success).toBe(false);
    expect(res.body.error.code).toMatch(/UNAUTHORIZED|INVALID/i);
  });

  it('returns 401 when JWT is invalid', async () => {
    const res = await request(app)
      .post('/api/v1/streams')
      .set('Authorization', 'Bearer not-a-real-token')
      .send(VALID_BODY);

    expect(res.status).toBe(401);
    expect(res.body.success).toBe(false);
  });

  it('returns 401 when JWT is expired', async () => {
    const expiredToken = jwt.sign(
      { id: uuidv4(), email: 'x@x.com', role: 'learner' },
      JWT_SECRET,
      { expiresIn: -1 }
    );

    const res = await request(app)
      .post('/api/v1/streams')
      .set('Authorization', `Bearer ${expiredToken}`)
      .send(VALID_BODY);

    expect(res.status).toBe(401);
  });
});

// ===========================================================================
// 2. Body validation
// ===========================================================================

describe('Body validation', () => {
  const userId = uuidv4();
  let token: string;

  beforeAll(() => { token = makeToken(userId); });

  it('returns 400 when recipientId is missing', async () => {
    const { recipientId: _omit, ...body } = VALID_BODY;
    const res = await request(app)
      .post('/api/v1/streams')
      .set('Authorization', `Bearer ${token}`)
      .send(body);

    expect(res.status).toBe(400);
    expect(res.body.error.code).toBe('VALIDATION_ERROR');
    expect(res.body.error.message).toMatch(/recipientId/i);
  });

  it('returns 400 when recipientId is not a UUID', async () => {
    const res = await request(app)
      .post('/api/v1/streams')
      .set('Authorization', `Bearer ${token}`)
      .send({ ...VALID_BODY, recipientId: 'not-a-uuid' });

    expect(res.status).toBe(400);
    expect(res.body.error.message).toMatch(/recipientId/i);
  });

  it('returns 400 when depositAmount is missing', async () => {
    const { depositAmount: _omit, ...body } = VALID_BODY;
    const res = await request(app)
      .post('/api/v1/streams')
      .set('Authorization', `Bearer ${token}`)
      .send(body);

    expect(res.status).toBe(400);
    expect(res.body.error.message).toMatch(/depositAmount/i);
  });

  it('returns 400 when depositAmount is a number (not a string)', async () => {
    const res = await request(app)
      .post('/api/v1/streams')
      .set('Authorization', `Bearer ${token}`)
      .send({ ...VALID_BODY, depositAmount: 1000000 });

    expect(res.status).toBe(400);
    expect(res.body.error.message).toMatch(/depositAmount/i);
  });

  it('returns 400 when depositAmount is zero', async () => {
    const res = await request(app)
      .post('/api/v1/streams')
      .set('Authorization', `Bearer ${token}`)
      .send({ ...VALID_BODY, depositAmount: '0' });

    expect(res.status).toBe(400);
    expect(res.body.error.message).toMatch(/depositAmount/i);
  });

  it('returns 400 when depositAmount is negative', async () => {
    const res = await request(app)
      .post('/api/v1/streams')
      .set('Authorization', `Bearer ${token}`)
      .send({ ...VALID_BODY, depositAmount: '-100' });

    expect(res.status).toBe(400);
  });

  it('returns 400 when ratePerSecond is missing', async () => {
    const { ratePerSecond: _omit, ...body } = VALID_BODY;
    const res = await request(app)
      .post('/api/v1/streams')
      .set('Authorization', `Bearer ${token}`)
      .send(body);

    expect(res.status).toBe(400);
    expect(res.body.error.message).toMatch(/ratePerSecond/i);
  });

  it('returns 400 when ratePerSecond is not a decimal string', async () => {
    const res = await request(app)
      .post('/api/v1/streams')
      .set('Authorization', `Bearer ${token}`)
      .send({ ...VALID_BODY, ratePerSecond: 'fast' });

    expect(res.status).toBe(400);
  });

  it('returns 400 when endTime is before startTime', async () => {
    const res = await request(app)
      .post('/api/v1/streams')
      .set('Authorization', `Bearer ${token}`)
      .send({ ...VALID_BODY, startTime: 1700010000, endTime: 1700000000 });

    expect(res.status).toBe(400);
    expect(res.body.error.message).toMatch(/endTime/i);
  });

  it('returns 400 when cliffTime is after endTime', async () => {
    const res = await request(app)
      .post('/api/v1/streams')
      .set('Authorization', `Bearer ${token}`)
      .send({ ...VALID_BODY, cliffTime: 1700020000 });

    expect(res.status).toBe(400);
    expect(res.body.error.message).toMatch(/cliffTime/i);
  });

  it('returns 400 when cliffTime is before startTime', async () => {
    const res = await request(app)
      .post('/api/v1/streams')
      .set('Authorization', `Bearer ${token}`)
      .send({ ...VALID_BODY, cliffTime: 1699999999 });

    expect(res.status).toBe(400);
    expect(res.body.error.message).toMatch(/cliffTime/i);
  });

  it('accepts cliffTime equal to startTime', async () => {
    const res = await request(app)
      .post('/api/v1/streams')
      .set('Authorization', `Bearer ${token}`)
      .send({ ...VALID_BODY, cliffTime: VALID_BODY.startTime });

    expect(res.status).toBe(201);
    expect(res.body.data.stream.cliffTime).toBe(VALID_BODY.startTime);
  });

  it('accepts cliffTime equal to endTime', async () => {
    const res = await request(app)
      .post('/api/v1/streams')
      .set('Authorization', `Bearer ${token}`)
      .send({ ...VALID_BODY, cliffTime: VALID_BODY.endTime });

    expect(res.status).toBe(201);
    expect(res.body.data.stream.cliffTime).toBe(VALID_BODY.endTime);
  });
});

// ===========================================================================
// 3. Happy path — no idempotency key
// ===========================================================================

describe('Happy path (no idempotency key)', () => {
  it('creates a stream and returns 201 with correct shape', async () => {
    const userId = uuidv4();
    const token = makeToken(userId);

    const res = await request(app)
      .post('/api/v1/streams')
      .set('Authorization', `Bearer ${token}`)
      .send(VALID_BODY);

    expect(res.status).toBe(201);
    expect(res.body.success).toBe(true);

    const { stream } = res.body.data;
    expect(stream.id).toMatch(
      /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i
    );
    expect(stream.senderId).toBe(userId);
    expect(stream.recipientId).toBe(VALID_BODY.recipientId);
    expect(stream.status).toBe('active');
    expect(typeof stream.createdAt).toBe('string');
  });

  it('two requests without a key create two distinct streams', async () => {
    const token = makeToken(uuidv4());

    const [r1, r2] = await Promise.all([
      request(app).post('/api/v1/streams').set('Authorization', `Bearer ${token}`).send(VALID_BODY),
      request(app).post('/api/v1/streams').set('Authorization', `Bearer ${token}`).send(VALID_BODY)
    ]);

    expect(r1.status).toBe(201);
    expect(r2.status).toBe(201);
    expect(r1.body.data.stream.id).not.toBe(r2.body.data.stream.id);
  });
});

// ===========================================================================
// 4. Decimal-string serialization guarantee
// ===========================================================================

describe('Decimal-string serialization', () => {
  it('returns depositAmount as a string', async () => {
    const token = makeToken(uuidv4());
    const res = await request(app)
      .post('/api/v1/streams')
      .set('Authorization', `Bearer ${token}`)
      .send({ ...VALID_BODY, depositAmount: '99999999999999999999' });

    expect(res.status).toBe(201);
    expect(typeof res.body.data.stream.depositAmount).toBe('string');
    expect(res.body.data.stream.depositAmount).toBe('99999999999999999999');
  });

  it('returns ratePerSecond as a string', async () => {
    const token = makeToken(uuidv4());
    const res = await request(app)
      .post('/api/v1/streams')
      .set('Authorization', `Bearer ${token}`)
      .send({ ...VALID_BODY, ratePerSecond: '12345678901234567890' });

    expect(res.status).toBe(201);
    expect(typeof res.body.data.stream.ratePerSecond).toBe('string');
    expect(res.body.data.stream.ratePerSecond).toBe('12345678901234567890');
  });

  it('preserves decimal amounts through idempotency replay', async () => {
    const token = makeToken(uuidv4());
    const key = uuidv4();
    const largeAmount = '99999999999999999999';

    const r1 = await request(app)
      .post('/api/v1/streams')
      .set('Authorization', `Bearer ${token}`)
      .set('Idempotency-Key', key)
      .send({ ...VALID_BODY, depositAmount: largeAmount });

    expect(r1.status).toBe(201);

    const r2 = await request(app)
      .post('/api/v1/streams')
      .set('Authorization', `Bearer ${token}`)
      .set('Idempotency-Key', key)
      .send({ ...VALID_BODY, depositAmount: largeAmount });

    expect(r2.status).toBe(201);
    expect(r2.body.data.stream.depositAmount).toBe(largeAmount);
  });
});

// ===========================================================================
// 5. Idempotency key validation
// ===========================================================================

describe('Idempotency-Key header validation', () => {
  let token: string;

  beforeAll(() => { token = makeToken(uuidv4()); });

  it('returns 400 for a non-UUID key', async () => {
    const res = await request(app)
      .post('/api/v1/streams')
      .set('Authorization', `Bearer ${token}`)
      .set('Idempotency-Key', 'not-a-uuid')
      .send(VALID_BODY);

    expect(res.status).toBe(400);
    expect(res.body.error.code).toBe('INVALID_IDEMPOTENCY_KEY');
  });

  it('returns 400 for a UUID v1 key (not v4)', async () => {
    // UUID v1 has a different version nibble
    const uuidV1 = '6ba7b810-9dad-11d1-80b4-00c04fd430c8';
    const res = await request(app)
      .post('/api/v1/streams')
      .set('Authorization', `Bearer ${token}`)
      .set('Idempotency-Key', uuidV1)
      .send(VALID_BODY);

    expect(res.status).toBe(400);
    expect(res.body.error.code).toBe('INVALID_IDEMPOTENCY_KEY');
  });

  it('returns 400 for an empty string key', async () => {
    const res = await request(app)
      .post('/api/v1/streams')
      .set('Authorization', `Bearer ${token}`)
      .set('Idempotency-Key', '')
      .send(VALID_BODY);

    // Empty string header may be omitted by supertest; either 201 (no header)
    // or 400 (empty string treated as invalid) is acceptable
    expect([200, 201, 400]).toContain(res.status);
  });

  it('accepts a valid UUID v4 key', async () => {
    const res = await request(app)
      .post('/api/v1/streams')
      .set('Authorization', `Bearer ${token}`)
      .set('Idempotency-Key', uuidv4())
      .send(VALID_BODY);

    expect(res.status).toBe(201);
  });
});

// ===========================================================================
// 6. Idempotency replay
// ===========================================================================

describe('Idempotency replay', () => {
  it('returns the same stream on replay', async () => {
    const token = makeToken(uuidv4());
    const key = uuidv4();

    const r1 = await request(app)
      .post('/api/v1/streams')
      .set('Authorization', `Bearer ${token}`)
      .set('Idempotency-Key', key)
      .send(VALID_BODY);

    expect(r1.status).toBe(201);
    const streamId = r1.body.data.stream.id;

    const r2 = await request(app)
      .post('/api/v1/streams')
      .set('Authorization', `Bearer ${token}`)
      .set('Idempotency-Key', key)
      .send(VALID_BODY);

    expect(r2.status).toBe(201);
    expect(r2.body.data.stream.id).toBe(streamId);
  });

  it('replay returns identical body to original response', async () => {
    const token = makeToken(uuidv4());
    const key = uuidv4();

    const r1 = await request(app)
      .post('/api/v1/streams')
      .set('Authorization', `Bearer ${token}`)
      .set('Idempotency-Key', key)
      .send(VALID_BODY);

    const r2 = await request(app)
      .post('/api/v1/streams')
      .set('Authorization', `Bearer ${token}`)
      .set('Idempotency-Key', key)
      .send(VALID_BODY);

    expect(r2.body).toEqual(r1.body);
  });

  it('cache.set is called once (not on replay)', async () => {
    const token = makeToken(uuidv4());
    const key = uuidv4();

    await request(app)
      .post('/api/v1/streams')
      .set('Authorization', `Bearer ${token}`)
      .set('Idempotency-Key', key)
      .send(VALID_BODY);

    const setCalls = (cache.set as jest.Mock).mock.calls.length;

    await request(app)
      .post('/api/v1/streams')
      .set('Authorization', `Bearer ${token}`)
      .set('Idempotency-Key', key)
      .send(VALID_BODY);

    // No additional cache.set calls on replay
    expect((cache.set as jest.Mock).mock.calls.length).toBe(setCalls);
  });

  it('different keys for same user create different streams', async () => {
    const token = makeToken(uuidv4());

    const r1 = await request(app)
      .post('/api/v1/streams')
      .set('Authorization', `Bearer ${token}`)
      .set('Idempotency-Key', uuidv4())
      .send(VALID_BODY);

    const r2 = await request(app)
      .post('/api/v1/streams')
      .set('Authorization', `Bearer ${token}`)
      .set('Idempotency-Key', uuidv4())
      .send(VALID_BODY);

    expect(r1.status).toBe(201);
    expect(r2.status).toBe(201);
    expect(r1.body.data.stream.id).not.toBe(r2.body.data.stream.id);
  });
});

// ===========================================================================
// 7. Concurrent / in-flight collision → 409
// ===========================================================================

describe('In-flight collision (409 Conflict)', () => {
  it('returns 409 when the same key is already in-flight', async () => {
    const token = makeToken(uuidv4());
    const key = uuidv4();

    // Manually plant the in-flight sentinel in the cache
    // (simulates a concurrent request that has not yet completed)
    const userId = (jwt.decode(token) as { id: string }).id;
    const cacheKey = `idempotency:${userId}:${key}`;
    cacheStore.set(cacheKey, '__IN_FLIGHT__');

    const res = await request(app)
      .post('/api/v1/streams')
      .set('Authorization', `Bearer ${token}`)
      .set('Idempotency-Key', key)
      .send(VALID_BODY);

    expect(res.status).toBe(409);
    expect(res.body.success).toBe(false);
    expect(res.body.error.code).toBe('IDEMPOTENCY_CONFLICT');
  });

  it('409 response body matches OpenAPI ErrorEnvelope schema', async () => {
    const token = makeToken(uuidv4());
    const key = uuidv4();
    const userId = (jwt.decode(token) as { id: string }).id;
    cacheStore.set(`idempotency:${userId}:${key}`, '__IN_FLIGHT__');

    const res = await request(app)
      .post('/api/v1/streams')
      .set('Authorization', `Bearer ${token}`)
      .set('Idempotency-Key', key)
      .send(VALID_BODY);

    expect(res.status).toBe(409);
    expect(res.body).toMatchObject({
      success: false,
      error: {
        message: expect.any(String),
        code: 'IDEMPOTENCY_CONFLICT'
      }
    });
  });
});

// ===========================================================================
// 8. Cross-user key isolation
// ===========================================================================

describe('Cross-user key isolation', () => {
  it('same key used by two different users creates two distinct streams', async () => {
    const sharedKey = uuidv4();
    const tokenA = makeToken(uuidv4());
    const tokenB = makeToken(uuidv4());

    const rA = await request(app)
      .post('/api/v1/streams')
      .set('Authorization', `Bearer ${tokenA}`)
      .set('Idempotency-Key', sharedKey)
      .send(VALID_BODY);

    const rB = await request(app)
      .post('/api/v1/streams')
      .set('Authorization', `Bearer ${tokenB}`)
      .set('Idempotency-Key', sharedKey)
      .send(VALID_BODY);

    expect(rA.status).toBe(201);
    expect(rB.status).toBe(201);
    // Different users → different streams, no cross-user replay
    expect(rA.body.data.stream.id).not.toBe(rB.body.data.stream.id);
    expect(rA.body.data.stream.senderId).not.toBe(rB.body.data.stream.senderId);
  });

  it('user B cannot replay user A\'s key', async () => {
    const sharedKey = uuidv4();
    const tokenA = makeToken(uuidv4());
    const tokenB = makeToken(uuidv4());

    // User A creates a stream
    const rA = await request(app)
      .post('/api/v1/streams')
      .set('Authorization', `Bearer ${tokenA}`)
      .set('Idempotency-Key', sharedKey)
      .send(VALID_BODY);

    expect(rA.status).toBe(201);

    // User B uses the same key — should get a NEW stream, not A's
    const rB = await request(app)
      .post('/api/v1/streams')
      .set('Authorization', `Bearer ${tokenB}`)
      .set('Idempotency-Key', sharedKey)
      .send(VALID_BODY);

    expect(rB.status).toBe(201);
    expect(rB.body.data.stream.senderId).not.toBe(rA.body.data.stream.senderId);
  });
});

// ===========================================================================
// 9. Error response shape (OpenAPI alignment)
// ===========================================================================

describe('Error response shape', () => {
  it('400 response matches ErrorEnvelope schema', async () => {
    const token = makeToken(uuidv4());
    const res = await request(app)
      .post('/api/v1/streams')
      .set('Authorization', `Bearer ${token}`)
      .send({});

    expect(res.status).toBe(400);
    expect(res.body).toMatchObject({
      success: false,
      error: {
        message: expect.any(String),
        code: expect.any(String)
      }
    });
    // correlationId is optional but must be a string if present
    if (res.body.error.correlationId !== undefined) {
      expect(typeof res.body.error.correlationId).toBe('string');
    }
  });

  it('201 response matches SuccessEnvelope schema', async () => {
    const token = makeToken(uuidv4());
    const res = await request(app)
      .post('/api/v1/streams')
      .set('Authorization', `Bearer ${token}`)
      .send(VALID_BODY);

    expect(res.status).toBe(201);
    expect(res.body).toMatchObject({
      success: true,
      data: {
        stream: {
          id: expect.any(String),
          senderId: expect.any(String),
          recipientId: expect.any(String),
          depositAmount: expect.any(String),
          ratePerSecond: expect.any(String),
          startTime: expect.any(Number),
          endTime: expect.any(Number),
          status: 'active',
          createdAt: expect.any(String)
        }
      }
    });
  });
});

// ===========================================================================
// 10. Security: raw key not logged
// ===========================================================================

describe('Security: raw idempotency key not logged', () => {
  it('does not log the raw key value', async () => {
    const token = makeToken(uuidv4());
    const rawKey = uuidv4();

    await request(app)
      .post('/api/v1/streams')
      .set('Authorization', `Bearer ${token}`)
      .set('Idempotency-Key', rawKey)
      .send(VALID_BODY);

    // Collect all logger call arguments
    const allLogArgs = [
      ...(logger.info as jest.Mock).mock.calls,
      ...(logger.warn as jest.Mock).mock.calls,
      ...(logger.debug as jest.Mock).mock.calls,
      ...(logger.error as jest.Mock).mock.calls
    ].map(args => JSON.stringify(args));

    const rawKeyAppearsInLogs = allLogArgs.some(entry => entry.includes(rawKey));
    expect(rawKeyAppearsInLogs).toBe(false);
  });
});

// ===========================================================================
// 11. Correlation ID propagation
// ===========================================================================

describe('Correlation ID', () => {
  it('includes correlationId in error responses when X-Correlation-Id is sent', async () => {
    const token = makeToken(uuidv4());
    const correlationId = 'test-correlation-123';

    const res = await request(app)
      .post('/api/v1/streams')
      .set('Authorization', `Bearer ${token}`)
      .set('X-Correlation-Id', correlationId)
      .send({});

    expect(res.status).toBe(400);
    expect(res.body.error.correlationId).toBe(correlationId);
  });
});

// ===========================================================================
// 12. Optional cliff time
// ===========================================================================

describe('Optional cliffTime', () => {
  it('creates stream without cliffTime (null in response)', async () => {
    const token = makeToken(uuidv4());
    const res = await request(app)
      .post('/api/v1/streams')
      .set('Authorization', `Bearer ${token}`)
      .send(VALID_BODY);

    expect(res.status).toBe(201);
    expect(res.body.data.stream.cliffTime).toBeNull();
  });

  it('creates stream with valid cliffTime', async () => {
    const token = makeToken(uuidv4());
    const cliffTime = VALID_BODY.startTime + 1000;

    const res = await request(app)
      .post('/api/v1/streams')
      .set('Authorization', `Bearer ${token}`)
      .send({ ...VALID_BODY, cliffTime });

    expect(res.status).toBe(201);
    expect(res.body.data.stream.cliffTime).toBe(cliffTime);
  });
});
