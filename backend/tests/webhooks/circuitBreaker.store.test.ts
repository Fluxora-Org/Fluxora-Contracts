/**
 * Unit tests for src/redis/webhookCircuitBreakerStore.ts
 *
 * Verifies that:
 *   - All Redis error paths use the structured logger (never console.error).
 *   - Correct circuit state transitions occur (CLOSED → OPEN → HALF_OPEN → CLOSED).
 *   - Safe fallback values are returned when Redis is unavailable.
 *   - No secret material (URLs, tokens) appears in log fields.
 */

// ---------------------------------------------------------------------------
// Mocks — must be declared before any imports that reference the mocked modules
// ---------------------------------------------------------------------------

const mockLogger = {
  info: jest.fn(),
  warn: jest.fn(),
  error: jest.fn(),
  debug: jest.fn(),
};

jest.mock('../../src/utils/logger', () => ({
  __esModule: true,
  default: mockLogger,
}));

// Minimal Redis mock — re-created per test via the factory below.
const mockMget = jest.fn();
const mockIncr = jest.fn();
const mockPipelineExec = jest.fn();
const mockPipelineSet = jest.fn();
const mockPipelineDel = jest.fn();
const mockPipelineExpire = jest.fn();

const mockPipeline = {
  set: mockPipelineSet,
  del: mockPipelineDel,
  expire: mockPipelineExpire,
  exec: mockPipelineExec,
};

mockPipelineSet.mockReturnValue(mockPipeline);
mockPipelineDel.mockReturnValue(mockPipeline);
mockPipelineExpire.mockReturnValue(mockPipeline);

const mockRedis = {
  mget: mockMget,
  incr: mockIncr,
  pipeline: jest.fn(() => mockPipeline),
};

// ---------------------------------------------------------------------------
// Subject under test
// ---------------------------------------------------------------------------

import {
  WebhookCircuitBreakerStore,
  CircuitStatus,
} from '../../src/redis/webhookCircuitBreakerStore';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const CONSUMER_KEY = 'consumer-abc123';
const FAILURE_THRESHOLD = 3;
const OPEN_TTL_SECONDS = 30;

function makeStore(): WebhookCircuitBreakerStore {
  return new WebhookCircuitBreakerStore(mockRedis as never, {
    failureThreshold: FAILURE_THRESHOLD,
    openTtlSeconds: OPEN_TTL_SECONDS,
  });
}

// ---------------------------------------------------------------------------
// Setup / teardown
// ---------------------------------------------------------------------------

beforeEach(() => {
  jest.clearAllMocks();
  mockPipelineSet.mockReturnValue(mockPipeline);
  mockPipelineDel.mockReturnValue(mockPipeline);
  mockPipelineExpire.mockReturnValue(mockPipeline);
  mockPipelineExec.mockResolvedValue([]);
});

// ---------------------------------------------------------------------------
// getStatus
// ---------------------------------------------------------------------------

describe('getStatus', () => {
  it('returns CLOSED with zero failures when no Redis entry exists', async () => {
    mockMget.mockResolvedValue([null, null, null]);
    const store = makeStore();

    const status = await store.getStatus(CONSUMER_KEY);

    expect(status).toEqual<CircuitStatus>({
      state: 'CLOSED',
      failures: 0,
      probeAt: null,
    });
    expect(mockLogger.error).not.toHaveBeenCalled();
  });

  it('returns parsed state from Redis when data exists', async () => {
    const probeAt = Date.now() + 10_000;
    mockMget.mockResolvedValue(['OPEN', '5', String(probeAt)]);
    const store = makeStore();

    const status = await store.getStatus(CONSUMER_KEY);

    expect(status).toEqual<CircuitStatus>({
      state: 'OPEN',
      failures: 5,
      probeAt,
    });
  });

  it('returns HALF_OPEN state when stored in Redis', async () => {
    mockMget.mockResolvedValue(['HALF_OPEN', '3', null]);
    const store = makeStore();

    const status = await store.getStatus(CONSUMER_KEY);

    expect(status.state).toBe('HALF_OPEN');
    expect(status.failures).toBe(3);
    expect(status.probeAt).toBeNull();
  });

  it('uses structured logger on Redis failure and does NOT use console.error', async () => {
    const redisError = new Error('Redis connection refused');
    mockMget.mockRejectedValue(redisError);
    const consoleSpy = jest.spyOn(console, 'error').mockImplementation(() => undefined);
    const store = makeStore();

    const status = await store.getStatus(CONSUMER_KEY);

    // Fallback: fail open
    expect(status).toEqual<CircuitStatus>({
      state: 'CLOSED',
      failures: 0,
      probeAt: null,
    });

    expect(mockLogger.error).toHaveBeenCalledTimes(1);
    expect(mockLogger.error).toHaveBeenCalledWith(
      'Circuit breaker: failed to read state from Redis',
      expect.objectContaining({
        consumerKey: CONSUMER_KEY,
        operation: 'getStatus',
        error: redisError.message,
      })
    );
    expect(consoleSpy).not.toHaveBeenCalled();

    consoleSpy.mockRestore();
  });

  it('includes consumerKey but no secret material in error log fields', async () => {
    mockMget.mockRejectedValue(new Error('timeout'));
    const store = makeStore();

    await store.getStatus(CONSUMER_KEY);

    const [, logContext] = mockLogger.error.mock.calls[0] as [string, Record<string, unknown>];
    expect(logContext).toHaveProperty('consumerKey', CONSUMER_KEY);
    // Ensure no field that might carry a URL or token is present
    expect(Object.keys(logContext)).not.toContain('url');
    expect(Object.keys(logContext)).not.toContain('token');
    expect(Object.keys(logContext)).not.toContain('secret');
  });
});

// ---------------------------------------------------------------------------
// recordFailure
// ---------------------------------------------------------------------------

describe('recordFailure', () => {
  it('increments failure count and returns CLOSED when below threshold', async () => {
    mockIncr.mockResolvedValue(1);
    const store = makeStore();

    const status = await store.recordFailure(CONSUMER_KEY);

    expect(status.state).toBe('CLOSED');
    expect(status.failures).toBe(1);
    expect(mockLogger.info).toHaveBeenCalledWith(
      'Circuit breaker: failure recorded',
      expect.objectContaining({ consumerKey: CONSUMER_KEY, failures: 1 })
    );
    expect(mockPipelineExec).not.toHaveBeenCalled();
  });

  it('opens the circuit when failures reach the threshold', async () => {
    mockIncr.mockResolvedValue(FAILURE_THRESHOLD);
    const store = makeStore();

    const now = Date.now();
    const status = await store.recordFailure(CONSUMER_KEY);

    expect(status.state).toBe('OPEN');
    expect(status.failures).toBe(FAILURE_THRESHOLD);
    expect(status.probeAt).toBeGreaterThanOrEqual(now + OPEN_TTL_SECONDS * 1_000);
    expect(mockPipelineExec).toHaveBeenCalled();
    expect(mockLogger.warn).toHaveBeenCalledWith(
      'Circuit breaker: circuit opened due to excessive failures',
      expect.objectContaining({
        consumerKey: CONSUMER_KEY,
        failures: FAILURE_THRESHOLD,
        threshold: FAILURE_THRESHOLD,
      })
    );
  });

  it('opens the circuit when failures exceed the threshold', async () => {
    mockIncr.mockResolvedValue(FAILURE_THRESHOLD + 2);
    const store = makeStore();

    const status = await store.recordFailure(CONSUMER_KEY);

    expect(status.state).toBe('OPEN');
  });

  it('uses structured logger on Redis failure and does NOT use console.error', async () => {
    const redisError = new Error('ECONNRESET');
    mockIncr.mockRejectedValue(redisError);
    const consoleSpy = jest.spyOn(console, 'error').mockImplementation(() => undefined);
    const store = makeStore();

    const status = await store.recordFailure(CONSUMER_KEY);

    expect(status).toEqual<CircuitStatus>({
      state: 'CLOSED',
      failures: 0,
      probeAt: null,
    });
    expect(mockLogger.error).toHaveBeenCalledTimes(1);
    expect(mockLogger.error).toHaveBeenCalledWith(
      'Circuit breaker: failed to record failure in Redis',
      expect.objectContaining({
        consumerKey: CONSUMER_KEY,
        operation: 'recordFailure',
        error: redisError.message,
      })
    );
    expect(consoleSpy).not.toHaveBeenCalled();

    consoleSpy.mockRestore();
  });

  it('sets pipeline TTL on circuit open', async () => {
    mockIncr.mockResolvedValue(FAILURE_THRESHOLD);
    const store = makeStore();

    await store.recordFailure(CONSUMER_KEY);

    expect(mockPipelineExpire).toHaveBeenCalledWith(
      expect.stringContaining(CONSUMER_KEY),
      OPEN_TTL_SECONDS * 2
    );
  });
});

// ---------------------------------------------------------------------------
// allowProbe
// ---------------------------------------------------------------------------

describe('allowProbe', () => {
  it('returns false when circuit is CLOSED', async () => {
    mockMget.mockResolvedValue([null, null, null]);
    const store = makeStore();

    const allowed = await store.allowProbe(CONSUMER_KEY);

    expect(allowed).toBe(false);
    expect(mockPipelineExec).not.toHaveBeenCalled();
  });

  it('returns true immediately when circuit is already HALF_OPEN', async () => {
    mockMget.mockResolvedValue(['HALF_OPEN', '3', null]);
    const store = makeStore();

    const allowed = await store.allowProbe(CONSUMER_KEY);

    expect(allowed).toBe(true);
    expect(mockPipelineExec).not.toHaveBeenCalled();
  });

  it('returns false when circuit is OPEN but probeAt has not elapsed', async () => {
    const futureProbeAt = Date.now() + 60_000;
    mockMget.mockResolvedValue(['OPEN', '5', String(futureProbeAt)]);
    const store = makeStore();

    const allowed = await store.allowProbe(CONSUMER_KEY);

    expect(allowed).toBe(false);
    expect(mockPipelineExec).not.toHaveBeenCalled();
  });

  it('transitions to HALF_OPEN when OPEN and probeAt has elapsed', async () => {
    const pastProbeAt = Date.now() - 1_000;
    mockMget.mockResolvedValue(['OPEN', '5', String(pastProbeAt)]);
    const store = makeStore();

    const allowed = await store.allowProbe(CONSUMER_KEY);

    expect(allowed).toBe(true);
    expect(mockPipelineSet).toHaveBeenCalledWith(
      expect.stringContaining(CONSUMER_KEY),
      'HALF_OPEN'
    );
    expect(mockPipelineExec).toHaveBeenCalled();
    expect(mockLogger.info).toHaveBeenCalledWith(
      'Circuit breaker: transitioned to HALF_OPEN for probe',
      expect.objectContaining({ consumerKey: CONSUMER_KEY })
    );
  });

  it('uses structured logger on Redis pipeline failure and does NOT use console.error', async () => {
    // getStatus succeeds (OPEN, probe window elapsed), but the HALF_OPEN pipeline fails.
    const pastProbeAt = Date.now() - 1_000;
    mockMget.mockResolvedValue(['OPEN', '5', String(pastProbeAt)]);
    const redisError = new Error('Redis unavailable');
    mockPipelineExec.mockRejectedValue(redisError);
    const consoleSpy = jest.spyOn(console, 'error').mockImplementation(() => undefined);
    const store = makeStore();

    const allowed = await store.allowProbe(CONSUMER_KEY);

    expect(allowed).toBe(false);
    expect(mockLogger.error).toHaveBeenCalledWith(
      'Circuit breaker: failed to transition to HALF_OPEN in Redis',
      expect.objectContaining({
        consumerKey: CONSUMER_KEY,
        operation: 'allowProbe',
        error: redisError.message,
      })
    );
    expect(consoleSpy).not.toHaveBeenCalled();

    consoleSpy.mockRestore();
  });
});

// ---------------------------------------------------------------------------
// reset
// ---------------------------------------------------------------------------

describe('reset', () => {
  it('deletes all Redis keys for the consumer', async () => {
    const store = makeStore();

    await store.reset(CONSUMER_KEY);

    expect(mockPipelineDel).toHaveBeenCalledTimes(3);
    expect(mockPipelineDel).toHaveBeenCalledWith(
      `webhook:cb:${CONSUMER_KEY}:state`
    );
    expect(mockPipelineDel).toHaveBeenCalledWith(
      `webhook:cb:${CONSUMER_KEY}:failures`
    );
    expect(mockPipelineDel).toHaveBeenCalledWith(
      `webhook:cb:${CONSUMER_KEY}:probe_at`
    );
    expect(mockLogger.info).toHaveBeenCalledWith(
      'Circuit breaker: circuit reset to CLOSED',
      expect.objectContaining({ consumerKey: CONSUMER_KEY })
    );
  });

  it('uses structured logger on Redis failure and does NOT use console.error', async () => {
    const redisError = new Error('pipeline failed');
    mockPipelineExec.mockRejectedValue(redisError);
    const consoleSpy = jest.spyOn(console, 'error').mockImplementation(() => undefined);
    const store = makeStore();

    await expect(store.reset(CONSUMER_KEY)).resolves.toBeUndefined();

    expect(mockLogger.error).toHaveBeenCalledTimes(1);
    expect(mockLogger.error).toHaveBeenCalledWith(
      'Circuit breaker: failed to reset circuit in Redis',
      expect.objectContaining({
        consumerKey: CONSUMER_KEY,
        operation: 'reset',
        error: redisError.message,
      })
    );
    expect(consoleSpy).not.toHaveBeenCalled();

    consoleSpy.mockRestore();
  });
});

// ---------------------------------------------------------------------------
// Non-Error thrown values (covers the `String(error)` branch in each catch)
// ---------------------------------------------------------------------------

describe('non-Error thrown values', () => {
  it('getStatus: logs String(error) when a non-Error is thrown', async () => {
    mockMget.mockRejectedValue('connection string error');
    const store = makeStore();

    const status = await store.getStatus(CONSUMER_KEY);

    expect(status.state).toBe('CLOSED');
    expect(mockLogger.error).toHaveBeenCalledWith(
      'Circuit breaker: failed to read state from Redis',
      expect.objectContaining({ error: 'connection string error' })
    );
  });

  it('recordFailure: logs String(error) when a non-Error is thrown', async () => {
    mockIncr.mockRejectedValue({ code: 'ECONNRESET' });
    const store = makeStore();

    const status = await store.recordFailure(CONSUMER_KEY);

    expect(status.state).toBe('CLOSED');
    expect(mockLogger.error).toHaveBeenCalledWith(
      'Circuit breaker: failed to record failure in Redis',
      expect.objectContaining({ error: '[object Object]' })
    );
  });

  it('allowProbe: logs String(error) when a non-Error is thrown in pipeline', async () => {
    const pastProbeAt = Date.now() - 1_000;
    mockMget.mockResolvedValue(['OPEN', '5', String(pastProbeAt)]);
    mockPipelineExec.mockRejectedValue('pipeline timeout');
    const store = makeStore();

    const allowed = await store.allowProbe(CONSUMER_KEY);

    expect(allowed).toBe(false);
    expect(mockLogger.error).toHaveBeenCalledWith(
      'Circuit breaker: failed to transition to HALF_OPEN in Redis',
      expect.objectContaining({ error: 'pipeline timeout' })
    );
  });

  it('reset: logs String(error) when a non-Error is thrown', async () => {
    mockPipelineExec.mockRejectedValue(42);
    const store = makeStore();

    await expect(store.reset(CONSUMER_KEY)).resolves.toBeUndefined();

    expect(mockLogger.error).toHaveBeenCalledWith(
      'Circuit breaker: failed to reset circuit in Redis',
      expect.objectContaining({ error: '42' })
    );
  });
});

// ---------------------------------------------------------------------------
// Edge: OPEN circuit with null probeAt (allowProbe skips the time check)
// ---------------------------------------------------------------------------

describe('allowProbe with null probeAt', () => {
  it('allows probe immediately when circuit is OPEN with no probeAt timestamp', async () => {
    mockMget.mockResolvedValue(['OPEN', '3', null]);
    const store = makeStore();

    const allowed = await store.allowProbe(CONSUMER_KEY);

    expect(allowed).toBe(true);
    expect(mockPipelineSet).toHaveBeenCalledWith(
      expect.stringContaining(CONSUMER_KEY),
      'HALF_OPEN'
    );
  });
});

// ---------------------------------------------------------------------------
// Default option values
// ---------------------------------------------------------------------------

describe('default options', () => {
  it('uses a failure threshold of 5 by default', async () => {
    mockIncr.mockResolvedValue(5);
    const store = new WebhookCircuitBreakerStore(mockRedis as never);

    const status = await store.recordFailure(CONSUMER_KEY);

    expect(status.state).toBe('OPEN');
  });

  it('does not open the circuit with 4 failures under default threshold', async () => {
    mockIncr.mockResolvedValue(4);
    const store = new WebhookCircuitBreakerStore(mockRedis as never);

    const status = await store.recordFailure(CONSUMER_KEY);

    expect(status.state).toBe('CLOSED');
  });
});
