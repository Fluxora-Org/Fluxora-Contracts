/**
 * Stream realtime runtime
 *
 * Process-wide holders for the shared WebSocketHub and StreamChannel.
 * Initialized once at server start (see index.ts); route handlers access
 * the channel via getStreamChannel() so they stay decoupled from construction.
 *
 * When not initialized (unit tests of routes without a WS server), getters
 * return null so callers can no-op safely with optional chaining.
 */

import { WebSocketHub } from '../ws/hub';
import { StreamChannel } from './streamChannel';

let hub: WebSocketHub | null = null;
let channel: StreamChannel | null = null;

/**
 * Construct (or return) the shared hub + stream channel pair.
 * Safe to call more than once; subsequent calls reuse the same instances.
 */
export function initStreamRealtime(): {
  hub: WebSocketHub;
  channel: StreamChannel;
} {
  if (!hub) {
    hub = new WebSocketHub();
    channel = new StreamChannel(hub);
  }
  // channel is always set whenever hub is set
  return { hub, channel: channel as StreamChannel };
}

/** Shared StreamChannel, or null if realtime has not been initialized. */
export function getStreamChannel(): StreamChannel | null {
  return channel;
}

/** Shared WebSocketHub, or null if realtime has not been initialized. */
export function getWebSocketHub(): WebSocketHub | null {
  return hub;
}

/**
 * Tear down the shared runtime (clears hub timers/clients).
 * Intended for graceful process shutdown and integration-test cleanup.
 */
export function resetStreamRealtime(): void {
  if (hub) {
    hub.cleanup();
  }
  hub = null;
  channel = null;
}
