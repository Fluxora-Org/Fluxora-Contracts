/**
 * WebSocket Integration Tests
 *
 * Exercises the WebSocket layer against a real HTTP server so we can verify
 * end-to-end upgrade handling, JWT extraction from query params, and
 * subscription authorization flows.
 *
 * Auth-related tests cover:
 *   - connecting without a token and receiving UNAUTHORIZED on subscribe
 *   - connecting with a valid token and successfully subscribing
 *   - connecting with an expired / invalid token and receiving UNAUTHORIZED
 */

import WebSocket from 'ws';
import http from 'http';
import express from 'express';
import jwt from 'jsonwebtoken';
import { WebSocketHub } from '../src/ws/hub';
import { StreamChannel } from '../src/websockets/streamChannel';

// ---------------------------------------------------------------------------
// JWT test helpers
// ---------------------------------------------------------------------------

const TEST_JWT_SECRET = 'integration-test-jwt-secret';

function makeToken(
  payload: { id: string; email: string; role: string },
  secret = TEST_JWT_SECRET,
  options: jwt.SignOptions = { expiresIn: '1h' }
): string {
  return jwt.sign(payload, secret, options);
}

const VALID_USER = { id: 'int-user-1', email: 'charlie@example.com', role: 'user' };

// ---------------------------------------------------------------------------
// Integration suite
// ---------------------------------------------------------------------------

describe('WebSocket Integration', () => {
  let server: http.Server;
  let wsHub: WebSocketHub;
  let streamChannel: StreamChannel;
  let port: number;
  /** Base URL without token — append ?token=... as needed. */
  let baseUrl: string;
  /** Convenience URL for an authenticated connection. */
  let authUrl: string;

  beforeAll((done) => {
    // Set JWT_SECRET before the hub is created so addConnection can verify.
    process.env.JWT_SECRET = TEST_JWT_SECRET;

    const app = express();

    server = http.createServer(app);

    wsHub = new WebSocketHub();
    streamChannel = new StreamChannel(wsHub);

    const wss = new WebSocket.Server({
      server,
      path: '/ws',
      maxPayload: 1024 * 16
    });

    wss.on('connection', (socket, request) => {
      wsHub.addConnection(socket, request);
    });

    server.listen(0, () => {
      const address = server.address();
      if (address && typeof address === 'object') {
        port = address.port;
        baseUrl = `ws://localhost:${port}/ws`;
        const validToken = makeToken(VALID_USER);
        authUrl = `${baseUrl}?token=${encodeURIComponent(validToken)}`;
        done();
      } else {
        done(new Error('Failed to get server address'));
      }
    });
  });

  afterAll((done) => {
    delete process.env.JWT_SECRET;
    wsHub.cleanup();
    server.close(done);
  });

  // ─── Connection Lifecycle ─────────────────────────────────────────────────

  describe('Connection Lifecycle', () => {
    test('should establish WebSocket connection (no token)', (done) => {
      const ws = new WebSocket(baseUrl);

      ws.on('open', () => {
        expect(ws.readyState).toBe(WebSocket.OPEN);
        ws.close();
        done();
      });

      ws.on('error', done);
    });

    test('should receive welcome message on connection', (done) => {
      const ws = new WebSocket(baseUrl);
      let welcomed = false;

      ws.on('message', (data) => {
        const message = JSON.parse(data.toString());
        if (message.type === 'connected') {
          welcomed = true;
          expect(message.payload).toBeDefined();
          expect(message.payload.clientId).toBeDefined();
          expect(message.payload.maxStreams).toBe(100);
          ws.close();
          done();
        }
      });

      ws.on('error', (err) => {
        if (!welcomed) done(err);
      });
    });

    test('should handle connection close', (done) => {
      const ws = new WebSocket(baseUrl);

      ws.on('open', () => ws.close());

      ws.on('close', () => {
        expect(ws.readyState).toBe(WebSocket.CLOSED);
        done();
      });

      ws.on('error', done);
    });
  });

  // ─── Authentication ───────────────────────────────────────────────────────

  describe('Authentication', () => {
    test('subscribe without a token is rejected with UNAUTHORIZED', (done) => {
      const ws = new WebSocket(baseUrl); // no ?token=
      let errorReceived = false;

      ws.on('open', () => {
        ws.send(
          JSON.stringify({
            type: 'subscribe',
            streamId: '123e4567-e89b-12d3-a456-426614174000'
          })
        );
      });

      ws.on('message', (data) => {
        const message = JSON.parse(data.toString());
        if (message.type === 'connected') return; // ignore welcome

        expect(message.type).toBe('error');
        expect(message.payload.code).toBe('UNAUTHORIZED');
        errorReceived = true;
        ws.close();
        done();
      });

      ws.on('error', (err) => {
        if (!errorReceived) done(err);
      });
    });

    test('subscribe with a valid token succeeds', (done) => {
      const token = makeToken(VALID_USER);
      const ws = new WebSocket(`${baseUrl}?token=${encodeURIComponent(token)}`);
      let subscribed = false;

      ws.on('open', () => {
        ws.send(
          JSON.stringify({
            type: 'subscribe',
            streamId: '123e4567-e89b-12d3-a456-426614174000'
          })
        );
      });

      ws.on('message', (data) => {
        const message = JSON.parse(data.toString());
        if (message.type === 'connected') return;

        expect(message.type).toBe('subscribed');
        subscribed = true;
        ws.close();
        done();
      });

      ws.on('error', (err) => {
        if (!subscribed) done(err);
      });
    });

    test('subscribe with an expired token is rejected with UNAUTHORIZED', (done) => {
      const expiredToken = makeToken(VALID_USER, TEST_JWT_SECRET, {
        expiresIn: -1
      });
      const ws = new WebSocket(
        `${baseUrl}?token=${encodeURIComponent(expiredToken)}`
      );
      let errorReceived = false;

      ws.on('open', () => {
        ws.send(
          JSON.stringify({
            type: 'subscribe',
            streamId: '123e4567-e89b-12d3-a456-426614174000'
          })
        );
      });

      ws.on('message', (data) => {
        const message = JSON.parse(data.toString());
        if (message.type === 'connected') return;

        expect(message.type).toBe('error');
        expect(message.payload.code).toBe('UNAUTHORIZED');
        errorReceived = true;
        ws.close();
        done();
      });

      ws.on('error', (err) => {
        if (!errorReceived) done(err);
      });
    });

    test('subscribe with an invalid (non-JWT) token is rejected with UNAUTHORIZED', (done) => {
      const ws = new WebSocket(`${baseUrl}?token=this.is.garbage`);
      let errorReceived = false;

      ws.on('open', () => {
        ws.send(
          JSON.stringify({
            type: 'subscribe',
            streamId: '123e4567-e89b-12d3-a456-426614174000'
          })
        );
      });

      ws.on('message', (data) => {
        const message = JSON.parse(data.toString());
        if (message.type === 'connected') return;

        expect(message.type).toBe('error');
        expect(message.payload.code).toBe('UNAUTHORIZED');
        errorReceived = true;
        ws.close();
        done();
      });

      ws.on('error', (err) => {
        if (!errorReceived) done(err);
      });
    });

    test('subscribe with token signed with wrong secret is rejected with UNAUTHORIZED', (done) => {
      const wrongToken = makeToken(VALID_USER, 'completely-wrong-secret');
      const ws = new WebSocket(
        `${baseUrl}?token=${encodeURIComponent(wrongToken)}`
      );
      let errorReceived = false;

      ws.on('open', () => {
        ws.send(
          JSON.stringify({
            type: 'subscribe',
            streamId: '123e4567-e89b-12d3-a456-426614174000'
          })
        );
      });

      ws.on('message', (data) => {
        const message = JSON.parse(data.toString());
        if (message.type === 'connected') return;

        expect(message.type).toBe('error');
        expect(message.payload.code).toBe('UNAUTHORIZED');
        errorReceived = true;
        ws.close();
        done();
      });

      ws.on('error', (err) => {
        if (!errorReceived) done(err);
      });
    });

    test('unauthenticated client can still ping without UNAUTHORIZED', (done) => {
      const ws = new WebSocket(baseUrl); // no token
      let pongReceived = false;

      ws.on('open', () => {
        ws.send(JSON.stringify({ type: 'ping' }));
      });

      ws.on('message', (data) => {
        const message = JSON.parse(data.toString());
        if (message.type === 'connected') return;

        expect(message.type).toBe('pong');
        pongReceived = true;
        ws.close();
        done();
      });

      ws.on('error', (err) => {
        if (!pongReceived) done(err);
      });
    });

    test('welcome message is sent regardless of auth state', (done) => {
      // Specifically verify that an unauthenticated connection still gets the
      // welcome message (so the client can read its clientId).
      const ws = new WebSocket(baseUrl);
      let welcomed = false;

      ws.on('message', (data) => {
        const message = JSON.parse(data.toString());
        if (message.type === 'connected') {
          welcomed = true;
          expect(message.payload.clientId).toBeDefined();
          ws.close();
          done();
        }
      });

      ws.on('error', (err) => {
        if (!welcomed) done(err);
      });
    });
  });

  // ─── Per-stream Authorization (Interim Policy) ────────────────────────────

  describe('Per-stream Authorization (interim policy)', () => {
    /**
     * Until stream ownership data is persisted and exposed to the hub, any
     * authenticated user can subscribe to any well-formed UUID.  These tests
     * document the current behaviour and the TODO for full enforcement.
     */
    test('authenticated user can subscribe to any stream UUID (interim)', (done) => {
      const token = makeToken(VALID_USER);
      const ws = new WebSocket(`${baseUrl}?token=${encodeURIComponent(token)}`);
      let subscribed = false;

      ws.on('open', () => {
        ws.send(
          JSON.stringify({
            type: 'subscribe',
            // A UUID that would not belong to this user under full auth.
            streamId: 'aabbccdd-1234-1abc-8def-aabbccddeeff'
          })
        );
      });

      ws.on('message', (data) => {
        const message = JSON.parse(data.toString());
        if (message.type === 'connected') return;

        // Interim: no UNAUTHORIZED for mismatched stream ownership.
        expect(message.type).toBe('subscribed');
        subscribed = true;
        ws.close();
        done();
      });

      ws.on('error', (err) => {
        if (!subscribed) done(err);
      });
    });
  });

  // ─── Subscription Protocol ────────────────────────────────────────────────

  describe('Subscription Protocol', () => {
    test('should subscribe to stream and receive confirmation', (done) => {
      const token = makeToken(VALID_USER);
      const ws = new WebSocket(`${baseUrl}?token=${encodeURIComponent(token)}`);
      const streamId = '123e4567-e89b-12d3-a456-426614174000';
      let subscribed = false;

      ws.on('open', () => {
        ws.send(JSON.stringify({ type: 'subscribe', streamId }));
      });

      ws.on('message', (data) => {
        const message = JSON.parse(data.toString());
        if (message.type === 'subscribed') {
          expect(message.streamId).toBe(streamId);
          expect(message.payload.subscribedAt).toBeDefined();
          subscribed = true;
          ws.close();
          done();
        }
      });

      ws.on('error', (err) => {
        if (!subscribed) done(err);
      });
    });

    test('should unsubscribe from stream and receive confirmation', (done) => {
      const token = makeToken(VALID_USER);
      const ws = new WebSocket(`${baseUrl}?token=${encodeURIComponent(token)}`);
      const streamId = '123e4567-e89b-12d3-a456-426614174000';
      let unsubscribed = false;

      ws.on('open', () => {
        ws.send(JSON.stringify({ type: 'subscribe', streamId }));
        setTimeout(() => {
          ws.send(JSON.stringify({ type: 'unsubscribe', streamId }));
        }, 100);
      });

      ws.on('message', (data) => {
        const message = JSON.parse(data.toString());
        if (message.type === 'unsubscribed') {
          expect(message.streamId).toBe(streamId);
          expect(message.payload.unsubscribedAt).toBeDefined();
          unsubscribed = true;
          ws.close();
          done();
        }
      });

      ws.on('error', (err) => {
        if (!unsubscribed) done(err);
      });
    });

    test('should reject unsubscribe when not subscribed', (done) => {
      // Unsubscribe does not require auth — this tests the NOT_SUBSCRIBED path.
      const token = makeToken(VALID_USER);
      const ws = new WebSocket(`${baseUrl}?token=${encodeURIComponent(token)}`);
      const streamId = '123e4567-e89b-12d3-a456-426614174000';
      let errorReceived = false;

      ws.on('open', () => {
        ws.send(JSON.stringify({ type: 'unsubscribe', streamId }));
      });

      ws.on('message', (data) => {
        const message = JSON.parse(data.toString());
        if (message.type === 'error') {
          expect(message.payload.code).toBe('NOT_SUBSCRIBED');
          errorReceived = true;
          ws.close();
          done();
        }
      });

      ws.on('error', (err) => {
        if (!errorReceived) done(err);
      });
    });

    test('should handle ping/pong heartbeat', (done) => {
      const ws = new WebSocket(baseUrl);
      let pongReceived = false;

      ws.on('open', () => {
        ws.send(JSON.stringify({ type: 'ping' }));
      });

      ws.on('message', (data) => {
        const message = JSON.parse(data.toString());
        if (message.type === 'pong') {
          pongReceived = true;
          ws.close();
          done();
        }
      });

      ws.on('error', (err) => {
        if (!pongReceived) done(err);
      });
    });
  });

  // ─── Error Handling ───────────────────────────────────────────────────────

  describe('Error Handling', () => {
    test('should reject invalid JSON message', (done) => {
      const ws = new WebSocket(baseUrl);
      let errorReceived = false;

      ws.on('open', () => ws.send('invalid json'));

      ws.on('message', (data) => {
        const message = JSON.parse(data.toString());
        if (message.type === 'error') {
          expect(message.payload.code).toBe('INVALID_MESSAGE');
          errorReceived = true;
          ws.close();
          done();
        }
      });

      ws.on('error', (err) => {
        if (!errorReceived) done(err);
      });
    });

    test('should reject message without type field', (done) => {
      const ws = new WebSocket(baseUrl);
      let errorReceived = false;

      ws.on('open', () => {
        ws.send(
          JSON.stringify({ streamId: '123e4567-e89b-12d3-a456-426614174000' })
        );
      });

      ws.on('message', (data) => {
        const message = JSON.parse(data.toString());
        if (message.type === 'error') {
          expect(message.payload.code).toBe('INVALID_MESSAGE');
          errorReceived = true;
          ws.close();
          done();
        }
      });

      ws.on('error', (err) => {
        if (!errorReceived) done(err);
      });
    });

    test('should reject oversized payload', (done) => {
      // The WS server has maxPayload: 1024 * 16.  When a frame exceeds that
      // limit, the ws library closes the connection with code 1009 (message too
      // big) before the message ever reaches the hub handler.  In this case
      // the client receives a close event rather than an error message.
      const ws = new WebSocket(baseUrl);
      let rejected = false;

      ws.on('open', () => {
        const largePayload = 'x'.repeat(1024 * 17); // 17 KB
        ws.send(
          JSON.stringify({
            type: 'subscribe',
            streamId: '123e4567-e89b-12d3-a456-426614174000',
            payload: largePayload
          })
        );
      });

      ws.on('message', (data) => {
        const message = JSON.parse(data.toString());
        // If the hub gets the message (payload <= maxPayload), it returns an error.
        if (message.type === 'error') {
          expect(message.payload.code).toBe('PAYLOAD_TOO_LARGE');
          rejected = true;
          ws.close();
          done();
        }
      });

      // Protocol-level rejection: ws closes the connection with 1009.
      ws.on('close', (code) => {
        if (!rejected) {
          // 1009 = message too big — that is a valid rejection.
          expect([1009, 1006]).toContain(code);
          done();
        }
      });

      ws.on('error', (err) => {
        if (!rejected) {
          // Some environments surface this as an error event instead.
          expect(err).toBeDefined();
          done();
        }
      });
    });

    test('should reject invalid stream ID format', (done) => {
      const token = makeToken(VALID_USER);
      const ws = new WebSocket(`${baseUrl}?token=${encodeURIComponent(token)}`);
      let errorReceived = false;

      ws.on('open', () => {
        ws.send(
          JSON.stringify({ type: 'subscribe', streamId: 'invalid-uuid-format' })
        );
      });

      ws.on('message', (data) => {
        const message = JSON.parse(data.toString());
        if (message.type === 'error') {
          expect(message.payload.code).toBe('INVALID_STREAM_ID');
          errorReceived = true;
          ws.close();
          done();
        }
      });

      ws.on('error', (err) => {
        if (!errorReceived) done(err);
      });
    });

    test('should reject subscribe without streamId', (done) => {
      const token = makeToken(VALID_USER);
      const ws = new WebSocket(`${baseUrl}?token=${encodeURIComponent(token)}`);
      let errorReceived = false;

      ws.on('open', () => {
        ws.send(JSON.stringify({ type: 'subscribe' }));
      });

      ws.on('message', (data) => {
        const message = JSON.parse(data.toString());
        if (message.type === 'error') {
          expect(message.payload.code).toBe('STREAM_ID_REQUIRED');
          errorReceived = true;
          ws.close();
          done();
        }
      });

      ws.on('error', (err) => {
        if (!errorReceived) done(err);
      });
    });
  });

  // ─── Broadcast Functionality ──────────────────────────────────────────────

  describe('Broadcast Functionality', () => {
    test('should broadcast stream updates to subscribers', (done) => {
      const token1 = makeToken(VALID_USER);
      const token2 = makeToken({
        id: 'int-user-2',
        email: 'dave@example.com',
        role: 'user'
      });

      const ws1 = new WebSocket(`${baseUrl}?token=${encodeURIComponent(token1)}`);
      const ws2 = new WebSocket(`${baseUrl}?token=${encodeURIComponent(token2)}`);
      const streamId = '123e4567-e89b-12d3-a456-426614174000';

      let ws1Subscribed = false;
      let ws2Subscribed = false;
      let ws1ReceivedUpdate = false;
      let ws2ReceivedUpdate = false;

      ws1.on('open', () => {
        ws1.send(JSON.stringify({ type: 'subscribe', streamId }));
      });

      ws1.on('message', (data) => {
        const message = JSON.parse(data.toString());
        if (message.type === 'subscribed') ws1Subscribed = true;
        if (message.type === 'stream_update') {
          expect(message.streamId).toBe(streamId);
          ws1ReceivedUpdate = true;
          checkDone();
        }
      });

      ws2.on('open', () => {
        ws2.send(JSON.stringify({ type: 'subscribe', streamId }));
      });

      ws2.on('message', (data) => {
        const message = JSON.parse(data.toString());
        if (message.type === 'subscribed') {
          ws2Subscribed = true;
          if (ws1Subscribed && ws2Subscribed) {
            setTimeout(() => {
              streamChannel.notifyStreamUpdated(streamId, { test: 'broadcast' });
            }, 100);
          }
        }
        if (message.type === 'stream_update') {
          expect(message.streamId).toBe(streamId);
          ws2ReceivedUpdate = true;
          checkDone();
        }
      });

      function checkDone() {
        if (ws1ReceivedUpdate && ws2ReceivedUpdate) {
          ws1.close();
          ws2.close();
          done();
        }
      }

      ws1.on('error', done);
      ws2.on('error', done);
    });

    test('should not broadcast to unsubscribed clients', (done) => {
      const ws = new WebSocket(baseUrl); // not subscribing
      const streamId = '123e4567-e89b-12d3-a456-426614174000';
      let updateReceived = false;

      ws.on('open', () => {
        setTimeout(() => {
          streamChannel.notifyStreamUpdated(streamId, { test: 'broadcast' });
          setTimeout(() => {
            expect(updateReceived).toBe(false);
            ws.close();
            done();
          }, 300);
        }, 100);
      });

      ws.on('message', (data) => {
        const message = JSON.parse(data.toString());
        if (message.type === 'stream_update') updateReceived = true;
      });

      ws.on('error', done);
    });
  });

  // ─── Subscription Limits ──────────────────────────────────────────────────

  describe('Subscription Limits', () => {
    test('should enforce maximum subscriptions per client', (done) => {
      const token = makeToken(VALID_USER);
      const ws = new WebSocket(`${baseUrl}?token=${encodeURIComponent(token)}`);
      let subscriptionCount = 0;
      let limitErrorReceived = false;

      ws.on('open', () => {
        for (let i = 0; i < 101; i++) {
          const streamId = `123e4567-e89b-12d3-a456-426614174${i
            .toString()
            .padStart(3, '0')}`;
          setTimeout(() => {
            ws.send(JSON.stringify({ type: 'subscribe', streamId }));
          }, i * 10);
        }
      });

      ws.on('message', (data) => {
        const message = JSON.parse(data.toString());
        if (message.type === 'subscribed') subscriptionCount++;
        if (
          message.type === 'error' &&
          message.payload.code === 'SUBSCRIPTION_LIMIT_EXCEEDED'
        ) {
          limitErrorReceived = true;
          expect(subscriptionCount).toBe(100);
          ws.close();
          done();
        }
      });

      ws.on('error', done);
    });
  });
});
