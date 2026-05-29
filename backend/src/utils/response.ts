/**
 * Standardized HTTP response helpers.
 *
 * All API responses follow the envelope:
 *   { success: true,  data: T }
 *   { success: false, error: { message, code, correlationId? } }
 *
 * Amount fields that represent on-chain / API token amounts are serialized as
 * decimal strings to preserve precision across JSON parsers (avoids IEEE-754
 * truncation for values > Number.MAX_SAFE_INTEGER).
 */

import { Response } from 'express';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface SuccessEnvelope<T> {
  success: true;
  data: T;
}

export interface ErrorEnvelope {
  success: false;
  error: {
    message: string;
    code: string;
    correlationId?: string;
  };
}

export type ApiEnvelope<T> = SuccessEnvelope<T> | ErrorEnvelope;

// ---------------------------------------------------------------------------
// Decimal-string serialization
// ---------------------------------------------------------------------------

/**
 * Serialize a bigint or numeric amount to a decimal string.
 * This guarantees that chain/API amount fields survive JSON round-trips
 * without precision loss regardless of the client's JSON parser.
 *
 * @example
 *   toDecimalString(123456789012345678901234567890n) // "123456789012345678901234567890"
 *   toDecimalString(100)                             // "100"
 *   toDecimalString("99.5")                          // "99.5"
 */
export function toDecimalString(amount: bigint | number | string): string {
  if (typeof amount === 'bigint') return amount.toString(10);
  if (typeof amount === 'number') {
    if (!Number.isFinite(amount)) throw new TypeError('Amount must be a finite number');
    return amount.toString(10);
  }
  // string — validate it looks like a decimal number
  if (!/^-?\d+(\.\d+)?$/.test(amount)) {
    throw new TypeError(`Invalid decimal string: "${amount}"`);
  }
  return amount;
}

// ---------------------------------------------------------------------------
// Response helpers
// ---------------------------------------------------------------------------

/**
 * Send a successful JSON response.
 */
export function sendSuccess<T>(
  res: Response,
  data: T,
  statusCode = 200
): void {
  const body: SuccessEnvelope<T> = { success: true, data };
  res.status(statusCode).json(body);
}

/**
 * Send an error JSON response.
 */
export function sendError(
  res: Response,
  statusCode: number,
  message: string,
  code: string,
  correlationId?: string
): void {
  const body: ErrorEnvelope = {
    success: false,
    error: { message, code, ...(correlationId ? { correlationId } : {}) }
  };
  res.status(statusCode).json(body);
}

/**
 * Send a 409 Conflict response for idempotency key collisions.
 * The original cached response body is re-sent verbatim so the client
 * receives the same payload as the first successful request.
 */
export function sendConflict(
  res: Response,
  message: string,
  correlationId?: string
): void {
  sendError(res, 409, message, 'CONFLICT', correlationId);
}
