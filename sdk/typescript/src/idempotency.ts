/**
 * Idempotency Helper Contract
 *
 * This helper enforces idempotency for backend API requests to ensure that 
 * retried requests (e.g., due to network timeouts) do not result in duplicated 
 * operations (such as multiple stream creations).
 *
 * Requirements addressed:
 * - Validation: Ensures the idempotency key is well-formed.
 * - Retry/Collision: Differentiates between a concurrent retry (409 Conflict) 
 *   and a successful replay of a cached response.
 * - Auth/Isolation: Binds the idempotency key to the specific authenticated 
 *   caller (or session) to prevent cross-tenant key collisions.
 * - Observability: Provides logging hooks for cache hits, misses, and collisions.
 */

export interface CacheEntry<T> {
  response: T;
  statusCode: number;
}

export interface ICacheStore {
  get<T>(key: string): Promise<CacheEntry<T> | null>;
  set<T>(key: string, entry: CacheEntry<T>, ttlSeconds?: number): Promise<void>;
  /** Returns false if the lock is already held */
  acquireLock(key: string, ttlSeconds: number): Promise<boolean>;
  releaseLock(key: string): Promise<void>;
}

export interface IdempotencyOptions {
  /** The unique idempotency key provided by the client */
  idempotencyKey: string;
  /** The identifier of the authenticated caller (e.g. user ID or API key ID) */
  callerId: string;
  /** TTL for the idempotency cache in seconds (default: 86400 / 24h) */
  ttlSeconds?: number;
}

export interface Logger {
  info(message: string, meta?: any): void;
  warn(message: string, meta?: any): void;
  error(message: string, meta?: any): void;
}

export class IdempotencyError extends Error {
  constructor(public code: string, message: string) {
    super(message);
    this.name = 'IdempotencyError';
  }
}

export class IdempotencyHelper {
  constructor(
    private store: ICacheStore,
    private logger: Logger = console
  ) {}

  /**
   * Executes an operation idempotently.
   * 
   * @param options Idempotency settings including the key and caller context.
   * @param operation The business logic to execute if this is a fresh request.
   * @returns The result of the operation or the cached result.
   */
  async execute<T>(
    options: IdempotencyOptions,
    operation: () => Promise<{ response: T; statusCode: number }>
  ): Promise<{ response: T; statusCode: number; cached: boolean }> {
    const { idempotencyKey, callerId, ttlSeconds = 86400 } = options;

    if (!idempotencyKey || typeof idempotencyKey !== 'string' || idempotencyKey.trim() === '') {
      this.logger.warn('Idempotency validation failed: Invalid idempotency key', { idempotencyKey, callerId });
      throw new IdempotencyError('INVALID_KEY', 'Idempotency key is required and must be a valid string.');
    }

    if (!callerId || typeof callerId !== 'string' || callerId.trim() === '') {
      this.logger.warn('Idempotency validation failed: Invalid caller ID', { idempotencyKey, callerId });
      throw new IdempotencyError('INVALID_CALLER', 'Caller ID is required and must be a valid string.');
    }

    // Isolate keys per caller to prevent cross-tenant collisions
    const scopedKey = `idempotency:${callerId}:${idempotencyKey}`;

    // 1. Check if we already have a completed response
    const cached = await this.store.get<T>(scopedKey);
    if (cached) {
      this.logger.info('Idempotency cache hit', { idempotencyKey, callerId });
      return { ...cached, cached: true };
    }

    // 2. Acquire a lock to prevent concurrent processing of the same key
    const lockKey = `${scopedKey}:lock`;
    const lockAcquired = await this.store.acquireLock(lockKey, 30); // 30 second lock
    if (!lockAcquired) {
      this.logger.warn('Concurrent request collision', { idempotencyKey, callerId });
      throw new IdempotencyError('CONCURRENT_REQUEST', 'A request with this idempotency key is already in progress.');
    }

    try {
      // 3. Execute the operation
      this.logger.info('Executing fresh operation', { idempotencyKey, callerId });
      const result = await operation();

      // 4. Cache the result for future replays
      await this.store.set(scopedKey, result, ttlSeconds);
      
      return { ...result, cached: false };
    } catch (error) {
      this.logger.error('Operation failed during idempotent execution', { idempotencyKey, callerId, error });
      throw error;
    } finally {
      // 5. Release the lock
      await this.store.releaseLock(lockKey);
    }
  }
}
