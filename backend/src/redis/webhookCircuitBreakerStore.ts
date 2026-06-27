/**
 * Webhook Circuit Breaker Store
 *
 * Persists circuit-breaker state for webhook consumers in Redis so that
 * persistently-failing endpoints are isolated automatically and recovered
 * gracefully via a timed probe window.
 *
 * ## Circuit states
 *
 * - `CLOSED`    — normal operation; failures are counted toward the threshold.
 * - `OPEN`      — too many consecutive failures; all calls are rejected until
 *                 `probeAt` (epoch ms) elapses.
 * - `HALF_OPEN` — one probe request is let through; success resets the circuit,
 *                 failure reopens it.
 *
 * ## Redis key schema  (all scoped to `consumerKey` to prevent cross-consumer bleed)
 *
 *   webhook:cb:<consumerKey>:state      → CircuitState string
 *   webhook:cb:<consumerKey>:failures   → integer failure count
 *   webhook:cb:<consumerKey>:probe_at   → epoch ms as string
 *
 * ## Security
 *
 * Only the opaque `consumerKey` identifier appears in log fields.
 * Consumer URLs, tokens, and any other secret material are never logged.
 */

import Redis from 'ioredis';
import logger from '../utils/logger';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export type CircuitState = 'CLOSED' | 'OPEN' | 'HALF_OPEN';

export interface CircuitStatus {
  state: CircuitState;
  failures: number;
  /** Epoch ms at which a probe is allowed; `null` when the circuit is CLOSED. */
  probeAt: number | null;
}

export interface CircuitBreakerStoreOptions {
  /** Number of consecutive failures required to open the circuit. Default: 5. */
  failureThreshold?: number;
  /**
   * Seconds the circuit stays OPEN before a probe is permitted. Default: 60.
   * Redis keys are given a TTL of `2 × openTtlSeconds` to self-clean.
   */
  openTtlSeconds?: number;
}

// ---------------------------------------------------------------------------
// Defaults
// ---------------------------------------------------------------------------

const DEFAULT_FAILURE_THRESHOLD = 5;
const DEFAULT_OPEN_TTL_SECONDS = 60;

// ---------------------------------------------------------------------------
// Key helpers
// ---------------------------------------------------------------------------

function stateKey(consumerKey: string): string {
  return `webhook:cb:${consumerKey}:state`;
}

function failuresKey(consumerKey: string): string {
  return `webhook:cb:${consumerKey}:failures`;
}

function probeAtKey(consumerKey: string): string {
  return `webhook:cb:${consumerKey}:probe_at`;
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

/**
 * Redis-backed store for webhook circuit-breaker state.
 *
 * All Redis errors are caught, logged via the structured logger with
 * operation context, and handled with safe fallbacks so a Redis outage
 * does not cascade into webhook delivery failures.
 */
export class WebhookCircuitBreakerStore {
  private readonly redis: Redis;
  private readonly failureThreshold: number;
  private readonly openTtlSeconds: number;

  constructor(redis: Redis, options: CircuitBreakerStoreOptions = {}) {
    this.redis = redis;
    this.failureThreshold =
      options.failureThreshold ?? DEFAULT_FAILURE_THRESHOLD;
    this.openTtlSeconds = options.openTtlSeconds ?? DEFAULT_OPEN_TTL_SECONDS;
  }

  /**
   * Retrieve the current circuit status for a consumer.
   *
   * @returns CLOSED with zero failures when no entry exists.
   *          Falls back to CLOSED on Redis error so callers fail open.
   */
  async getStatus(consumerKey: string): Promise<CircuitStatus> {
    try {
      const [state, failures, probeAt] = await this.redis.mget(
        stateKey(consumerKey),
        failuresKey(consumerKey),
        probeAtKey(consumerKey)
      );

      return {
        state: (state as CircuitState | null) ?? 'CLOSED',
        failures: failures !== null ? parseInt(failures, 10) : 0,
        probeAt: probeAt !== null ? parseInt(probeAt, 10) : null,
      };
    } catch (error) {
      logger.error('Circuit breaker: failed to read state from Redis', {
        consumerKey,
        operation: 'getStatus',
        error: error instanceof Error ? error.message : String(error),
      });
      // Fail open — assume CLOSED so webhook delivery can still proceed.
      return { state: 'CLOSED', failures: 0, probeAt: null };
    }
  }

  /**
   * Record a delivery failure for a consumer and open the circuit when the
   * failure count reaches the configured threshold.
   *
   * @returns Updated circuit status after recording the failure.
   *          Falls back to CLOSED on Redis error.
   */
  async recordFailure(consumerKey: string): Promise<CircuitStatus> {
    try {
      const failures = await this.redis.incr(failuresKey(consumerKey));

      if (failures >= this.failureThreshold) {
        const probeAt = Date.now() + this.openTtlSeconds * 1_000;
        const ttl = this.openTtlSeconds * 2;

        const pipeline = this.redis.pipeline();
        pipeline.set(stateKey(consumerKey), 'OPEN');
        pipeline.set(probeAtKey(consumerKey), String(probeAt));
        pipeline.expire(stateKey(consumerKey), ttl);
        pipeline.expire(failuresKey(consumerKey), ttl);
        pipeline.expire(probeAtKey(consumerKey), ttl);
        await pipeline.exec();

        logger.warn('Circuit breaker: circuit opened due to excessive failures', {
          consumerKey,
          failures,
          threshold: this.failureThreshold,
          probeAt,
        });

        return { state: 'OPEN', failures, probeAt };
      }

      logger.info('Circuit breaker: failure recorded', {
        consumerKey,
        failures,
        threshold: this.failureThreshold,
      });

      return { state: 'CLOSED', failures, probeAt: null };
    } catch (error) {
      logger.error('Circuit breaker: failed to record failure in Redis', {
        consumerKey,
        operation: 'recordFailure',
        error: error instanceof Error ? error.message : String(error),
      });
      return { state: 'CLOSED', failures: 0, probeAt: null };
    }
  }

  /**
   * Transition the circuit to `HALF_OPEN` so a single probe request can test
   * whether the consumer has recovered.  Should only be called after verifying
   * that `probeAt` has elapsed.
   *
   * @returns `true` when the probe is permitted (state is now HALF_OPEN),
   *          `false` when the circuit is still within its open window or the
   *          operation fails.
   */
  async allowProbe(consumerKey: string): Promise<boolean> {
    try {
      const status = await this.getStatus(consumerKey);

      if (status.state === 'HALF_OPEN') {
        return true;
      }

      if (status.state !== 'OPEN') {
        return false;
      }

      if (status.probeAt !== null && Date.now() < status.probeAt) {
        return false;
      }

      const pipeline = this.redis.pipeline();
      pipeline.set(stateKey(consumerKey), 'HALF_OPEN');
      pipeline.expire(stateKey(consumerKey), this.openTtlSeconds * 2);
      await pipeline.exec();

      logger.info('Circuit breaker: transitioned to HALF_OPEN for probe', {
        consumerKey,
      });

      return true;
    } catch (error) {
      logger.error('Circuit breaker: failed to transition to HALF_OPEN in Redis', {
        consumerKey,
        operation: 'allowProbe',
        error: error instanceof Error ? error.message : String(error),
      });
      return false;
    }
  }

  /**
   * Reset the circuit to `CLOSED` after a successful probe delivery.
   * Deletes all Redis keys for the consumer so state starts fresh.
   */
  async reset(consumerKey: string): Promise<void> {
    try {
      const pipeline = this.redis.pipeline();
      pipeline.del(stateKey(consumerKey));
      pipeline.del(failuresKey(consumerKey));
      pipeline.del(probeAtKey(consumerKey));
      await pipeline.exec();

      logger.info('Circuit breaker: circuit reset to CLOSED', {
        consumerKey,
      });
    } catch (error) {
      logger.error('Circuit breaker: failed to reset circuit in Redis', {
        consumerKey,
        operation: 'reset',
        error: error instanceof Error ? error.message : String(error),
      });
    }
  }
}
