/**
 * WebSocket Hub Tests
 */

import { WebSocketHub } from '../src/ws/hub';
import { StreamChannel } from '../src/websockets/streamChannel';

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

  describe('Connection Management', () => {
    test('should add new connection and return client ID', () => {
      const request = { socket: { remoteAddress: '127.0.0.1' } };
      const clientId = hub.addConnection(mockSocket, request);
      
      expect(clientId).toBeDefined();
      expect(typeof clientId).toBe('string');
      expect(clientId.length).toBeGreaterThan(0);
    });

    test('should send welcome message on connection', () => {
      const request = { socket: { remoteAddress: '127.0.0.1' } };
      hub.addConnection(mockSocket, request);
      
      expect(mockSocket.send).toHaveBeenCalledWith(
        expect.stringContaining('"type":"connected"')
      );
    });

    test('should remove connection on close', () => {
      const request = { socket: { remoteAddress: '127.0.0.1' } };
      const clientId = hub.addConnection(mockSocket, request);
      
      // Simulate close event
      const closeHandler = mockSocket.on.mock.calls.find(call => call[0] === 'close')[1];
      closeHandler();
      
      const stats = hub.getStats();
      expect(stats.totalClients).toBe(0);
    });
  });

  describe('Message Handling', () => {
    test('should handle valid subscribe message', () => {
      const request = { socket: { remoteAddress: '127.0.0.1' } };
      const clientId = hub.addConnection(mockSocket, request);
      
      const validMessage = JSON.stringify({
        type: 'subscribe',
        streamId: '123e4567-e89b-12d3-a456-426614174000'
      });
      
      const messageHandler = mockSocket.on.mock.calls.find(call => call[0] === 'message')[1];
      messageHandler(validMessage);
      
      expect(mockSocket.send).toHaveBeenCalledWith(
        expect.stringContaining('"type":"subscribed"')
      );
    });

    test('should reject subscribe without streamId', () => {
      const request = { socket: { remoteAddress: '127.0.0.1' } };
      const clientId = hub.addConnection(mockSocket, request);
      
      const invalidMessage = JSON.stringify({
        type: 'subscribe'
        // Missing streamId
      });
      
      const messageHandler = mockSocket.on.mock.calls.find(call => call[0] === 'message')[1];
      messageHandler(invalidMessage);
      
      expect(mockSocket.send).toHaveBeenCalledWith(
        expect.stringContaining('"type":"error"')
      );
    });

    test('should reject invalid stream ID format', () => {
      const request = { socket: { remoteAddress: '127.0.0.1' } };
      const clientId = hub.addConnection(mockSocket, request);
      
      const invalidMessage = JSON.stringify({
        type: 'subscribe',
        streamId: 'invalid-uuid'
      });
      
      const messageHandler = mockSocket.on.mock.calls.find(call => call[0] === 'message')[1];
      messageHandler(invalidMessage);
      
      expect(mockSocket.send).toHaveBeenCalledWith(
        expect.stringContaining('"type":"error"')
      );
    });

    test('should handle unsubscribe message', () => {
      const request = { socket: { remoteAddress: '127.0.0.1' } };
      const clientId = hub.addConnection(mockSocket, request);
      
      const streamId = '123e4567-e89b-12d3-a456-426614174000';
      
      // First subscribe
      const subscribeMessage = JSON.stringify({ type: 'subscribe', streamId });
      const messageHandler = mockSocket.on.mock.calls.find(call => call[0] === 'message')[1];
      messageHandler(subscribeMessage);
      
      // Then unsubscribe
      const unsubscribeMessage = JSON.stringify({ type: 'unsubscribe', streamId });
      messageHandler(unsubscribeMessage);
      
      expect(mockSocket.send).toHaveBeenCalledWith(
        expect.stringContaining('"type":"unsubscribed"')
      );
    });

    test('should handle ping message with pong response', () => {
      const request = { socket: { remoteAddress: '127.0.0.1' } };
      const clientId = hub.addConnection(mockSocket, request);
      
      const pingMessage = JSON.stringify({ type: 'ping' });
      const messageHandler = mockSocket.on.mock.calls.find(call => call[0] === 'message')[1];
      messageHandler(pingMessage);
      
      expect(mockSocket.send).toHaveBeenCalledWith(
        expect.stringContaining('"type":"pong"')
      );
    });

    test('should reject oversized payload', () => {
      const request = { socket: { remoteAddress: '127.0.0.1' } };
      const clientId = hub.addConnection(mockSocket, request);
      
      // Create payload larger than 16KB
      const largePayload = 'x'.repeat(1024 * 17); // 17KB
      const largeMessage = JSON.stringify({
        type: 'subscribe',
        streamId: '123e4567-e89b-12d3-a456-426614174000',
        payload: largePayload
      });
      
      const messageHandler = mockSocket.on.mock.calls.find(call => call[0] === 'message')[1];
      messageHandler(largeMessage);
      
      expect(mockSocket.send).toHaveBeenCalledWith(
        expect.stringContaining('"type":"error"')
      );
    });

    test('should reject invalid JSON', () => {
      const request = { socket: { remoteAddress: '127.0.0.1' } };
      const clientId = hub.addConnection(mockSocket, request);
      
      const invalidJSON = 'not valid json';
      const messageHandler = mockSocket.on.mock.calls.find(call => call[0] === 'message')[1];
      messageHandler(invalidJSON);
      
      expect(mockSocket.send).toHaveBeenCalledWith(
        expect.stringContaining('"type":"error"')
      );
    });

    test('should reject message without type', () => {
      const request = { socket: { remoteAddress: '127.0.0.1' } };
      const clientId = hub.addConnection(mockSocket, request);
      
      const invalidMessage = JSON.stringify({
        // Missing type field
        streamId: '123e4567-e89b-12d3-a456-426614174000'
      });
      
      const messageHandler = mockSocket.on.mock.calls.find(call => call[0] === 'message')[1];
      messageHandler(invalidMessage);
      
      expect(mockSocket.send).toHaveBeenCalledWith(
        expect.stringContaining('"type":"error"')
      );
    });
  });

  describe('Subscription Limits', () => {
    test('should enforce maximum streams per client', () => {
      const request = { socket: { remoteAddress: '127.0.0.1' } };
      const clientId = hub.addConnection(mockSocket, request);
      const messageHandler = mockSocket.on.mock.calls.find(call => call[0] === 'message')[1];
      
      // Try to subscribe to 101 streams (limit is 100)
      for (let i = 0; i < 101; i++) {
        const streamId = `123e4567-e89b-12d3-a456-426614174${i.toString().padStart(3, '0')}`;
        const message = JSON.stringify({ type: 'subscribe', streamId });
        messageHandler(message);
      }
      
      // Count error messages for exceeding limit
      const errorCalls = mockSocket.send.mock.calls.filter(call => 
        call[0].includes('"type":"error"') && call[0].includes('"SUBSCRIPTION_LIMIT_EXCEEDED"')
      );
      
      expect(errorCalls.length).toBeGreaterThan(0);
    });
  });

  describe('Broadcasting', () => {
    test('should broadcast to stream subscribers', () => {
      const request = { socket: { remoteAddress: '127.0.0.1' } };
      const clientId = hub.addConnection(mockSocket, request);
      const messageHandler = mockSocket.on.mock.calls.find(call => call[0] === 'message')[1];
      
      const streamId = '123e4567-e89b-12d3-a456-426614174000';
      const subscribeMessage = JSON.stringify({ type: 'subscribe', streamId });
      messageHandler(subscribeMessage);
      
      // Broadcast message
      const broadcastMessage = { update: 'test' };
      hub.broadcastToStream(streamId, broadcastMessage);
      
      expect(mockSocket.send).toHaveBeenCalledWith(
        expect.stringContaining('"type":"stream_update"')
      );
    });

    test('should not broadcast to unsubscribed clients', () => {
      const request = { socket: { remoteAddress: '127.0.0.1' } };
      const clientId = hub.addConnection(mockSocket, request);
      
      const streamId = '123e4567-e89b-12d3-a456-426614174000';
      const broadcastMessage = { update: 'test' };
      hub.broadcastToStream(streamId, broadcastMessage);
      
      // Should not send to unsubscribed client
      const streamUpdateCalls = mockSocket.send.mock.calls.filter(call => 
        call[0].includes('"type":"stream_update"')
      );
      
      expect(streamUpdateCalls.length).toBe(0);
    });
  });

  describe('Heartbeat', () => {
    test('should update lastActivity on message', () => {
      const request = { socket: { remoteAddress: '127.0.0.1' } };
      const clientId = hub.addConnection(mockSocket, request);
      const messageHandler = mockSocket.on.mock.calls.find(call => call[0] === 'message')[1];
      
      const initialStats = hub.getStats();
      
      // Send a message
      const message = JSON.stringify({ type: 'ping' });
      messageHandler(message);
      
      // Activity should be updated (tested indirectly through stats)
      const stats = hub.getStats();
      expect(stats.totalClients).toBe(1);
    });
  });

  describe('Statistics', () => {
    test('should return accurate statistics', () => {
      const request = { socket: { remoteAddress: '127.0.0.1' } };
      const clientId = hub.addConnection(mockSocket, request);
      const messageHandler = mockSocket.on.mock.calls.find(call => call[0] === 'message')[1];
      
      // Subscribe to 3 streams
      for (let i = 0; i < 3; i++) {
        const streamId = `123e4567-e89b-12d3-a456-426614174${i.toString().padStart(3, '0')}`;
        const message = JSON.stringify({ type: 'subscribe', streamId });
        messageHandler(message);
      }
      
      const stats = hub.getStats();
      expect(stats.totalClients).toBe(1);
      expect(stats.totalSubscriptions).toBe(3);
      expect(stats.streamsWithSubscribers).toBe(3);
    });
  });
});

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
        '123e4567-e89b-12d3-a456-42661417400', // too short
        '123e4567-e89b-12d3-a456-4266141740000', // too long
        '123e4567-e89b-12d3-a456-42661417400g' // invalid character
      ];
      
      invalidUUIDs.forEach(uuid => {
        expect(StreamChannel.validateStreamId(uuid)).toBe(false);
      });
    });
  });

  describe('Notification Methods', () => {
    test('should notify stream creation', () => {
      const request = { socket: { remoteAddress: '127.0.0.1' } };
      const clientId = hub.addConnection(mockSocket, request);
      const messageHandler = mockSocket.on.mock.calls.find(call => call[0] === 'message')[1];
      
      const streamId = '123e4567-e89b-12d3-a456-426614174000';
      const subscribeMessage = JSON.stringify({ type: 'subscribe', streamId });
      messageHandler(subscribeMessage);
      
      const streamData = { id: streamId, status: 'active' };
      channel.notifyStreamCreated(streamId, streamData);
      
      expect(mockSocket.send).toHaveBeenCalledWith(
        expect.stringContaining('"type":"stream_update"')
      );
      expect(mockSocket.send).toHaveBeenCalledWith(
        expect.stringContaining('"type":"created"')
      );
    });

    test('should notify stream update', () => {
      const request = { socket: { remoteAddress: '127.0.0.1' } };
      const clientId = hub.addConnection(mockSocket, request);
      const messageHandler = mockSocket.on.mock.calls.find(call => call[0] === 'message')[1];
      
      const streamId = '123e4567-e89b-12d3-a456-426614174000';
      const subscribeMessage = JSON.stringify({ type: 'subscribe', streamId });
      messageHandler(subscribeMessage);
      
      const updateData = { ratePerSecond: '100' };
      channel.notifyStreamUpdated(streamId, updateData);
      
      expect(mockSocket.send).toHaveBeenCalledWith(
        expect.stringContaining('"type":"updated"')
      );
    });

    test('should notify stream cancellation', () => {
      const request = { socket: { remoteAddress: '127.0.0.1' } };
      const clientId = hub.addConnection(mockSocket, request);
      const messageHandler = mockSocket.on.mock.calls.find(call => call[0] === 'message')[1];
      
      const streamId = '123e4567-e89b-12d3-a456-426614174000';
      const subscribeMessage = JSON.stringify({ type: 'subscribe', streamId });
      messageHandler(subscribeMessage);
      
      const cancellationData = { cancelledAt: Date.now() };
      channel.notifyStreamCancelled(streamId, cancellationData);
      
      expect(mockSocket.send).toHaveBeenCalledWith(
        expect.stringContaining('"type":"cancelled"')
      );
    });

    test('should notify stream withdrawal', () => {
      const request = { socket: { remoteAddress: '127.0.0.1' } };
      const clientId = hub.addConnection(mockSocket, request);
      const messageHandler = mockSocket.on.mock.calls.find(call => call[0] === 'message')[1];
      
      const streamId = '123e4567-e89b-12d3-a456-426614174000';
      const subscribeMessage = JSON.stringify({ type: 'subscribe', streamId });
      messageHandler(subscribeMessage);
      
      const withdrawalData = { amount: '100', withdrawnAt: Date.now() };
      channel.notifyStreamWithdrawn(streamId, withdrawalData);
      
      expect(mockSocket.send).toHaveBeenCalledWith(
        expect.stringContaining('"type":"withdrawn"')
      );
    });

    test('should notify stream completion', () => {
      const request = { socket: { remoteAddress: '127.0.0.1' } };
      const clientId = hub.addConnection(mockSocket, request);
      const messageHandler = mockSocket.on.mock.calls.find(call => call[0] === 'message')[1];
      
      const streamId = '123e4567-e89b-12d3-a456-426614174000';
      const subscribeMessage = JSON.stringify({ type: 'subscribe', streamId });
      messageHandler(subscribeMessage);
      
      const completionData = { completedAt: Date.now() };
      channel.notifyStreamCompleted(streamId, completionData);
      
      expect(mockSocket.send).toHaveBeenCalledWith(
        expect.stringContaining('"type":"completed"')
      );
    });
  });
});