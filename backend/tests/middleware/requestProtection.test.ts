/**
 * Unit tests for src/middleware/requestProtection.ts
 *
 * Tests the idempotency middleware in isolation using a minimal Express app.
 */

import express, { Application, Request, Response } from 'express';
import request from 'supertest';
import { v4 as uuidv4 } from 'uuid';
import jwt from 'jsonwebtoken';

// ---------------------------------------------------------------------------
// Mocks
// ---------------------------------------------------------------------------

const cacheStore = new Map<string, unknown>();

jest.mock('../../src/utils/cache', () => ({
  __esModule: true,
  default: {
    get: jest.fn(async (key: string) => cacheStore.get(key) ?? null),
    set: jest.fn(async (key: string, value: unknown) => { cacheStore.set(key, value); }),
    del: jest.fn(async (key: string) => { cacheStore.delete(key); })
  }
}));

jest.mock('../../src/utils/logger', () => ({
  __esModule: true,
  default: { info: jest.fn(), warn: jest.fn(), error: jest.fn(), debug: jest.fn() }
}));

// ---------------------------------------------------------------------------
// Imports (after mocks)
// ---------------------------------------------------------------------------

import { idempotency, IDEMPOTENCY_HEADER } from '../../src/middleware/requestProtection';
import cache from '../../src/utils/cache';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const JWT_SECRET = 'test-secret';

function makeToken(userId: string): string {
  return jwt.sign({ id: userId, email: 'u@test.com', role: 'learner' }, JWT_SECRET);
}

function buildApp(): Application {
  process.env.JWT_SECRET = JWT_SECRET;
  const app = express();
  app.use(express.json());

  // Minimal auth middleware
  app.use((req: Request, _res: Response, next) => {
    const auth = req.headers.authorization;
    if (auth?.startsWith('Bearer ')) {
      try {
        const decoded = jwt.verify(auth.slice(7), JWT_SECRET) as { id: string; email: string; role: string };
        req.user = { id: decoded.id, email: decoded.email, role: decoded.role };
      } catch { /* ignore */ }
    }
    next();
  });

  app.post('/test', idempotency(), (_req: Request, res: Response) => {
    res.status(201).json({ success: true, data: { value: 'created' } });
  });

  return app;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

let app: Application;

beforeAll(() => { app = buildApp(); });
beforeEach(() => { cacheStore.clear(); jest.clearAllMocks(); });

describe('idempotency middleware', () => {
  it('passes through when no Idempotency-Key header is present', async () => {
    const token = makeToken(uuidv4());
    const res = await request(app)
      .post('/test')
      .set('Authorization', `Bearer ${token}`)
      .send({});

    expect(res.status).toBe(201);
    expect(cache.set).not.toHaveBeenCalledWith(
      expect.stringContaining('idempotency'),
      expect.anything(),
      expect.anything()
    );
  });

  it('stores result in cache on first request', async () => {
    const token = makeToken(uuidv4());
    const key = uuidv4();

    const res = await request(app)
      .post('/test')
      .set('Authorization', `Bearer ${token}`)
      .set(IDEMPOTENCY_HEADER, key)
      .send({});

    expect(res.status).toBe(201);
    // Sentinel set + result set = 2 calls
    expect(cache.set).toHaveBeenCalledTimes(2);
  });

  it('returns cached result on replay without calling handler again', async () => {
    const token = makeToken(uuidv4());
    const key = uuidv4();

    const r1 = await request(app)
      .post('/test')
      .set('Authorization', `Bearer ${token}`)
      .set(IDEMPOTENCY_HEADER, key)
      .send({});

    expect(r1.status).toBe(201);
    const setCalls = (cache.set as jest.Mock).mock.calls.length;

    const r2 = await request(app)
      .post('/test')
      .set('Authorization', `Bearer ${token}`)
      .set(IDEMPOTENCY_HEADER, key)
      .send({});

    expect(r2.status).toBe(201);
    expect(r2.body).toEqual(r1.body);
    // No additional cache.set on replay
    expect((cache.set as jest.Mock).mock.calls.length).toBe(setCalls);
  });

  it('returns 409 for in-flight sentinel', async () => {
    const userId = uuidv4();
    const token = makeToken(userId);
    const key = uuidv4();

    cacheStore.set(`idempotency:${userId}:${key}`, '__IN_FLIGHT__');

    const res = await request(app)
      .post('/test')
      .set('Authorization', `Bearer ${token}`)
      .set(IDEMPOTENCY_HEADER, key)
      .send({});

    expect(res.status).toBe(409);
    expect(res.body.error.code).toBe('IDEMPOTENCY_CONFLICT');
  });

  it('returns 400 for malformed key', async () => {
    const token = makeToken(uuidv4());

    const res = await request(app)
      .post('/test')
      .set('Authorization', `Bearer ${token}`)
      .set(IDEMPOTENCY_HEADER, 'not-a-uuid')
      .send({});

    expect(res.status).toBe(400);
    expect(res.body.error.code).toBe('INVALID_IDEMPOTENCY_KEY');
  });

  it('returns 401 when user is not authenticated but key is present', async () => {
    const key = uuidv4();

    const res = await request(app)
      .post('/test')
      .set(IDEMPOTENCY_HEADER, key)
      .send({});

    expect(res.status).toBe(401);
  });

  it('does not cache non-2xx responses', async () => {
    // Build an app that returns 400
    const errorApp = express();
    errorApp.use(express.json());
    errorApp.use((req: Request, _res: Response, next) => {
      const auth = req.headers.authorization;
      if (auth?.startsWith('Bearer ')) {
        try {
          const decoded = jwt.verify(auth.slice(7), JWT_SECRET) as { id: string; email: string; role: string };
          req.user = { id: decoded.id, email: decoded.email, role: decoded.role };
        } catch { /* ignore */ }
      }
      next();
    });
    errorApp.post('/test', idempotency(), (_req: Request, res: Response) => {
      res.status(400).json({ success: false, error: { message: 'bad', code: 'BAD' } });
    });

    const token = makeToken(uuidv4());
    const key = uuidv4();

    await request(errorApp)
      .post('/test')
      .set('Authorization', `Bearer ${token}`)
      .set(IDEMPOTENCY_HEADER, key)
      .send({});

    // Sentinel should have been deleted (del called), result not stored
    expect(cache.del).toHaveBeenCalled();
  });
});
