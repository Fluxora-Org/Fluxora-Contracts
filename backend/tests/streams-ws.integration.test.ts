/**
 * End-to-end: POST /api/v1/streams → WebSocket StreamCreated notification
 *
 * Proves that production wiring (http.Server + attachWebSocketHub +
 * initStreamRealtime + getStreamChannel().notifyStreamCreated) delivers a
 * `created` update to an authenticated client subscribed to the new stream.
 *
 * Stream IDs are generated inside the route via uuid.v4(). Clients must
 * subscribe before create, so this suite forces the next uuid.v4() return
 * value to a predetermined ID after the socket is already subscribed.
 */

import http from 'http';
import express, { Application } from 'express';
import request from 'supertest';
import WebSocket from 'ws';
import jwt from 'jsonwebtoken';

// ---------------------------------------------------------------------------
// Controllable uuid mock (hoisted)
// ---------------------------------------------------------------------------

const uuidState: { next: string | null } = { next: null };

jest.mock('uuid', () => {
  const actual = jest.requireActual<typeof import('uuid')>('uuid');
  return {
    ...actual,
    v4: () => {
      if (uuidState.next !== null) {
        const id = uuidState.next;
        uuidState.next = null;
        return id;
      }
      return actual.v4();
    }
  };
});

// ---------------------------------------------------------------------------
// Hermetic mocks for cache + logger (same pattern as streams.test.ts)
// ---------------------------------------------------------------------------

const cacheStore = new Map<string, unknown>();

jest.mock('../src/utils/cache', () => ({
  __esModule: true,
  default: {
    get: jest.fn(async (key: string) => cacheStore.get(key) ?? null),
    set: jest.fn(async (key: string, value: unknown) => {
      cacheStore.set(key, value);
    }),
    del: jest.fn(async (key: string) => {
      cacheStore.delete(key);
    })
  }
}));

jest.mock('../src/utils/logger', () => ({
  __esModule: true,
  default: {
    info: jest.fn(),
    warn: jest.fn(),
    error: jest.fn(),
    debug: jest.fn()
  }
}));

// ---------------------------------------------------------------------------
// Imports after mocks
// ---------------------------------------------------------------------------

import streamRouter from '../src/routes/streams';
import {
  initStreamRealtime,
  resetStreamRealtime,
  getStreamChannel,
  getWebSocketHub
} from '../src/websockets/runtime';
import { attachWebSocketHub, WEBSOCKET_PATH } from '../src/ws/attach';

// ---------------------------------------------------------------------------
// Constants / helpers
// ---------------------------------------------------------------------------

const JWT_SECRET = 'streams-ws-integration-secret';
const PREDETERMINED_STREAM_ID = '123e4567-e89b-12d3-a456-426614174000';
const SENDER = { id: 'sender-user-1', email: 'sender@test.com', role: 'user' };

const VALID_BODY = {
  recipientId: '987fcdeb-51a2-43d7-b890-123456789abc',
  depositAmount: '1000000',
  ratePerSecond: '100',
  startTime: 1700000000,
  endTime: 1700010000
};

function makeToken(
  payload: { id: string; email: string; role: string } = SENDER
): string {
  return jwt.sign(payload, JWT_SECRET, { expiresIn: '1h' });
}

function buildApp(): Application {
  const app = express();
  app.use(express.json());
  process.env.JWT_SECRET = JWT_SECRET;
  process.env.NODE_ENV = 'test';
  app.use('/api/v1/streams', streamRouter);
  app.use(
    (
      err: Error & { statusCode?: number },
      _req: express.Request,
      res: express.Response,
      _next: express.NextFunction
    ) => {
      const status = err.statusCode ?? 500;
      res.status(status).json({
        success: false,
        error: {
          message: err.message,
          code: status === 401 ? 'UNAUTHORIZED' : 'INTERNAL_ERROR'
        }
      });
    }
  );
  return app;
}

// ---------------------------------------------------------------------------
// Suite
// ---------------------------------------------------------------------------

describe('POST /streams → WebSocket StreamCreated (e2e)', () => {
  let server: http.Server;
  let port: number;
  let baseWsUrl: string;

  beforeAll((done) => {
    process.env.JWT_SECRET = JWT_SECRET;
    process.env.NODE_ENV = 'test';

    const app = buildApp();
    const { hub } = initStreamRealtime();

    // Runtime must expose both instances after init
    expect(getWebSocketHub()).toBe(hub);
    expect(getStreamChannel()).not.toBeNull();

    server = http.createServer(app);
    attachWebSocketHub(server, hub);

    server.listen(0, () => {
      const address = server.address();
      if (address && typeof address === 'object') {
        port = address.port;
        baseWsUrl = `ws://localhost:${port}${WEBSOCKET_PATH}`;
        done();
      } else {
        done(new Error('Failed to bind ephemeral HTTP server'));
      }
    });
  });

  afterAll((done) => {
    resetStreamRealtime();
    expect(getStreamChannel()).toBeNull();
    expect(getWebSocketHub()).toBeNull();
    server.close(done);
  });

  beforeEach(() => {
    cacheStore.clear();
    uuidState.next = null;
    jest.clearAllMocks();
  });

  it(
    'delivers a created notification to a client subscribed to the new stream id',
    async () => {
      const token = makeToken();

      // Attach the message collector before open to avoid racing the welcome frame.
      const ws = new WebSocket(
        `${baseWsUrl}?token=${encodeURIComponent(token)}`
      );
      const inbox: Record<string, unknown>[] = [];
      const waiters: Array<{
        predicate: (msg: Record<string, unknown>) => boolean;
        resolve: (msg: Record<string, unknown>) => void;
      }> = [];

      ws.on('message', (data) => {
        let msg: Record<string, unknown>;
        try {
          msg = JSON.parse(data.toString()) as Record<string, unknown>;
        } catch {
          return;
        }
        inbox.push(msg);
        for (let i = waiters.length - 1; i >= 0; i--) {
          if (waiters[i].predicate(msg)) {
            const waiter = waiters[i];
            waiters.splice(i, 1);
            waiter.resolve(msg);
          }
        }
      });

      const nextMatching = (
        predicate: (msg: Record<string, unknown>) => boolean,
        timeoutMs = 5000
      ): Promise<Record<string, unknown>> => {
        const existing = inbox.find(predicate);
        if (existing) return Promise.resolve(existing);
        return new Promise((resolve, reject) => {
          const timer = setTimeout(() => {
            const idx = waiters.findIndex((w) => w.resolve === resolveWrapped);
            if (idx >= 0) waiters.splice(idx, 1);
            reject(
              new Error(
                `Timed out waiting for matching WS message. Inbox: ${JSON.stringify(inbox)}`
              )
            );
          }, timeoutMs);
          const resolveWrapped = (msg: Record<string, unknown>) => {
            clearTimeout(timer);
            resolve(msg);
          };
          waiters.push({ predicate, resolve: resolveWrapped });
        });
      };

      await new Promise<void>((resolve, reject) => {
        ws.once('open', () => resolve());
        ws.once('error', reject);
      });

      await nextMatching((msg) => msg.type === 'connected');

      // Subscribe to the predetermined stream id before create
      ws.send(
        JSON.stringify({
          type: 'subscribe',
          streamId: PREDETERMINED_STREAM_ID
        })
      );
      await nextMatching((msg) => msg.type === 'subscribed');

      // Force the next uuid.v4() (streamId in createStream) to the predetermined id.
      // Pass X-Correlation-Id so the route does not consume an extra uuid for correlation.
      uuidState.next = PREDETERMINED_STREAM_ID;

      const createdPromise = nextMatching(
        (msg) =>
          msg.type === 'stream_update' &&
          msg.streamId === PREDETERMINED_STREAM_ID &&
          (msg.payload as { type?: string } | undefined)?.type === 'created'
      );

      const res = await request(server)
        .post('/api/v1/streams')
        .set('Authorization', `Bearer ${token}`)
        .set('X-Correlation-Id', 'e2e-correlation-1')
        .send(VALID_BODY);

      expect(res.status).toBe(201);
      expect(res.body.success).toBe(true);
      expect(res.body.data.stream.id).toBe(PREDETERMINED_STREAM_ID);

      const notification = await createdPromise;
      const payload = notification.payload as {
        type: string;
        streamId: string;
        data: {
          id: string;
          senderId: string;
          recipientId: string;
          status: string;
          depositAmount: string;
        };
      };

      expect(payload.type).toBe('created');
      expect(payload.streamId).toBe(PREDETERMINED_STREAM_ID);
      expect(payload.data.id).toBe(PREDETERMINED_STREAM_ID);
      expect(payload.data.senderId).toBe(SENDER.id);
      expect(payload.data.recipientId).toBe(VALID_BODY.recipientId);
      expect(payload.data.status).toBe('active');
      expect(payload.data.depositAmount).toBe(VALID_BODY.depositAmount);

      ws.close();
    },
    15000
  );

  it('still returns 201 when no WebSocket clients are subscribed', async () => {
    const token = makeToken();
    // Do not force a predetermined id; any uuid is fine when nobody is listening.
    const res = await request(server)
      .post('/api/v1/streams')
      .set('Authorization', `Bearer ${token}`)
      .set('X-Correlation-Id', 'e2e-correlation-2')
      .send(VALID_BODY);

    expect(res.status).toBe(201);
    expect(res.body.success).toBe(true);
    expect(res.body.data.stream.id).toMatch(
      /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i
    );
  });

  it('reuses the same hub and channel across initStreamRealtime calls', () => {
    const first = initStreamRealtime();
    const second = initStreamRealtime();
    expect(second.hub).toBe(first.hub);
    expect(second.channel).toBe(first.channel);
  });
});
