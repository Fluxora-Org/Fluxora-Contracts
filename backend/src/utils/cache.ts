import Redis from 'ioredis';
import logger from './logger';

// Cache client configuration
const REDIS_URL = process.env.REDIS_URL || 'redis://localhost:6379';

let redisClient: Redis | null = null;

// TODO: Replace with Redis for production. Current Map-based cache is for development only.
class InMemoryCache {
  private cache: Map<string, { value: string; expiry: number }> = new Map();

  async get(key: string): Promise<string | null> {
    const item = this.cache.get(key);
    if (!item) return null;
    
    if (Date.now() > item.expiry) {
      this.cache.delete(key);
      return null;
    }
    
    return item.value;
  }

  async set(key: string, value: string, ttlSeconds: number): Promise<void> {
    const expiry = Date.now() + (ttlSeconds * 1000);
    this.cache.set(key, { value, expiry });
  }

  async del(key: string): Promise<void> {
    this.cache.delete(key);
  }
}

class CacheWrapper {
  private client: Redis | InMemoryCache;
  private useRedis: boolean;

  constructor() {
    this.useRedis = process.env.USE_REDIS === 'true';
    
    if (this.useRedis) {
      try {
        redisClient = new Redis(REDIS_URL);
        this.client = redisClient;
        logger.info('Connected to Redis');
      } catch (error) {
        logger.warn('Redis connection failed, falling back to in-memory cache');
        this.client = new InMemoryCache();
        this.useRedis = false;
      }
    } else {
      this.client = new InMemoryCache();
      logger.info('Using in-memory cache (set USE_REDIS=true for Redis)');
    }
  }

  async get<T>(key: string): Promise<T | null> {
    try {
      const data = await this.client.get(key);
      if (!data) return null;
      return JSON.parse(data) as T;
    } catch (error) {
      logger.error('Cache get error', { key, error });
      return null;
    }
  }

  async set<T>(key: string, value: T, ttlSeconds: number): Promise<void> {
    try {
      const serialized = JSON.stringify(value);
      await this.client.set(key, serialized, ttlSeconds);
    } catch (error) {
      logger.error('Cache set error', { key, error });
    }
  }

  async del(key: string): Promise<void> {
    try {
      await this.client.del(key);
    } catch (error) {
      logger.error('Cache delete error', { key, error });
    }
  }

  // In-memory cache TTL cleanup interval
  startCleanupInterval(): void {
    if (!this.useRedis) {
      setInterval(() => {
        // Cleanup is handled in get() for in-memory cache
      }, 60000);
    }
  }
}

const cache = new CacheWrapper();
export default cache;
