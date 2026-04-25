/**
 * WebSocket Hub
 * 
 * Manages WebSocket connections, subscriptions, and message routing.
 * Enforces subscribe/unsubscribe semantics and per-stream filtering.
 */

import WebSocket from 'ws';
import { v4 as uuidv4 } from 'uuid';
import logger from '../utils/logger';

export interface WebSocketMessage {
  type: 'subscribe' | 'unsubscribe' | 'ping' | 'pong' | 'error';
  streamId?: string;
  payload?: any;
}

export interface ClientInfo {
  id: string;
  socket: WebSocket;
  subscribedStreams: Set<string>;
  lastActivity: number;
  ip: string;
}

export class WebSocketHub {
  private clients: Map<string, ClientInfo> = new Map();
  private streamSubscriptions: Map<string, Set<string>> = new Map(); // streamId -> clientIds
  private readonly MAX_STREAMS_PER_CLIENT = 100;
  private readonly MAX_PAYLOAD_SIZE = 1024 * 16; // 16KB
  private readonly HEARTBEAT_INTERVAL = 30000; // 30 seconds
  private heartbeatInterval: NodeJS.Timeout;

  constructor() {
    this.heartbeatInterval = setInterval(() => this.checkHeartbeats(), this.HEARTBEAT_INTERVAL);
  }

  /**
   * Add a new WebSocket connection to the hub
   */
  addConnection(socket: WebSocket, request: any): string {
    const clientId = uuidv4();
    const ip = request.socket.remoteAddress || 'unknown';
    
    const client: ClientInfo = {
      id: clientId,
      socket,
      subscribedStreams: new Set(),
      lastActivity: Date.now(),
      ip
    };

    this.clients.set(clientId, client);

    socket.on('message', (data) => this.handleMessage(clientId, data));
    socket.on('close', () => this.removeConnection(clientId));
    socket.on('error', (error) => this.handleError(clientId, error));

    // Send welcome message
    this.sendToClient(clientId, {
      type: 'connected',
      payload: { clientId, maxStreams: this.MAX_STREAMS_PER_CLIENT }
    });

    logger.info('WebSocket client connected', { clientId, ip });
    return clientId;
  }

  /**
   * Handle incoming WebSocket messages
   */
  private async handleMessage(clientId: string, data: WebSocket.RawData): Promise<void> {
    const client = this.clients.get(clientId);
    if (!client) return;

    client.lastActivity = Date.now();

    try {
      // Validate payload size
      if (data.length > this.MAX_PAYLOAD_SIZE) {
        this.sendError(clientId, 'Payload too large', 'PAYLOAD_TOO_LARGE');
        return;
      }

      const message = this.parseMessage(data);
      if (!message) {
        this.sendError(clientId, 'Invalid message format', 'INVALID_MESSAGE');
        return;
      }

      await this.processMessage(clientId, message);
    } catch (error) {
      logger.error('Error handling WebSocket message', { clientId, error });
      this.sendError(clientId, 'Internal server error', 'INTERNAL_ERROR');
    }
  }

  /**
   * Parse and validate WebSocket message
   */
  private parseMessage(data: WebSocket.RawData): WebSocketMessage | null {
    try {
      const text = data.toString();
      const parsed = JSON.parse(text);

      // Validate message structure
      if (!parsed.type || typeof parsed.type !== 'string') {
        return null;
      }

      // Validate streamId if present
      if (parsed.streamId && typeof parsed.streamId !== 'string') {
        return null;
      }

      return parsed as WebSocketMessage;
    } catch {
      return null;
    }
  }

  /**
   * Process validated WebSocket message
   */
  private async processMessage(clientId: string, message: WebSocketMessage): Promise<void> {
    const client = this.clients.get(clientId);
    if (!client) return;

    switch (message.type) {
      case 'subscribe':
        await this.handleSubscribe(clientId, message.streamId);
        break;
      case 'unsubscribe':
        await this.handleUnsubscribe(clientId, message.streamId);
        break;
      case 'ping':
        this.sendToClient(clientId, { type: 'pong' });
        break;
      default:
        this.sendError(clientId, `Unknown message type: ${message.type}`, 'UNKNOWN_MESSAGE_TYPE');
    }
  }

  /**
   * Handle subscribe request
   */
  private async handleSubscribe(clientId: string, streamId?: string): Promise<void> {
    const client = this.clients.get(clientId);
    if (!client) return;

    if (!streamId) {
      this.sendError(clientId, 'Stream ID required for subscription', 'STREAM_ID_REQUIRED');
      return;
    }

    // Validate stream ID format (UUID)
    const uuidRegex = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
    if (!uuidRegex.test(streamId)) {
      this.sendError(clientId, 'Invalid stream ID format', 'INVALID_STREAM_ID');
      return;
    }

    // Check subscription limit
    if (client.subscribedStreams.size >= this.MAX_STREAMS_PER_CLIENT) {
      this.sendError(clientId, 'Subscription limit reached', 'SUBSCRIPTION_LIMIT_EXCEEDED');
      return;
    }

    // Add to client's subscriptions
    client.subscribedStreams.add(streamId);

    // Add to stream's subscriber list
    if (!this.streamSubscriptions.has(streamId)) {
      this.streamSubscriptions.set(streamId, new Set());
    }
    this.streamSubscriptions.get(streamId)!.add(clientId);

    logger.info('Client subscribed to stream', { clientId, streamId, ip: client.ip });

    // Send confirmation
    this.sendToClient(clientId, {
      type: 'subscribed',
      streamId,
      payload: { subscribedAt: Date.now() }
    });
  }

  /**
   * Handle unsubscribe request
   */
  private async handleUnsubscribe(clientId: string, streamId?: string): Promise<void> {
    const client = this.clients.get(clientId);
    if (!client) return;

    if (!streamId) {
      this.sendError(clientId, 'Stream ID required for unsubscription', 'STREAM_ID_REQUIRED');
      return;
    }

    // Remove from client's subscriptions
    const wasSubscribed = client.subscribedStreams.delete(streamId);

    // Remove from stream's subscriber list
    const streamSubscribers = this.streamSubscriptions.get(streamId);
    if (streamSubscribers) {
      streamSubscribers.delete(clientId);
      if (streamSubscribers.size === 0) {
        this.streamSubscriptions.delete(streamId);
      }
    }

    if (wasSubscribed) {
      logger.info('Client unsubscribed from stream', { clientId, streamId, ip: client.ip });
      this.sendToClient(clientId, {
        type: 'unsubscribed',
        streamId,
        payload: { unsubscribedAt: Date.now() }
      });
    } else {
      this.sendError(clientId, 'Not subscribed to this stream', 'NOT_SUBSCRIBED');
    }
  }

  /**
   * Broadcast message to all subscribers of a stream
   */
  broadcastToStream(streamId: string, message: any): void {
    const subscribers = this.streamSubscriptions.get(streamId);
    if (!subscribers) return;

    const broadcastMessage = JSON.stringify({
      type: 'stream_update',
      streamId,
      payload: message,
      timestamp: Date.now()
    });

    for (const clientId of subscribers) {
      const client = this.clients.get(clientId);
      if (client && client.socket.readyState === WebSocket.OPEN) {
        try {
          client.socket.send(broadcastMessage);
        } catch (error) {
          logger.error('Error broadcasting to client', { clientId, error });
        }
      }
    }
  }

  /**
   * Send message to specific client
   */
  private sendToClient(clientId: string, message: any): void {
    const client = this.clients.get(clientId);
    if (client && client.socket.readyState === WebSocket.OPEN) {
      try {
        client.socket.send(JSON.stringify(message));
      } catch (error) {
        logger.error('Error sending message to client', { clientId, error });
      }
    }
  }

  /**
   * Send error message to client
   */
  private sendError(clientId: string, message: string, code: string): void {
    this.sendToClient(clientId, {
      type: 'error',
      payload: { message, code }
    });
  }

  /**
   * Remove WebSocket connection
   */
  private removeConnection(clientId: string): void {
    const client = this.clients.get(clientId);
    if (!client) return;

    // Clean up subscriptions
    for (const streamId of client.subscribedStreams) {
      const streamSubscribers = this.streamSubscriptions.get(streamId);
      if (streamSubscribers) {
        streamSubscribers.delete(clientId);
        if (streamSubscribers.size === 0) {
          this.streamSubscriptions.delete(streamId);
        }
      }
    }

    this.clients.delete(clientId);
    logger.info('WebSocket client disconnected', { clientId, ip: client.ip });
  }

  /**
   * Handle WebSocket error
   */
  private handleError(clientId: string, error: Error): void {
    logger.error('WebSocket error', { clientId, error: error.message });
    this.removeConnection(clientId);
  }

  /**
   * Check client heartbeats and remove inactive connections
   */
  private checkHeartbeats(): void {
    const now = Date.now();
    const maxInactiveTime = this.HEARTBEAT_INTERVAL * 3; // 90 seconds

    for (const [clientId, client] of this.clients.entries()) {
      if (now - client.lastActivity > maxInactiveTime) {
        logger.info('Closing inactive WebSocket connection', { clientId, ip: client.ip });
        client.socket.close(1000, 'Connection timeout');
        this.removeConnection(clientId);
      }
    }
  }

  /**
   * Get statistics about the hub
   */
  getStats(): {
    totalClients: number;
    totalSubscriptions: number;
    streamsWithSubscribers: number;
  } {
    let totalSubscriptions = 0;
    for (const client of this.clients.values()) {
      totalSubscriptions += client.subscribedStreams.size;
    }

    return {
      totalClients: this.clients.size,
      totalSubscriptions,
      streamsWithSubscribers: this.streamSubscriptions.size
    };
  }

  /**
   * Clean up resources
   */
  cleanup(): void {
    clearInterval(this.heartbeatInterval);
    
    for (const client of this.clients.values()) {
      client.socket.close(1001, 'Server shutdown');
    }
    
    this.clients.clear();
    this.streamSubscriptions.clear();
  }
}