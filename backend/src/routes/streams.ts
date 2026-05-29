/**
 * Stream Creation Routes
 *
 * POST /api/v1/streams
 *   Create a new payment stream.
 *   Supports idempotency via the `Idempotency-Key` header (UUID v4).
 *
 * Idempotency behaviour (aligned with OpenAPI spec):
 *   - First request with a given key → 201 Created, result cached 24 h.
 *   - Replay with same key           → 200 OK, original body returned.
 *   - Concurrent duplicate           → 409 Conflict.
 *   - Missing / malformed key        → request proceeds without idempotency
 *     protection (key is optional).
 *
 * Amount fields (depositAmount, ratePerSecond) are serialized as decimal
 * strings to preserve precision for on-chain / large-integer values.
 */

import { Router, Request, Response, NextFunction } from 'express';
import { v4 as uuidv4 } from 'uuid';
import rateLimit from 'express-rate-limit';
import { authenticate } from '../middleware/auth.middleware';
import { idempotency } from '../middleware/requestProtection';
import { sendSuccess, sendError, toDecimalString } from '../utils/response';
import { ValidationError } from '../utils/errors';
import logger from '../utils/logger';

const router = Router();

// ---------------------------------------------------------------------------
// Rate limiting — stream creation is a sensitive write operation
// ---------------------------------------------------------------------------

const createStreamLimiter = rateLimit({
  windowMs: 15 * 60 * 1000, // 15 minutes
  max: process.env.NODE_ENV === 'test' ? 10_000 : 30,
  standardHeaders: true,
  legacyHeaders: false,
  message: {
    success: false,
    error: { message: 'Too many requests', code: 'RATE_LIMIT_EXCEEDED' }
  }
});

// ---------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------

const UUID_RE =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;

/** Validate that a string represents a positive decimal integer (for amounts). */
function isPositiveDecimalString(value: unknown): value is string {
  if (typeof value !== 'string') return false;
  if (!/^\d+(\.\d+)?$/.test(value)) return false;
  const n = Number(value);
  return Number.isFinite(n) && n > 0;
}

/**
 * Validate and parse the stream creation request body.
 * Throws ValidationError with a descriptive message on any failure.
 */
function validateCreateStreamBody(body: Record<string, unknown>): CreateStreamInput {
  const { recipientId, depositAmount, ratePerSecond, startTime, endTime, cliffTime } = body;

  if (!recipientId || !UUID_RE.test(String(recipientId))) {
    throw new ValidationError('recipientId must be a valid UUID');
  }

  if (!isPositiveDecimalString(depositAmount)) {
    throw new ValidationError(
      'depositAmount must be a positive decimal string (e.g. "1000000")'
    );
  }

  if (!isPositiveDecimalString(ratePerSecond)) {
    throw new ValidationError(
      'ratePerSecond must be a positive decimal string (e.g. "100")'
    );
  }

  const startTs = Number(startTime);
  if (!Number.isInteger(startTs) || startTs <= 0) {
    throw new ValidationError('startTime must be a positive integer Unix timestamp');
  }

  const endTs = Number(endTime);
  if (!Number.isInteger(endTs) || endTs <= startTs) {
    throw new ValidationError('endTime must be a positive integer Unix timestamp after startTime');
  }

  // cliffTime is optional but must be between startTime and endTime if provided
  let cliffTs: number | undefined;
  if (cliffTime !== undefined && cliffTime !== null) {
    cliffTs = Number(cliffTime);
    if (!Number.isInteger(cliffTs) || cliffTs < startTs || cliffTs > endTs) {
      throw new ValidationError(
        'cliffTime must be a Unix timestamp between startTime and endTime (inclusive)'
      );
    }
  }

  return {
    recipientId: String(recipientId),
    depositAmount: String(depositAmount),
    ratePerSecond: String(ratePerSecond),
    startTime: startTs,
    endTime: endTs,
    cliffTime: cliffTs
  };
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface CreateStreamInput {
  recipientId: string;
  depositAmount: string;   // decimal string — chain amount
  ratePerSecond: string;   // decimal string — chain amount
  startTime: number;
  endTime: number;
  cliffTime?: number;
}

export interface StreamResponse {
  id: string;
  senderId: string;
  recipientId: string;
  depositAmount: string;   // decimal string
  ratePerSecond: string;   // decimal string
  startTime: number;
  endTime: number;
  cliffTime: number | null;
  status: 'active';
  createdAt: string;       // ISO-8601
}

// ---------------------------------------------------------------------------
// Route handler
// ---------------------------------------------------------------------------

/**
 * POST /api/v1/streams
 *
 * Create a new payment stream.
 * Protected by JWT auth + optional idempotency key.
 */
async function createStream(
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> {
  const correlationId =
    (req.headers['x-correlation-id'] as string | undefined) ?? uuidv4();

  try {
    const senderId = req.user?.id;
    if (!senderId) {
      sendError(res, 401, 'Authentication required', 'UNAUTHORIZED', correlationId);
      return;
    }

    // Validate body
    let input: CreateStreamInput;
    try {
      input = validateCreateStreamBody(req.body ?? {});
    } catch (err) {
      if (err instanceof ValidationError) {
        logger.warn('Stream creation validation failed', {
          correlationId,
          senderId,
          message: err.message
        });
        sendError(res, 400, err.message, 'VALIDATION_ERROR', correlationId);
        return;
      }
      throw err;
    }

    logger.info('Creating stream', {
      correlationId,
      senderId,
      recipientId: input.recipientId,
      depositAmount: input.depositAmount,
      ratePerSecond: input.ratePerSecond
    });

    // ── Business logic ──────────────────────────────────────────────────
    // In a real implementation this would call a service layer that
    // persists the stream and submits the on-chain transaction.
    // We generate a deterministic-looking ID here so the layer is easy
    // to swap out without changing the route or middleware.
    const streamId = uuidv4();
    const now = new Date().toISOString();

    const stream: StreamResponse = {
      id: streamId,
      senderId,
      recipientId: input.recipientId,
      // Guarantee decimal-string serialization for chain amount fields
      depositAmount: toDecimalString(input.depositAmount),
      ratePerSecond: toDecimalString(input.ratePerSecond),
      startTime: input.startTime,
      endTime: input.endTime,
      cliffTime: input.cliffTime ?? null,
      status: 'active',
      createdAt: now
    };

    logger.info('Stream created', {
      correlationId,
      senderId,
      streamId,
      recipientId: input.recipientId
    });

    // 201 Created — the idempotency middleware will cache this response
    sendSuccess(res, { stream }, 201);
  } catch (error) {
    logger.error('Unexpected error in createStream', {
      correlationId,
      error: (error as Error).message
    });
    next(error);
  }
}

// ---------------------------------------------------------------------------
// Route registration
// ---------------------------------------------------------------------------

router.post(
  '/',
  createStreamLimiter,
  authenticate,
  idempotency(),
  createStream
);

export default router;
