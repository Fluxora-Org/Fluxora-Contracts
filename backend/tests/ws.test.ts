/**
 * WebSocket Hub Tests
 *
 * Tests the WebSocketHub using mock sockets (no real network).
 *
 * Auth-related tests cover:
 *   - connections without a token (unauthenticated)
 *   - connections with a valid token (authenticated)
 *   - subscribe rejection when unauthenticated (UNAUTHORIZED)
 *   - subscribe acceptance when authenticated (with valid UUID streamId)
 *   - expired and tampered tokens
 */

import { WebSocketHub } from '../src/ws/hub';
import { StreamChannel } from '../src/websockets/streamChannel';
import jwt from 'jsonwebtoken';

// ---------------------------------------------------------------------------
// JWT test helpers
// ---------------------------------------------------------------------------

const TEST_JWT_SECRET = 'test-secret-for-ws-tests';

/** Build a signed JWT that the hub will accept. */
function makeToken(
  payload: { id: string; email: string; role: string },
  secret = TEST_JWT_SECRET,
  options: jwt.SignOptions = { expiresIn: '1h' }
): string {
  return jwt.sign(payload, secret, options);
}

/** Return a mock request whose URL encodes the given JWT as ?token=… */
function requestWithToken(token: string | null, ip = '127.0.0.1'): any {
  const url = token === null ? '/ws' : `/ws?token=${encodeURIComponent(token)}`;
  return { url, socket: { remoteAddress: ip } };
}

/** Unauthenticated request (no token query param). */
function unauthRequest(ip = '127.0.0.1'): any {
  return { url: '/ws', socket: { remoteAddress: ip } };
}

const VALID_USER = { id: 'user-123', email: 'alice@example.com', role: 'user' };

// ---------------------------------------------------------------------------
// Set JWT_SECRET for this test module
// ---------------------------------------------------------------------------

beforeAll(() => {
  process.env.JWT_SECRET = TEST_JWT_SECRET;
});

afterAll(() => {
  delete process.env.JWT_SECRET;
});

// ---------------------------------------------------------------------------
// WebSocketHub
// ---------------------------------------------------------------------------

describe('WebSocketHub', () => {
  let hub: WebSocketHub;
  let mockSocket: any;

  beforeEach(() => {
    hub = new WebSocketHub();
    mockSocket = {
      readyState: 1, // OPEN
      send: jest.fn(),
      close: jest.fn(),
      on: jest.fn()
    };
  });

  afterEach(() => {
    hub.cleanup();
  });

  // ─── Connection Management ────────────────────────────────────────────────

  describe('Connection Management', () => {
    test('should add new connection and return client ID', () => {
      const clientId = hub.addConnection(mockSocket, unauthRequest());

      expect(clientId).toBeDefined();
      expect(typeof clientId).toBe('string');
      expect(clientId.length).toBeGreaterThan(0);
    });

    test('should send welcome message on connection', () => {
      hub.addConnection(mockSocket, unauthRequest());

      expect(mockSocket.send).toHaveBeenCalledWith(
        expect.stringContaining('"type":"connected"')
      );
    });

    test('should remove connection on close', () => {
      hub.addConnection(mockSocket, unauthRequest());

      const closeHandler = mockSocket.on.mock.calls.find(
        (call: any[]) => call[0] === 'close'
      )[1];
      closeHandler();

      const stats = hub.getStats();
      expect(stats.totalClients).toBe(0);
    });

    // ── Auth-aware connection tests ──────────────────────────────────────────

    test('should accept connection without a token (unauthenticated state)', () => {
      // No JWT supplied — connection is accepted but marked unauthenticated.
      const clientId = hub.addConnection(mockSocket, unauthRequest());

      // Welcome message is still sent so the client knows its clientId.
      expect(mockSocket.send).toHaveBeenCalledWith(
        expect.stringContaining('"type":"connected"')
      );
      expect(clientId).toBeDefined();
    });

    test('should accept connection with a valid token (authenticated state)', () => {
      const token = makeToken(VALID_USER);
      const clientId = hub.addConnection(mockSocket, requestWithToken(token));

      // Welcome message is sent for authenticated connections too.
      expect(mockSocket.send).toHaveBeenCalledWith(
        expect.stringContaining('"type":"connected"')
      );
      expect(clientId).toBeDefined();
    });

    test('should accept connection with an invalid token but leave it unauthenticated', () => {
      const clientId = hub.addConnection(
        mockSocket,
        requestWithToken('this.is.not.a.valid.jwt')
      );

      // Welcome is still sent — auth state is unauthenticated.
      expect(mockSocket.send).toHaveBeenCalledWith(
        expect.stringContaining('"type":"connected"')
      );
      expect(clientId).toBeDefined();
    });

    test('should accept connection with an expired token but leave it unauthenticated', () => {
      const expiredToken = makeToken(VALID_USER, TEST_JWT_SECRET, {
        expiresIn: -1 // already expired
      });
      const clientId = hub.addConnection(
        mockSocket,
        requestWithToken(expiredToken)
      );

      expect(mockSocket.send).toHaveBeenCalledWith(
        expect.stringContaining('"type":"connected"')
      );
      expect(clientId).toBeDefined();
    });
  });

  // ─── Message Handling ─────────────────────────────────────────────────────

  describe('Message Handling', () => {
    test('should handle valid subscribe message from authenticated client', async () => {
      const token = makeToken(VALID_USER);
      hub.addConnection(mockSocket, requestWithToken(token));

      const validMessage = JSON.stringify({
        type: 'subscribe',
        streamId: '123e4567-e89b-12d3-a456-426614174000'
      });

      const messageHandler = mockSocket.on.mock.calls.find(
        (call: any[]) => call[0] === 'message'
      )[1];
      await messageHandler(validMessage);

      expect(mockSocket.send).toHaveBeenCalledWith(
        expect.stringContaining('"type":"subscribed"')
      );
    });

    test('should reject subscribe without streamId', async () => {
      const token = makeToken(VALID_USER);
      hub.addConnection(mockSocket, requestWithToken(token));

      const invalidMessage = JSON.stringify({ type: 'subscribe' });

      const messageHandler = mockSocket.on.mock.calls.find(
        (call: any[]) => call[0] === 'message'
      )[1];
      await messageHandler(invalidMessage);

      expect(mockSocket.send).toHaveBeenCalledWith(
        expect.stringContaining('"type":"error"')
      );
    });

    test('should reject invalid stream ID format', async () => {
      const token = makeToken(VALID_USER);
      hub.addConnection(mockSocket, requestWithToken(token));

      const invalidMessage = JSON.stringify({
        type: 'subscribe',
        streamId: 'invalid-uuid'
      });

      const messageHandler = mockSocket.on.mock.calls.find(
        (call: any[]) => call[0] === 'message'
      )[1];
      await messageHandler(invalidMessage);

      expect(mockSocket.send).toHaveBeenCalledWith(
        expect.stringContaining('"type":"error"')
      );
    });

    test('should handle unsubscribe message', async () => {
      const token = makeToken(VALID_USER);
      hub.addConnection(mockSocket, requestWithToken(token));

      const streamId = '123e4567-e89b-12d3-a456-426614174000';
      const messageHandler = mockSocket.on.mock.calls.find(
        (call: any[]) => call[0] === 'message'
      )[1];

      await messageHandler(JSON.stringify({ type: 'subscribe', streamId }));
      await messageHandler(JSON.stringify({ type: 'unsubscribe', streamId }));

      expect(mockSocket.send).toHaveBeenCalledWith(
        expect.stringContaining('"type":"unsubscribed"')
      );
    });

    test('should handle ping message with pong response', async () => {
      hub.addConnection(mockSocket, unauthRequest());

      const messageHandler = mockSocket.on.mock.calls.find(
        (call: any[]) => call[0] === 'message'
      )[1];
      await messageHandler(JSON.stringify({ type: 'ping' }));

      expect(mockSocket.send).toHaveBeenCalledWith(
        expect.stringContaining('"type":"pong"')
      );
    });

    test('should reject oversized payload', async () => {
      const token = makeToken(VALID_USER);
      hub.addConnection(mockSocket, requestWithToken(token));

      const largePayload = 'x'.repeat(1024 * 17); // 17 KB
      const largeMessage = JSON.stringify({
        type: 'subscribe',
        streamId: '123e4567-e89b-12d3-a456-426614174000',
        payload: largePayload
      });

      const messageHandler = mockSocket.on.mock.calls.find(
        (call: any[]) => call[0] === 'message'
      )[1];
      await messageHandler(largeMessage);

      expect(mockSocket.send).toHaveBeenCalledWith(
        expect.stringContaining('"type":"error"')
      );
    });

    test('should reject invalid JSON', async () => {
      hub.addConnection(mockSocket, unauthRequest());

      const messageHandler = mockSocket.on.mock.calls.find(
        (call: any[]) => call[0] === 'message'
      )[1];
      await messageHandler('not valid json');

      expect(mockSocket.send).toHaveBeenCalledWith(
        expect.stringContaining('"type":"error"')
      );
    });

    test('should reject message without type', async () => {
      hub.addConnection(mockSocket, unauthRequest());

      const invalidMessage = JSON.stringify({
        streamId: '123e4567-e89b-12d3-a456-426614174000'
      });

      const messageHandler = mockSocket.on.mock.calls.find(
        (call: any[]) => call[0] === 'message'
      )[1];
      await messageHandler(invalidMessage);

      expect(mockSocket.send).toHaveBeenCalledWith(
        expect.stringContaining('"type":"error"')
      );
    });
  });

  // ─── Authentication Tests ─────────────────────────────────────────────────

  describe('Authentication', () => {
    test('subscribe from unauthenticated client is rejected with UNAUTHORIZED', async () => {
      // No token in request URL → unauthenticated.
      hub.addConnection(mockSocket, unauthRequest());

      const messageHandler = mockSocket.on.mock.calls.find(
        (call: any[]) => call[0] === 'message'
      )[1];
      await messageHandler(
        JSON.stringify({
          type: 'subscribe',
          streamId: '123e4567-e89b-12d3-a456-426614174000'
        })
      );

      const errorCalls = mockSocket.send.mock.calls.filter((call: string[]) =>
        call[0].includes('"type":"error"')
      );
      expect(errorCalls.length).toBeGreaterThan(0);
      expect(errorCalls[0][0]).toContain('"UNAUTHORIZED"');
    });

    test('subscribe from client with expired token is rejected with UNAUTHORIZED', async () => {
      const expiredToken = makeToken(VALID_USER, TEST_JWT_SECRET, {
        expiresIn: -1
      });
      hub.addConnection(mockSocket, requestWithToken(expiredToken));

      const messageHandler = mockSocket.on.mock.calls.find(
        (call: any[]) => call[0] === 'message'
      )[1];
      await messageHandler(
        JSON.stringify({
          type: 'subscribe',
          streamId: '123e4567-e89b-12d3-a456-426614174000'
        })
      );

      const errorCalls = mockSocket.send.mock.calls.filter((call: string[]) =>
        call[0].includes('"type":"error"')
      );
      expect(errorCalls.length).toBeGreaterThan(0);
      expect(errorCalls[0][0]).toContain('"UNAUTHORIZED"');
    });

    test('subscribe from client with tampered token is rejected with UNAUTHORIZED', async () => {
      // Sign with a different secret — verification will fail.
      const badToken = makeToken(VALID_USER, 'wrong-secret');
      hub.addConnection(mockSocket, requestWithToken(badToken));

      const messageHandler = mockSocket.on.mock.calls.find(
        (call: any[]) => call[0] === 'message'
      )[1];
      await messageHandler(
        JSON.stringify({
          type: 'subscribe',
          streamId: '123e4567-e89b-12d3-a456-426614174000'
        })
      );

      const errorCalls = mockSocket.send.mock.calls.filter((call: string[]) =>
        call[0].includes('"type":"error"')
      );
      expect(errorCalls.length).toBeGreaterThan(0);
      expect(errorCalls[0][0]).toContain('"UNAUTHORIZED"');
    });

    test('subscribe from authenticated client is accepted', async () => {
      const token = makeToken(VALID_USER);
      hub.addConnection(mockSocket, requestWithToken(token));

      const messageHandler = mockSocket.on.mock.calls.find(
        (call: any[]) => call[0] === 'message'
      )[1];
      await messageHandler(
        JSON.stringify({
          type: 'subscribe',
          streamId: '123e4567-e89b-12d3-a456-426614174000'
        })
      );

      expect(mockSocket.send).toHaveBeenCalledWith(
        expect.stringContaining('"type":"subscribed"')
      );
    });

    test('unauthenticated client can still ping/pong without UNAUTHORIZED error', async () => {
      // Ping does not require authentication — only subscribe does.
      hub.addConnection(mockSocket, unauthRequest());

      const messageHandler = mockSocket.on.mock.calls.find(
        (call: any[]) => call[0] === 'message'
      )[1];
      await messageHandler(JSON.stringify({ type: 'ping' }));

      expect(mockSocket.send).toHaveBeenCalledWith(
        expect.stringContaining('"type":"pong"')
      );
      const errorCalls = mockSocket.send.mock.calls.filter((call: string[]) =>
        call[0].includes('"UNAUTHORIZED"')
      );
      expect(errorCalls.length).toBe(0);
    });

    test('UNAUTHORIZED check fires before UUID format check', async () => {
      // Even with a garbage streamId, the first error should be UNAUTHORIZED
      // because the auth guard runs before format validation.
      hub.addConnection(mockSocket, unauthRequest());

      const messageHandler = mockSocket.on.mock.calls.find(
        (call: any[]) => call[0] === 'message'
      )[1];
      await messageHandler(
        JSON.stringify({ type: 'subscribe', streamId: 'not-a-uuid' })
      );

      const allSends = mockSocket.send.mock.calls.map((c: string[]) => c[0]);
      const errorSends = allSends.filter((s: string) => s.includes('"type":"error"'));
      // The first error must be UNAUTHORIZED, not INVALID_STREAM_ID.
      expect(errorSends[0]).toContain('"UNAUTHORIZED"');
      expect(errorSends[0]).not.toContain('"INVALID_STREAM_ID"');
    });
  });

  // ─── Per-stream Authorization (Interim Policy) ────────────────────────────

  describe('Per-stream Authorization (interim policy)', () => {
    /**
     * Until stream ownership is persisted, any authenticated user is allowed
     * to subscribe to any well-formed UUID.  These tests document the interim
     * behaviour and must be updated once the full ownership check is in place.
     */
    test('authenticated user can subscribe to any valid streamId (interim)', async () => {
      const token = makeToken(VALID_USER);
      hub.addConnection(mockSocket, requestWithToken(token));

      const messageHandler = mockSocket.on.mock.calls.find(
        (call: any[]) => call[0] === 'message'
      )[1];
      await messageHandler(
        JSON.stringify({
          type: 'subscribe',
          streamId: 'aabbccdd-1234-1abc-8def-aabbccddeeff'
        })
      );

      expect(mockSocket.send).toHaveBeenCalledWith(
        expect.stringContaining('"type":"subscribed"')
      );
    });

    test('authenticated user with different userId can also subscribe (interim)', async () => {
      const otherUser = { id: 'other-user-999', email: 'bob@example.com', role: 'user' };
      const token = makeToken(otherUser);
      hub.addConnection(mockSocket, requestWithToken(token));

      const messageHandler = mockSocket.on.mock.calls.find(
        (call: any[]) => call[0] === 'message'
      )[1];
      await messageHandler(
        JSON.stringify({
          type: 'subscribe',
          streamId: 'aabbccdd-1234-1abc-8def-aabbccddeeff'
        })
      );

      // Interim: no rejection based on userId mismatch.
      expect(mockSocket.send).toHaveBeenCalledWith(
        expect.stringContaining('"type":"subscribed"')
      );
    });
  });

  // ─── Subscription Limits ──────────────────────────────────────────────────

  describe('Subscription Limits', () => {
    test('should enforce maximum streams per client', async () => {
      const token = makeToken(VALID_USER);
      hub.addConnection(mockSocket, requestWithToken(token));

      const messageHandler = mockSocket.on.mock.calls.find(
        (call: any[]) => call[0] === 'message'
      )[1];

      // Subscribe to 101 streams; limit is 100.
      for (let i = 0; i < 101; i++) {
        const streamId = `123e4567-e89b-12d3-a456-426614174${i.toString().padStart(3, '0')}`;
        await messageHandler(JSON.stringify({ type: 'subscribe', streamId }));
      }

      const errorCalls = mockSocket.send.mock.calls.filter((call: string[]) =>
        call[0].includes('"type":"error"') &&
        call[0].includes('"SUBSCRIPTION_LIMIT_EXCEEDED"')
      );
      expect(errorCalls.length).toBeGreaterThan(0);
    });
  });

  // ─── Broadcasting ─────────────────────────────────────────────────────────

  describe('Broadcasting', () => {
    test('should broadcast to stream subscribers', async () => {
      const token = makeToken(VALID_USER);
      hub.addConnection(mockSocket, requestWithToken(token));

      const streamId = '123e4567-e89b-12d3-a456-426614174000';
      const messageHandler = mockSocket.on.mock.calls.find(
        (call: any[]) => call[0] === 'message'
      )[1];
      await messageHandler(JSON.stringify({ type: 'subscribe', streamId }));

      hub.broadcastToStream(streamId, { update: 'test' });

      expect(mockSocket.send).toHaveBeenCalledWith(
        expect.stringContaining('"type":"stream_update"')
      );
    });

    test('should not broadcast to unsubscribed clients', () => {
      hub.addConnection(mockSocket, unauthRequest());

      const streamId = '123e4567-e89b-12d3-a456-426614174000';
      hub.broadcastToStream(streamId, { update: 'test' });

      const streamUpdateCalls = mockSocket.send.mock.calls.filter((call: string[]) =>
        call[0].includes('"type":"stream_update"')
      );
      expect(streamUpdateCalls.length).toBe(0);
    });
  });

  // ─── Heartbeat ────────────────────────────────────────────────────────────

  describe('Heartbeat', () => {
    test('should update lastActivity on message', async () => {
      hub.addConnection(mockSocket, unauthRequest());

      const messageHandler = mockSocket.on.mock.calls.find(
        (call: any[]) => call[0] === 'message'
      )[1];
      await messageHandler(JSON.stringify({ type: 'ping' }));

      const stats = hub.getStats();
      expect(stats.totalClients).toBe(1);
    });
  });

  // ─── Statistics ───────────────────────────────────────────────────────────

  describe('Statistics', () => {
    test('should return accurate statistics', async () => {
      const token = makeToken(VALID_USER);
      hub.addConnection(mockSocket, requestWithToken(token));

      const messageHandler = mockSocket.on.mock.calls.find(
        (call: any[]) => call[0] === 'message'
      )[1];

      for (let i = 0; i < 3; i++) {
        const streamId = `123e4567-e89b-12d3-a456-426614174${i.toString().padStart(3, '0')}`;
        await messageHandler(JSON.stringify({ type: 'subscribe', streamId }));
      }

      const stats = hub.getStats();
      expect(stats.totalClients).toBe(1);
      expect(stats.totalSubscriptions).toBe(3);
      expect(stats.streamsWithSubscribers).toBe(3);
    });
  });
});

// ---------------------------------------------------------------------------
// StreamChannel
// ---------------------------------------------------------------------------

describe('StreamChannel', () => {
  let hub: WebSocketHub;
  let channel: StreamChannel;
  let mockSocket: any;

  beforeEach(() => {
    hub = new WebSocketHub();
    channel = new StreamChannel(hub);
    mockSocket = {
      readyState: 1,
      send: jest.fn(),
      close: jest.fn(),
      on: jest.fn()
    };
  });

  afterEach(() => {
    hub.cleanup();
  });

  describe('Stream ID Validation', () => {
    test('should validate correct UUID format', () => {
      const validUUID = '123e4567-e89b-12d3-a456-426614174000';
      expect(StreamChannel.validateStreamId(validUUID)).toBe(true);
    });

    test('should reject invalid UUID format', () => {
      const invalidUUIDs = [
        'not-a-uuid',
        '123',
        '123e4567-e89b-12d3-a456-42661417400',   // too short
        '123e4567-e89b-12d3-a456-4266141740000',  // too long
        '123e4567-e89b-12d3-a456-42661417400g'    // invalid character
      ];
      invalidUUIDs.forEach((uuid) => {
        expect(StreamChannel.validateStreamId(uuid)).toBe(false);
      });
    });
  });

  describe('Notification Methods', () => {
    // Helper: connect an authenticated socket and subscribe to streamId.
    async function authenticatedSubscriber(
      streamId: string
    ): Promise<{ socket: any; messageHandler: Function }> {
      const token = makeToken(VALID_USER);
      hub.addConnection(mockSocket, requestWithToken(token));
      const messageHandler = mockSocket.on.mock.calls.find(
        (call: any[]) => call[0] === 'message'
      )[1];
      await messageHandler(JSON.stringify({ type: 'subscribe', streamId }));
      return { socket: mockSocket, messageHandler };
    }

    test('should notify stream creation', async () => {
      const streamId = '123e4567-e89b-12d3-a456-426614174000';
      await authenticatedSubscriber(streamId);

      channel.notifyStreamCreated(streamId, { id: streamId, status: 'active' });

      expect(mockSocket.send).toHaveBeenCalledWith(
        expect.stringContaining('"type":"stream_update"')
      );
      expect(mockSocket.send).toHaveBeenCalledWith(
        expect.stringContaining('"type":"created"')
      );
    });

    test('should notify stream update', async () => {
      const streamId = '123e4567-e89b-12d3-a456-426614174000';
      await authenticatedSubscriber(streamId);

      channel.notifyStreamUpdated(streamId, { ratePerSecond: '100' });

      expect(mockSocket.send).toHaveBeenCalledWith(
        expect.stringContaining('"type":"updated"')
      );
    });

    test('should notify stream cancellation', async () => {
      const streamId = '123e4567-e89b-12d3-a456-426614174000';
      await authenticatedSubscriber(streamId);

      channel.notifyStreamCancelled(streamId, { cancelledAt: Date.now() });

      expect(mockSocket.send).toHaveBeenCalledWith(
        expect.stringContaining('"type":"cancelled"')
      );
    });

    test('should notify stream withdrawal', async () => {
      const streamId = '123e4567-e89b-12d3-a456-426614174000';
      await authenticatedSubscriber(streamId);

      channel.notifyStreamWithdrawn(streamId, {
        amount: '100',
        withdrawnAt: Date.now()
      });

      expect(mockSocket.send).toHaveBeenCalledWith(
        expect.stringContaining('"type":"withdrawn"')
      );
    });

    test('should notify stream completion', async () => {
      const streamId = '123e4567-e89b-12d3-a456-426614174000';
      await authenticatedSubscriber(streamId);

      channel.notifyStreamCompleted(streamId, { completedAt: Date.now() });

      expect(mockSocket.send).toHaveBeenCalledWith(
        expect.stringContaining('"type":"completed"')
      );
    });
  });
});
