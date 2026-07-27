import { IdempotencyHelper, ICacheStore, IdempotencyError, Logger } from '../src/idempotency';

class InMemoryCache implements ICacheStore {
  private cache = new Map<string, any>();
  private locks = new Set<string>();

  async get<T>(key: string): Promise<{ response: T; statusCode: number } | null> {
    return this.cache.get(key) || null;
  }

  async set<T>(key: string, entry: { response: T; statusCode: number }): Promise<void> {
    this.cache.set(key, entry);
  }

  async acquireLock(key: string): Promise<boolean> {
    if (this.locks.has(key)) return false;
    this.locks.add(key);
    return true;
  }

  async releaseLock(key: string): Promise<void> {
    this.locks.delete(key);
  }
}

const dummyLogger: Logger = {
  info: jest.fn(),
  warn: jest.fn(),
  error: jest.fn()
};

describe('IdempotencyHelper', () => {
  let store: InMemoryCache;
  let helper: IdempotencyHelper;

  beforeEach(() => {
    store = new InMemoryCache();
    helper = new IdempotencyHelper(store, dummyLogger);
    jest.clearAllMocks();
  });

  it('executes a fresh operation successfully', async () => {
    const operation = jest.fn().mockResolvedValue({ response: { id: 1 }, statusCode: 201 });
    
    const result = await helper.execute({ idempotencyKey: 'key-1', callerId: 'user-a' }, operation);
    
    expect(result.cached).toBe(false);
    expect(result.statusCode).toBe(201);
    expect(result.response).toEqual({ id: 1 });
    expect(operation).toHaveBeenCalledTimes(1);
    expect(dummyLogger.info).toHaveBeenCalledWith('Executing fresh operation', expect.any(Object));
  });

  it('returns cached response on replay', async () => {
    const operation = jest.fn().mockResolvedValue({ response: { id: 1 }, statusCode: 201 });
    
    await helper.execute({ idempotencyKey: 'key-1', callerId: 'user-a' }, operation);
    const replayResult = await helper.execute({ idempotencyKey: 'key-1', callerId: 'user-a' }, operation);
    
    expect(replayResult.cached).toBe(true);
    expect(replayResult.statusCode).toBe(201);
    expect(replayResult.response).toEqual({ id: 1 });
    expect(operation).toHaveBeenCalledTimes(1); // Not called again
    expect(dummyLogger.info).toHaveBeenCalledWith('Idempotency cache hit', expect.any(Object));
  });

  it('isolates keys by callerId', async () => {
    const operationA = jest.fn().mockResolvedValue({ response: { owner: 'A' }, statusCode: 200 });
    const operationB = jest.fn().mockResolvedValue({ response: { owner: 'B' }, statusCode: 200 });
    
    await helper.execute({ idempotencyKey: 'shared-key', callerId: 'user-a' }, operationA);
    const resultB = await helper.execute({ idempotencyKey: 'shared-key', callerId: 'user-b' }, operationB);
    
    expect(resultB.cached).toBe(false);
    expect(resultB.response).toEqual({ owner: 'B' });
    expect(operationB).toHaveBeenCalledTimes(1);
  });

  it('rejects invalid idempotency keys', async () => {
    const operation = jest.fn();
    
    await expect(helper.execute({ idempotencyKey: '', callerId: 'user-a' }, operation))
      .rejects.toThrow(IdempotencyError);
    
    await expect(helper.execute({ idempotencyKey: '   ', callerId: 'user-a' }, operation))
      .rejects.toThrow(IdempotencyError);
  });

  it('throws CONCURRENT_REQUEST when a lock is already held', async () => {
    const operation = jest.fn().mockImplementation(() => new Promise(resolve => setTimeout(resolve, 50)));
    
    // Start first request
    const req1 = helper.execute({ idempotencyKey: 'key-2', callerId: 'user-a' }, operation);
    
    // Start concurrent request before req1 finishes
    const req2 = helper.execute({ idempotencyKey: 'key-2', callerId: 'user-a' }, operation);
    
    await expect(req2).rejects.toThrow(IdempotencyError);
    await expect(req2).rejects.toMatchObject({ code: 'CONCURRENT_REQUEST' });
    
    await req1;
    expect(dummyLogger.warn).toHaveBeenCalledWith('Concurrent request collision', expect.any(Object));
  });
});
