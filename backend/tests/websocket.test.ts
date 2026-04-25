/**
 * WebSocket Integration Tests
 */

import WebSocket from 'ws';
import http from 'http';
import express from 'express';
import { WebSocketHub } from '../src/ws/hub';
import { StreamChannel } from '../src/websockets/streamChannel';

describe('WebSocket Integration', () => {
  let server: http.Server;
  let wsHub: WebSocketHub;
  let streamChannel: StreamChannel;
  let port: number;
  let baseUrl: string;

  beforeAll((done) => {
    const app = express();
    
    // Create HTTP server
    server = http.createServer(app);
    
    // Initialize WebSocket hub and channel
    wsHub = new WebSocketHub();
    streamChannel = new StreamChannel(wsHub);
    
    // WebSocket server
    const wss = new WebSocket.Server({ 
      server,
      path: '/ws',
      maxPayload: 1024 * 16
    });

    wss.on('connection', (socket, request) => {
      wsHub.addConnection(socket, request);
    });

    // Start server on random port
    server.listen(0, () => {
      const address = server.address();
      if (address && typeof address === 'object') {
        port = address.port;
        baseUrl = `ws://localhost:${port}/ws`;
        done();
      } else {
        done(new Error('Failed to get server address'));
      }
    });
  });

  afterAll((done) => {
    wsHub.cleanup();
    server.close(done);
  });

  describe('Connection Lifecycle', () => {
    test('should establish WebSocket connection', (done) => {
      const ws = new WebSocket(baseUrl);
      
      ws.on('open', () => {
        expect(ws.readyState).toBe(WebSocket.OPEN);
        ws.close();
        done();
      });
      
      ws.on('error', (error) => {
        done(error);
      });
    });

    test('should receive welcome message on connection', (done) => {
      const ws = new WebSocket(baseUrl);
      let welcomeReceived = false;
      
      ws.on('message', (data) => {
        const message = JSON.parse(data.toString());
        if (message.type === 'connected') {
          welcomeReceived = true;
          expect(message.payload).toBeDefined();
          expect(message.payload.clientId).toBeDefined();
          expect(message.payload.maxStreams).toBe(100);
          ws.close();
          done();
        }
      });
      
      ws.on('error', (error) => {
        if (!welcomeReceived) {
          done(error);
        }
      });
    });

    test('should handle connection close', (done) => {
      const ws = new WebSocket(baseUrl);
      
      ws.on('open', () => {
        ws.close();
      });
      
      ws.on('close', () => {
        expect(ws.readyState).toBe(WebSocket.CLOSED);
        done();
      });
      
      ws.on('error', (error) => {
        done(error);
      });
    });
  });

  describe('Subscription Protocol', () => {
    test('should subscribe to stream and receive confirmation', (done) => {
      const ws = new WebSocket(baseUrl);
      const streamId = '123e4567-e89b-12d3-a456-426614174000';
      let subscribed = false;
      
      ws.on('open', () => {
        ws.send(JSON.stringify({
          type: 'subscribe',
          streamId
        }));
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
      
      ws.on('error', (error) => {
        if (!subscribed) {
          done(error);
        }
      });
    });

    test('should unsubscribe from stream and receive confirmation', (done) => {
      const ws = new WebSocket(baseUrl);
      const streamId = '123e4567-e89b-12d3-a456-426614174000';
      let unsubscribed = false;
      
      ws.on('open', () => {
        // First subscribe
        ws.send(JSON.stringify({
          type: 'subscribe',
          streamId
        }));
        
        // Then unsubscribe after a short delay
        setTimeout(() => {
          ws.send(JSON.stringify({
            type: 'unsubscribe',
            streamId
          }));
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
      
      ws.on('error', (error) => {
        if (!unsubscribed) {
          done(error);
        }
      });
    });

    test('should reject unsubscribe when not subscribed', (done) => {
      const ws = new WebSocket(baseUrl);
      const streamId = '123e4567-e89b-12d3-a456-426614174000';
      let errorReceived = false;
      
      ws.on('open', () => {
        // Try to unsubscribe without subscribing first
        ws.send(JSON.stringify({
          type: 'unsubscribe',
          streamId
        }));
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
      
      ws.on('error', (error) => {
        if (!errorReceived) {
          done(error);
        }
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
      
      ws.on('error', (error) => {
        if (!pongReceived) {
          done(error);
        }
      });
    });
  });

  describe('Error Handling', () => {
    test('should reject invalid JSON message', (done) => {
      const ws = new WebSocket(baseUrl);
      let errorReceived = false;
      
      ws.on('open', () => {
        ws.send('invalid json');
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
      
      ws.on('error', (error) => {
        if (!errorReceived) {
          done(error);
        }
      });
    });

    test('should reject message without type field', (done) => {
      const ws = new WebSocket(baseUrl);
      let errorReceived = false;
      
      ws.on('open', () => {
        ws.send(JSON.stringify({
          // Missing type field
          streamId: '123e4567-e89b-12d3-a456-426614174000'
        }));
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
      
      ws.on('error', (error) => {
        if (!errorReceived) {
          done(error);
        }
      });
    });

    test('should reject oversized payload', (done) => {
      const ws = new WebSocket(baseUrl);
      let errorReceived = false;
      
      ws.on('open', () => {
        // Create payload larger than 16KB
        const largePayload = 'x'.repeat(1024 * 17);
        const largeMessage = JSON.stringify({
          type: 'subscribe',
          streamId: '123e4567-e89b-12d3-a456-426614174000',
          payload: largePayload
        });
        
        ws.send(largeMessage);
      });
      
      ws.on('message', (data) => {
        const message = JSON.parse(data.toString());
        
        if (message.type === 'error') {
          expect(message.payload.code).toBe('PAYLOAD_TOO_LARGE');
          errorReceived = true;
          ws.close();
          done();
        }
      });
      
      ws.on('error', (error) => {
        if (!errorReceived) {
          done(error);
        }
      });
    });

    test('should reject invalid stream ID format', (done) => {
      const ws = new WebSocket(baseUrl);
      let errorReceived = false;
      
      ws.on('open', () => {
        ws.send(JSON.stringify({
          type: 'subscribe',
          streamId: 'invalid-uuid-format'
        }));
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
      
      ws.on('error', (error) => {
        if (!errorReceived) {
          done(error);
        }
      });
    });

    test('should reject subscribe without streamId', (done) => {
      const ws = new WebSocket(baseUrl);
      let errorReceived = false;
      
      ws.on('open', () => {
        ws.send(JSON.stringify({
          type: 'subscribe'
          // Missing streamId
        }));
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
      
      ws.on('error', (error) => {
        if (!errorReceived) {
          done(error);
        }
      });
    });
  });

  describe('Broadcast Functionality', () => {
    test('should broadcast stream updates to subscribers', (done) => {
      const ws1 = new WebSocket(baseUrl);
      const ws2 = new WebSocket(baseUrl);
      const streamId = '123e4567-e89b-12d3-a456-426614174000';
      
      let ws1Subscribed = false;
      let ws2Subscribed = false;
      let ws1ReceivedUpdate = false;
      let ws2ReceivedUpdate = false;
      
      // First client subscribes
      ws1.on('open', () => {
        ws1.send(JSON.stringify({
          type: 'subscribe',
          streamId
        }));
      });
      
      ws1.on('message', (data) => {
        const message = JSON.parse(data.toString());
        
        if (message.type === 'subscribed') {
          ws1Subscribed = true;
        }
        
        if (message.type === 'stream_update') {
          expect(message.streamId).toBe(streamId);
          expect(message.type).toBe('stream_update');
          expect(message.payload).toBeDefined();
          ws1ReceivedUpdate = true;
          checkDone();
        }
      });
      
      // Second client subscribes
      ws2.on('open', () => {
        ws2.send(JSON.stringify({
          type: 'subscribe',
          streamId
        }));
      });
      
      ws2.on('message', (data) => {
        const message = JSON.parse(data.toString());
        
        if (message.type === 'subscribed') {
          ws2Subscribed = true;
          
          // Once both are subscribed, trigger broadcast
          if (ws1Subscribed && ws2Subscribed) {
            setTimeout(() => {
              const updateData = { test: 'broadcast' };
              streamChannel.notifyStreamUpdated(streamId, updateData);
            }, 100);
          }
        }
        
        if (message.type === 'stream_update') {
          expect(message.streamId).toBe(streamId);
          expect(message.type).toBe('stream_update');
          expect(message.payload).toBeDefined();
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
      
      ws1.on('error', (error) => {
        done(error);
      });
      
      ws2.on('error', (error) => {
        done(error);
      });
    });

    test('should not broadcast to unsubscribed clients', (done) => {
      const ws = new WebSocket(baseUrl);
      const streamId = '123e4567-e89b-12d3-a456-426614174000';
      let updateReceived = false;
      
      ws.on('open', () => {
        // Don't subscribe, just connect
        
        // Trigger broadcast after short delay
        setTimeout(() => {
          const updateData = { test: 'broadcast' };
          streamChannel.notifyStreamUpdated(streamId, updateData);
          
          // Wait a bit to ensure no message is received
          setTimeout(() => {
            expect(updateReceived).toBe(false);
            ws.close();
            done();
          }, 500);
        }, 100);
      });
      
      ws.on('message', (data) => {
        const message = JSON.parse(data.toString());
        
        if (message.type === 'stream_update') {
          updateReceived = true;
        }
      });
      
      ws.on('error', (error) => {
        done(error);
      });
    });
  });

  describe('Subscription Limits', () => {
    test('should enforce maximum subscriptions per client', (done) => {
      const ws = new WebSocket(baseUrl);
      let subscriptionCount = 0;
      let limitErrorReceived = false;
      
      ws.on('open', () => {
        // Try to subscribe to 101 streams (limit is 100)
        for (let i = 0; i < 101; i++) {
          const streamId = `123e4567-e89b-12d3-a456-426614174${i.toString().padStart(3, '0')}`;
          
          setTimeout(() => {
            ws.send(JSON.stringify({
              type: 'subscribe',
              streamId
            }));
          }, i * 10); // Stagger requests
        }
      });
      
      ws.on('message', (data) => {
        const message = JSON.parse(data.toString());
        
        if (message.type === 'subscribed') {
          subscriptionCount++;
        }
        
        if (message.type === 'error' && message.payload.code === 'SUBSCRIPTION_LIMIT_EXCEEDED') {
          limitErrorReceived = true;
          
          // Verify we hit the limit
          expect(subscriptionCount).toBe(100);
          expect(limitErrorReceived).toBe(true);
          
          ws.close();
          done();
        }
      });
      
      ws.on('error', (error) => {
        done(error);
      });
    });
  });
});