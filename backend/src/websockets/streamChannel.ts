/**
 * Stream Channel
 * 
 * Manages real-time updates for payment streams.
 * Integrates with WebSocket hub for per-stream filtering.
 */

import { WebSocketHub } from '../ws/hub';
import logger from '../utils/logger';

export interface StreamUpdate {
  streamId: string;
  type: 'created' | 'updated' | 'cancelled' | 'withdrawn' | 'completed';
  data: any;
  timestamp: number;
}

export class StreamChannel {
  private hub: WebSocketHub;

  constructor(hub: WebSocketHub) {
    this.hub = hub;
  }

  /**
   * Notify subscribers about stream creation
   */
  notifyStreamCreated(streamId: string, streamData: any): void {
    const update: StreamUpdate = {
      streamId,
      type: 'created',
      data: streamData,
      timestamp: Date.now()
    };

    this.hub.broadcastToStream(streamId, update);
    logger.info('Stream creation notified', { streamId });
  }

  /**
   * Notify subscribers about stream update
   */
  notifyStreamUpdated(streamId: string, updateData: any): void {
    const update: StreamUpdate = {
      streamId,
      type: 'updated',
      data: updateData,
      timestamp: Date.now()
    };

    this.hub.broadcastToStream(streamId, update);
    logger.debug('Stream update notified', { streamId });
  }

  /**
   * Notify subscribers about stream cancellation
   */
  notifyStreamCancelled(streamId: string, cancellationData: any): void {
    const update: StreamUpdate = {
      streamId,
      type: 'cancelled',
      data: cancellationData,
      timestamp: Date.now()
    };

    this.hub.broadcastToStream(streamId, update);
    logger.info('Stream cancellation notified', { streamId });
  }

  /**
   * Notify subscribers about stream withdrawal
   */
  notifyStreamWithdrawn(streamId: string, withdrawalData: any): void {
    const update: StreamUpdate = {
      streamId,
      type: 'withdrawn',
      data: withdrawalData,
      timestamp: Date.now()
    };

    this.hub.broadcastToStream(streamId, update);
    logger.debug('Stream withdrawal notified', { streamId });
  }

  /**
   * Notify subscribers about stream completion
   */
  notifyStreamCompleted(streamId: string, completionData: any): void {
    const update: StreamUpdate = {
      streamId,
      type: 'completed',
      data: completionData,
      timestamp: Date.now()
    };

    this.hub.broadcastToStream(streamId, update);
    logger.info('Stream completion notified', { streamId });
  }

  /**
   * Validate stream ID format
   */
  static validateStreamId(streamId: string): boolean {
    const uuidRegex = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
    return uuidRegex.test(streamId);
  }

  /**
   * Get subscription statistics for a stream
   */
  getStreamSubscriptionStats(streamId: string): { subscriberCount: number } | null {
    // This would need access to hub's internal state
    // For now, return placeholder
    return { subscriberCount: 0 };
  }

  /**
   * Clean up stream subscriptions
   */
  cleanupStreamSubscriptions(streamId: string): void {
    // This would clean up all subscriptions for a stream
    // when it's deleted or archived
    logger.info('Cleaning up stream subscriptions', { streamId });
  }
}