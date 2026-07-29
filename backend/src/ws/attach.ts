/**
 * Attach a WebSocket server to an HTTP server and route upgrades through
 * the shared WebSocketHub. Used by the application entrypoint and by
 * integration tests so production and test wiring cannot drift.
 */

import http from 'http';
import WebSocket from 'ws';
import { WebSocketHub } from './hub';

/** Path clients use to open a real-time connection. */
export const WEBSOCKET_PATH = '/ws';

/** Max inbound WebSocket frame size (matches hub MAX_PAYLOAD_SIZE). */
export const WEBSOCKET_MAX_PAYLOAD = 1024 * 16;

/**
 * Create a `ws.Server` bound to `server` on {@link WEBSOCKET_PATH} and
 * forward each new connection to `hub.addConnection`.
 */
export function attachWebSocketHub(
  server: http.Server,
  hub: WebSocketHub
): WebSocket.Server {
  const wss = new WebSocket.Server({
    server,
    path: WEBSOCKET_PATH,
    maxPayload: WEBSOCKET_MAX_PAYLOAD
  });

  wss.on('connection', (socket, request) => {
    hub.addConnection(socket, request);
  });

  return wss;
}
