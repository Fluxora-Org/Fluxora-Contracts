/**
 * WebSocket Hub
 *
 * Manages WebSocket connections, subscriptions, and message routing.
 * Enforces subscribe/unsubscribe semantics and per-stream filtering.
 *
 * ── Authentication ───────────────────────────────────────────────────────────
 * Every connection must prove identity via a JWT before it can subscribe to
 * any stream.  The token is read from the `token` query-string parameter of
 * the upgrade request (e.g. `ws://host/ws?token=<jwt>`).  The same
 * jsonwebtoken.verify call and JWT_SECRET environment variable used by the
 * REST authenticate() middleware are used here so the security model is
 * identical.
 *
 * If the token is absent or invalid the client still receives a welcome
 * message (so it knows the transport is up), but any subsequent `subscribe`
 * message is rejected with error code UNAUTHORIZED until a valid token has
 * been supplied.
 *
 * ── Per-stream Authorization ─────────────────────────────────────────────────
 * INTERIM POLICY (2026-07-23): Stream ownership (sender / recipient) is
 * currently stored only in memory inside the POST /api/v1/streams route
 * handler and is not accessible to this process.  Until a shared persistence
 * layer (database table, cache, or in-process registry) exposes a
 * `getStreamOwners(streamId)` call, the hub cannot enforce the sender-or-
 * recipient rule.
 *
 * The authorizeStreamAccess() method below is the single integration point
 * that will enforce that rule once the data is available.  It currently
 * allows any authenticated user to subscribe to any UUID-shaped streamId and
 * logs a warning so the gap is visible in production logs.  Replace the body
 * of that method with a real lookup when the service layer is ready.
 *
 * No other part of this file needs to change to implement full authorization.
 */

import WebSocket from 'ws';
import { v4 as uuidv4 } from 'uuid';
import jwt from 'jsonwebtoken';
import { URL } from 'url';
import logger from '../utils/logger';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface WebSocketMessage {
  type: 'subscribe' | 'unsubscribe' | 'ping' | 'pong' | 'error';
  streamId?: string;
  payload?: any;
}

/**
 * Per-connection state.
 *
 * `authenticated` is true when a valid JWT has been verified for this socket.
 * `userId` is populated from the `id` field in the JWT payload.
 * `userEmail` and `userRole` are carried for future use (e.g. admin bypass).
 */
export interface ClientInfo {
  id: string;
  socket: WebSocket;
  subscribedStreams: Set<string>;
  lastActivity: number;
  ip: string;
  /** True once a valid JWT has been verified for this connection. */
  authenticated: boolean;
  /** User id from the JWT `id` claim; undefined until authenticated. */
  userId?: string;
  /** Email from the JWT payload; undefined until authenticated. */
  userEmail?: string;
  /** Role from the JWT payload; undefined until authenticated. */
  userRole?: string;
}

interface DecodedToken {
  id: string;
  email: string;
  role: string;
  iat: number;
  exp: number;
}

// ---------------------------------------------------------------------------
// WebSocketHub
// ---------------------------------------------------------------------------

export class WebSocketHub {
  private clients: Map<string, ClientInfo> = new Map();
  private streamSubscriptions: Map<string, Set<string>> = new Map(); // streamId -> clientIds
  private readonly MAX_STREAMS_PER_CLIENT = 100;
  private readonly MAX_PAYLOAD_SIZE = 1024 * 16; // 16 KB
  private readonly HEARTBEAT_INTERVAL = 30000;   // 30 seconds
  private heartbeatInterval: NodeJS.Timeout;

  constructor() {
    this.heartbeatInterval = setInterval(
      () => this.checkHeartbeats(),
      this.HEARTBEAT_INTERVAL
    );
  }

  // ─── Connection management ───────────────────────────────────────────────

  /**
   * Add a new WebSocket connection to the hub.
   *
   * Reads the `token` query-string parameter from the upgrade request URL
   * and attempts JWT verification immediately.  If the token is valid, the
   * connection is marked authenticated and the user identity is stored on
   * ClientInfo.  If the token is absent or invalid the connection is still
   * accepted but left in the unauthenticated state — any subsequent
   * `subscribe` will be refused until the client provides a valid token.
   *
   * This design lets clients receive a welcome message (including their
   * assigned clientId) regardless of auth state, which is useful for
   * debugging and for clients that send the token as the very first message.
   */
  addConnection(socket: WebSocket, request: any): string {
    const clientId = uuidv4();
    const ip = request.socket?.remoteAddress ?? 'unknown';

    // Parse the JWT from the upgrade URL query string
    const { authenticated, userId, userEmail, userRole } =
      this.extractAndVerifyToken(request);

    const client: ClientInfo = {
      id: clientId,
      socket,
      subscribedStreams: new Set(),
      lastActivity: Date.now(),
      ip,
      authenticated,
      userId,
      userEmail,
      userRole
    };

    this.clients.set(clientId, client);

    socket.on('message', (data) => this.handleMessage(clientId, data));
    socket.on('close', () => this.removeConnection(clientId));
    socket.on('error', (error) => this.handleError(clientId, error));

    // Send welcome message — always, regardless of auth state.
    this.sendToClient(clientId, {
      type: 'connected',
      payload: { clientId, maxStreams: this.MAX_STREAMS_PER_CLIENT }
    });

    if (authenticated) {
      logger.info('WebSocket client connected (authenticated)', {
        clientId,
        ip,
        userId
      });
    } else {
      logger.info('WebSocket client connected (unauthenticated)', {
        clientId,
        ip
      });
    }

    return clientId;
  }

  // ─── Token extraction & verification ────────────────────────────────────

  /**
   * Extract the `token` query parameter from the HTTP upgrade request and
   * verify it against JWT_SECRET using the same logic as the REST
   * authenticate() middleware.
   *
   * Returns an auth result object; never throws.
   */
  private extractAndVerifyToken(request: any): {
    authenticated: boolean;
    userId?: string;
    userEmail?: string;
    userRole?: string;
  } {
    try {
      // request.url is the path+query from the upgrade request, e.g. "/ws?token=…"
      const rawUrl = request.url ?? '';
      // Use a base placeholder so URL can parse a relative path
      const parsed = new URL(rawUrl, 'ws://localhost');
      const token = parsed.searchParams.get('token');

      if (!token) {
        return { authenticated: false };
      }

      const jwtSecret = process.env.JWT_SECRET;
      if (!jwtSecret) {
        logger.error('JWT_SECRET not configured — cannot authenticate WebSocket clients');
        return { authenticated: false };
      }

      const decoded = jwt.verify(token, jwtSecret) as DecodedToken;
      return {
        authenticated: true,
        userId: decoded.id,
        userEmail: decoded.email,
        userRole: decoded.role
      };
    } catch (err) {
      if (err instanceof jwt.TokenExpiredError) {
        logger.debug('WebSocket upgrade rejected: token expired');
      } else if (err instanceof jwt.JsonWebTokenError) {
        logger.debug('WebSocket upgrade rejected: invalid token');
      } else {
        logger.debug('WebSocket upgrade: token verification failed', { err });
      }
      return { authenticated: false };
    }
  }

  // ─── Per-stream authorization ────────────────────────────────────────────

  /**
   * Determine whether the authenticated user is allowed to subscribe to the
   * given stream.
   *
   * INTERIM IMPLEMENTATION: Always returns true for any authenticated user
   * because stream ownership data (sender / recipient) is currently held only
   * in the in-memory route handler (src/routes/streams.ts) and is not exposed
   * to this module.
   *
   * HOW TO IMPLEMENT FULL AUTHORIZATION:
   *   1. Persist stream records (at minimum sender + recipient IDs) to a
   *      shared store (Postgres table, Redis hash, or an in-process registry
   *      shared via dependency injection).
   *   2. Inject a `StreamRepository` (or equivalent) into WebSocketHub.
   *   3. Replace the body of this method with:
   *        const owners = await streamRepo.getOwners(streamId);
   *        return owners?.senderId === userId || owners?.recipientId === userId;
   *
   * Until that work is done, every authenticated connection can subscribe to
   * any UUID-shaped streamId.  The WARNING log below keeps this gap visible
   * in production.
   */
  private async authorizeStreamAccess(
    userId: string,
    streamId: string
  ): Promise<boolean> {
    // TODO: replace with real ownership lookup when stream persistence is available.
    logger.warn(
      'Per-stream authorization not yet enforced — stream ownership data unavailable; ' +
      'any authenticated user may subscribe to any stream.',
      { userId, streamId }
    );
    return true;
  }

  // ─── Message handling ────────────────────────────────────────────────────

  /**
   * Handle incoming WebSocket messages
   */
  private async handleMessage(
    clientId: string,
    data: WebSocket.RawData
  ): Promise<void> {
    const client = this.clients.get(clientId);
    if (!client) return;

    client.lastActivity = Date.now();

    try {
      // Validate payload size.
      // RawData is Buffer | ArrayBuffer | Buffer[]; derive byte length safely.
      // Also handle the case where data is a plain string (used in unit tests
      // with mock sockets that call the message handler directly with strings).
      const byteLength: number =
        typeof data === 'string'
          ? Buffer.byteLength(data, 'utf8')
          : Buffer.isBuffer(data)
          ? data.length
          : Array.isArray(data)
          ? (data as Buffer[]).reduce((sum: number, b: Buffer) => sum + b.length, 0)
          : (data as ArrayBuffer).byteLength;

      if (byteLength > this.MAX_PAYLOAD_SIZE) {
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

      if (!parsed.type || typeof parsed.type !== 'string') {
        return null;
      }

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
  private async processMessage(
    clientId: string,
    message: WebSocketMessage
  ): Promise<void> {
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
        this.sendError(
          clientId,
          `Unknown message type: ${message.type}`,
          'UNKNOWN_MESSAGE_TYPE'
        );
    }
  }

  /**
   * Handle subscribe request.
   *
   * Guards (in order):
   *   1. UNAUTHORIZED  — client has no valid JWT.
   *   2. STREAM_ID_REQUIRED — streamId field missing.
   *   3. INVALID_STREAM_ID  — streamId is not a valid UUID.
   *   4. SUBSCRIPTION_LIMIT_EXCEEDED — per-client cap reached.
   *   5. UNAUTHORIZED  — authenticated user is not the stream's sender or
   *                       recipient (authorizeStreamAccess returns false).
   */
  private async handleSubscribe(
    clientId: string,
    streamId?: string
  ): Promise<void> {
    const client = this.clients.get(clientId);
    if (!client) return;

    // ── Guard 1: authentication ──────────────────────────────────────────
    if (!client.authenticated || !client.userId) {
      this.sendError(
        clientId,
        'Authentication required: provide a valid JWT via the token query parameter',
        'UNAUTHORIZED'
      );
      logger.warn('Unauthenticated subscribe attempt rejected', {
        clientId,
        ip: client.ip
      });
      return;
    }

    // ── Guard 2: streamId presence ───────────────────────────────────────
    if (!streamId) {
      this.sendError(clientId, 'Stream ID required for subscription', 'STREAM_ID_REQUIRED');
      return;
    }

    // ── Guard 3: UUID format ─────────────────────────────────────────────
    const uuidRegex =
      /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
    if (!uuidRegex.test(streamId)) {
      this.sendError(clientId, 'Invalid stream ID format', 'INVALID_STREAM_ID');
      return;
    }

    // ── Guard 4: subscription limit ──────────────────────────────────────
    if (client.subscribedStreams.size >= this.MAX_STREAMS_PER_CLIENT) {
      this.sendError(clientId, 'Subscription limit reached', 'SUBSCRIPTION_LIMIT_EXCEEDED');
      return;
    }

    // ── Guard 5: per-stream authorization ────────────────────────────────
    const allowed = await this.authorizeStreamAccess(client.userId, streamId);
    if (!allowed) {
      this.sendError(
        clientId,
        'Access denied: you are not a participant in this stream',
        'UNAUTHORIZED'
      );
      logger.warn('Unauthorized stream subscribe attempt rejected', {
        clientId,
        userId: client.userId,
        streamId,
        ip: client.ip
      });
      return;
    }

    // ── Subscribe ────────────────────────────────────────────────────────
    client.subscribedStreams.add(streamId);

    if (!this.streamSubscriptions.has(streamId)) {
      this.streamSubscriptions.set(streamId, new Set());
    }
    this.streamSubscriptions.get(streamId)!.add(clientId);

    logger.info('Client subscribed to stream', {
      clientId,
      userId: client.userId,
      streamId,
      ip: client.ip
    });

    this.sendToClient(clientId, {
      type: 'subscribed',
      streamId,
      payload: { subscribedAt: Date.now() }
    });
  }

  /**
   * Handle unsubscribe request
   */
  private async handleUnsubscribe(
    clientId: string,
    streamId?: string
  ): Promise<void> {
    const client = this.clients.get(clientId);
    if (!client) return;

    if (!streamId) {
      this.sendError(
        clientId,
        'Stream ID required for unsubscription',
        'STREAM_ID_REQUIRED'
      );
      return;
    }

    const wasSubscribed = client.subscribedStreams.delete(streamId);

    const streamSubscribers = this.streamSubscriptions.get(streamId);
    if (streamSubscribers) {
      streamSubscribers.delete(clientId);
      if (streamSubscribers.size === 0) {
        this.streamSubscriptions.delete(streamId);
      }
    }

    if (wasSubscribed) {
      logger.info('Client unsubscribed from stream', {
        clientId,
        userId: client.userId,
        streamId,
        ip: client.ip
      });
      this.sendToClient(clientId, {
        type: 'unsubscribed',
        streamId,
        payload: { unsubscribedAt: Date.now() }
      });
    } else {
      this.sendError(clientId, 'Not subscribed to this stream', 'NOT_SUBSCRIBED');
    }
  }

  // ─── Broadcasting ────────────────────────────────────────────────────────

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

  // ─── Helpers ─────────────────────────────────────────────────────────────

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

  private sendError(clientId: string, message: string, code: string): void {
    this.sendToClient(clientId, {
      type: 'error',
      payload: { message, code }
    });
  }

  // ─── Lifecycle management ────────────────────────────────────────────────

  private removeConnection(clientId: string): void {
    const client = this.clients.get(clientId);
    if (!client) return;

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

  private handleError(clientId: string, error: Error): void {
    logger.error('WebSocket error', { clientId, error: error.message });
    this.removeConnection(clientId);
  }

  private checkHeartbeats(): void {
    const now = Date.now();
    const maxInactiveTime = this.HEARTBEAT_INTERVAL * 3; // 90 seconds

    for (const [clientId, client] of this.clients.entries()) {
      if (now - client.lastActivity > maxInactiveTime) {
        logger.info('Closing inactive WebSocket connection', {
          clientId,
          ip: client.ip
        });
        client.socket.close(1000, 'Connection timeout');
        this.removeConnection(clientId);
      }
    }
  }

  // ─── Introspection ───────────────────────────────────────────────────────

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

  cleanup(): void {
    clearInterval(this.heartbeatInterval);
    for (const client of this.clients.values()) {
      client.socket.close(1001, 'Server shutdown');
    }
    this.clients.clear();
    this.streamSubscriptions.clear();
  }
}
