/**
 * Request Protection Middleware
 *
 * Provides idempotency-key handling for mutating routes (POST).
 *
 * ## How it works
 *
 * 1. Client sends `Idempotency-Key: <uuid-v4>` header on a POST request.
 * 2. Middleware checks the cache for a stored result keyed by
 *    `idempotency:<userId>:<key>`.
 *    - The key is scoped to the authenticated user so one user cannot
 *      replay another user's request.
 * 3. Cache miss → request proceeds normally.  After the handler writes a
 *    response the middleware intercepts the body and stores it in cache
 *    (TTL: 24 h) so future replays return the same payload.
 * 4. Cache hit → 200 (or original status) is returned immediately with the
 *    stored body.  No handler is invoked.
 * 5. In-flight duplicate (same key, request still processing) → 409 Conflict.
 *
 * ## Security
 *
 * - The raw key value is NEVER logged; only a SHA-256 hash is logged so
 *   secrets embedded in keys cannot leak into log aggregators.
 * - Keys are scoped per-user to prevent cross-user replay attacks.
 * - Keys must be UUID v4 format; any other value is rejected with 400.
 *
 * ## Decimal-string guarantee
 *
 * Stored response bodies are serialized/deserialized as plain strings so
 * amount fields that were already serialized as decimal strings by the
 * handler are preserved verbatim.
 */

import { Request, Response, NextFunction } from 'express';
import { createHash } from 'crypto';
import cache from '../utils/cache';
import logger from '../utils/logger';
import { sendError } from '../utils/response';

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/** Header name clients must use. */
export const IDEMPOTENCY_HEADER = 'idempotency-key';

/** TTL for stored idempotency results (24 hours). */
const IDEMPOTENCY_TTL_SECONDS = 86_400;

/** Sentinel stored while a request is in-flight to detect concurrent dupes. */
const IN_FLIGHT_SENTINEL = '__IN_FLIGHT__';

/** UUID v4 regex — the only accepted key format. */
const UUID_V4_RE =
  /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/**
 * Hash the raw idempotency key for safe logging.
 * The hash is deterministic so log correlation is still possible without
 * exposing the raw value.
 */
function hashKey(raw: string): string {
  return createHash('sha256').update(raw).digest('hex').slice(0, 16);
}

/**
 * Build the cache key scoped to the authenticated user.
 */
function buildCacheKey(userId: string, rawKey: string): string {
  return `idempotency:${userId}:${rawKey}`;
}

// ---------------------------------------------------------------------------
// Stored result shape
// ---------------------------------------------------------------------------

interface StoredResult {
  statusCode: number;
  body: unknown;
}

// ---------------------------------------------------------------------------
// Middleware
// ---------------------------------------------------------------------------

/**
 * Idempotency middleware factory.
 *
 * Usage:
 *   router.post('/streams', authenticate, idempotency(), createStream);
 *
 * The middleware is a no-op when the `Idempotency-Key` header is absent,
 * allowing routes to opt-in without breaking clients that don't send it.
 */
export function idempotency() {
  return async function idempotencyMiddleware(
    req: Request,
    res: Response,
    next: NextFunction
  ): Promise<void> {
    const rawKey = req.headers[IDEMPOTENCY_HEADER] as string | undefined;

    // ── No header → pass through ──────────────────────────────────────────
    if (!rawKey) {
      return next();
    }

    // ── Validate key format ───────────────────────────────────────────────
    if (!UUID_V4_RE.test(rawKey)) {
      sendError(
        res,
        400,
        'Idempotency-Key must be a valid UUID v4',
        'INVALID_IDEMPOTENCY_KEY'
      );
      return;
    }

    // ── Require authenticated user (key is scoped per-user) ───────────────
    const userId = req.user?.id;
    if (!userId) {
      sendError(res, 401, 'Authentication required', 'UNAUTHORIZED');
      return;
    }

    const cacheKey = buildCacheKey(userId, rawKey);
    const keyHash = hashKey(rawKey);
    const correlationId = req.headers['x-correlation-id'] as string | undefined;

    // ── Check cache ───────────────────────────────────────────────────────
    const cached = await cache.get<StoredResult | typeof IN_FLIGHT_SENTINEL>(cacheKey);

    if (cached !== null) {
      // In-flight duplicate
      if (cached === IN_FLIGHT_SENTINEL) {
        logger.warn('Idempotency: concurrent duplicate request rejected', {
          keyHash,
          userId,
          correlationId,
          path: req.path
        });
        sendError(
          res,
          409,
          'A request with this Idempotency-Key is already being processed',
          'IDEMPOTENCY_CONFLICT',
          correlationId
        );
        return;
      }

      // Replay — return stored result
      logger.info('Idempotency: replaying cached response', {
        keyHash,
        userId,
        correlationId,
        path: req.path,
        cachedStatus: (cached as StoredResult).statusCode
      });

      const stored = cached as StoredResult;
      res.status(stored.statusCode).json(stored.body);
      return;
    }

    // ── Mark in-flight ────────────────────────────────────────────────────
    // Short TTL for the sentinel so a crashed process doesn't permanently
    // block the key.  30 s is generous for any synchronous handler.
    await cache.set(cacheKey, IN_FLIGHT_SENTINEL, 30);

    logger.debug('Idempotency: new request, sentinel stored', {
      keyHash,
      userId,
      correlationId,
      path: req.path
    });

    // ── Intercept response to capture and store result ────────────────────
    const originalJson = res.json.bind(res);

    res.json = function interceptedJson(body: unknown): Response {
      // Only cache successful (2xx) responses
      if (res.statusCode >= 200 && res.statusCode < 300) {
        const stored: StoredResult = { statusCode: res.statusCode, body };

        // Fire-and-forget — don't delay the response
        cache.set(cacheKey, stored, IDEMPOTENCY_TTL_SECONDS).catch(err => {
          logger.error('Idempotency: failed to store result in cache', {
            keyHash,
            userId,
            correlationId,
            error: err
          });
        });

        logger.info('Idempotency: result stored for future replays', {
          keyHash,
          userId,
          correlationId,
          path: req.path,
          statusCode: res.statusCode
        });
      } else {
        // Non-2xx: remove sentinel so the client can retry
        cache.del(cacheKey).catch(err => {
          logger.error('Idempotency: failed to remove sentinel after error', {
            keyHash,
            userId,
            correlationId,
            error: err
          });
        });
      }

      return originalJson(body);
    };

    next();
  };
}
