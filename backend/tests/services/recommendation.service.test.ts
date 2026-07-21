/**
 * Unit and concurrency tests for RecommendationService
 */

import { EventType, RecommendationEvent } from '../../src/models/RecommendationEvent';
import { UserRole } from '../../src/models/User';
import { NotFoundError } from '../../src/utils/errors';

// ---------------------------------------------------------------------------
// Module Mocks
// ---------------------------------------------------------------------------

process.env.NODE_ENV = 'test';

const cacheStore = new Map<string, unknown>();

jest.mock('../../src/utils/cache', () => ({
  __esModule: true,
  default: {
    get: jest.fn(async (key: string) => cacheStore.get(key) ?? null),
    set: jest.fn(async (key: string, value: unknown) => { cacheStore.set(key, value); }),
    del: jest.fn(async (key: string) => { cacheStore.delete(key); })
  }
}));

jest.mock('../../src/utils/logger', () => ({
  __esModule: true,
  default: {
    info: jest.fn(),
    warn: jest.fn(),
    error: jest.fn(),
    debug: jest.fn()
  }
}));

const mockUserRepository = {
  findOne: jest.fn()
};

const mockMentorRepository = {
  findOne: jest.fn(),
  createQueryBuilder: jest.fn()
};

const mockEventRepository = {
  findOne: jest.fn(),
  find: jest.fn(),
  save: jest.fn()
};

jest.mock('../../src/config/database', () => ({
  AppDataSource: {
    getRepository: (entity: any) => {
      const entityName = typeof entity === 'function' ? entity.name : String(entity);
      if (entityName === 'User') {
        return mockUserRepository;
      }
      if (entityName === 'Mentor') {
        return mockMentorRepository;
      }
      return mockEventRepository;
    }
  }
}));

// Import RecommendationService after mocks are configured
import { RecommendationService, recommendationService } from '../../src/services/recommendation.service';

describe('RecommendationService', () => {
  let service: RecommendationService;

  beforeEach(() => {
    jest.clearAllMocks();
    cacheStore.clear();
    service = new RecommendationService();
  });

  describe('singleton export', () => {
    it('exports a RecommendationService instance', () => {
      expect(recommendationService).toBeInstanceOf(RecommendationService);
    });
  });

  describe('dismissRecommendation', () => {
    const learnerId = '11111111-1111-1111-1111-111111111111';
    const mentorId = '22222222-2222-2222-2222-222222222222';

    it('should throw NotFoundError if mentor does not exist', async () => {
      mockMentorRepository.findOne.mockResolvedValueOnce(null);

      await expect(service.dismissRecommendation(learnerId, mentorId)).rejects.toThrow(NotFoundError);
      expect(mockMentorRepository.findOne).toHaveBeenCalledWith({ where: { id: mentorId } });
      expect(mockEventRepository.save).not.toHaveBeenCalled();
    });

    it('should ignore duplicate dismissal if pre-check finds an existing dismissal event', async () => {
      mockMentorRepository.findOne.mockResolvedValueOnce({ id: mentorId });
      mockEventRepository.findOne.mockResolvedValueOnce({
        id: 'existing-event-id',
        learnerId,
        mentorId,
        eventType: EventType.DISMISS
      });

      await service.dismissRecommendation(learnerId, mentorId);

      expect(mockMentorRepository.findOne).toHaveBeenCalledWith({ where: { id: mentorId } });
      expect(mockEventRepository.findOne).toHaveBeenCalledWith({
        where: { learnerId, mentorId, eventType: EventType.DISMISS }
      });
      expect(mockEventRepository.save).not.toHaveBeenCalled();
    });

    it('should save dismissal event and invalidate cache on first call', async () => {
      mockMentorRepository.findOne.mockResolvedValueOnce({ id: mentorId });
      mockEventRepository.findOne.mockResolvedValueOnce(null);
      mockEventRepository.save.mockResolvedValueOnce({
        id: 'new-event-id',
        learnerId,
        mentorId,
        eventType: EventType.DISMISS
      });

      await service.dismissRecommendation(learnerId, mentorId);

      expect(mockEventRepository.save).toHaveBeenCalledTimes(1);
      const savedEvent = mockEventRepository.save.mock.calls[0][0];
      expect(savedEvent.learnerId).toBe(learnerId);
      expect(savedEvent.mentorId).toBe(mentorId);
      expect(savedEvent.eventType).toBe(EventType.DISMISS);
    });

    it('should handle concurrent dismissRecommendation calls gracefully via DB unique constraint failure (Promise.all)', async () => {
      mockMentorRepository.findOne.mockResolvedValue({ id: mentorId });
      // Pre-check for both calls returns null (simulating TOCTOU window)
      mockEventRepository.findOne.mockResolvedValue(null);

      const savedEvents: RecommendationEvent[] = [];

      // First call save succeeds; second call fails with DB unique constraint violation (code 23505)
      mockEventRepository.save
        .mockImplementationOnce(async (event: RecommendationEvent) => {
          savedEvents.push(event);
          return { ...event, id: 'event-1' };
        })
        .mockImplementationOnce(async () => {
          const dbError: any = new Error('duplicate key value violates unique constraint "idx_unique_learner_mentor_dismiss"');
          dbError.code = '23505';
          throw dbError;
        });

      // Fire 2 concurrent dismiss calls
      await expect(
        Promise.all([
          service.dismissRecommendation(learnerId, mentorId),
          service.dismissRecommendation(learnerId, mentorId)
        ])
      ).resolves.not.toThrow();

      // Exactly 1 DISMISS event was saved
      expect(savedEvents.length).toBe(1);
      expect(savedEvents[0].eventType).toBe(EventType.DISMISS);
      expect(savedEvents[0].learnerId).toBe(learnerId);
      expect(savedEvents[0].mentorId).toBe(mentorId);
    });

    it('should rethrow non-unique constraint errors from save', async () => {
      mockMentorRepository.findOne.mockResolvedValueOnce({ id: mentorId });
      mockEventRepository.findOne.mockResolvedValueOnce(null);

      const fatalError = new Error('Database connection failure');
      mockEventRepository.save.mockRejectedValueOnce(fatalError);

      await expect(service.dismissRecommendation(learnerId, mentorId)).rejects.toThrow('Database connection failure');
    });
  });

  describe('IMPRESSION and CLICK events (unaffected by DISMISS unique constraint)', () => {
    const learnerId = '11111111-1111-1111-1111-111111111111';
    const mentorId = '22222222-2222-2222-2222-222222222222';

    it('allows multiple CLICK events for the same (learnerId, mentorId) pair', async () => {
      mockEventRepository.save.mockResolvedValue({ id: 'click-event' });

      await service.logRecommendationClick(learnerId, mentorId, 1);
      await service.logRecommendationClick(learnerId, mentorId, 2);

      expect(mockEventRepository.save).toHaveBeenCalledTimes(2);
      expect(mockEventRepository.save.mock.calls[0][0].eventType).toBe(EventType.CLICK);
      expect(mockEventRepository.save.mock.calls[1][0].eventType).toBe(EventType.CLICK);
    });

    it('handles logRecommendationClick errors silently', async () => {
      mockEventRepository.save.mockRejectedValueOnce(new Error('DB Error'));

      await expect(service.logRecommendationClick(learnerId, mentorId, 1)).resolves.not.toThrow();
    });
  });

  describe('getRecommendations', () => {
    const learnerId = '11111111-1111-1111-1111-111111111111';

    it('returns cached result on cache hit', async () => {
      const cached = {
        recommendations: [],
        cachedAt: new Date(),
        cacheHit: true
      };
      cacheStore.set(`recommendations:${learnerId}`, cached);

      const result = await service.getRecommendations(learnerId);
      expect(result.cacheHit).toBe(true);
      expect(mockUserRepository.findOne).not.toHaveBeenCalled();
    });

    it('throws NotFoundError if learner is not found', async () => {
      mockUserRepository.findOne.mockResolvedValueOnce(null);

      await expect(service.getRecommendations(learnerId)).rejects.toThrow(NotFoundError);
    });

    it('fetches candidates, scores, ranks and caches recommendations on cache miss', async () => {
      const learner = {
        id: learnerId,
        role: UserRole.LEARNER,
        goals: ['TypeScript', 'Node.js'],
        skillGaps: ['Architecture'],
        budget: 100,
        pricePreference: 'standard'
      };
      mockUserRepository.findOne.mockResolvedValueOnce(learner);

      mockEventRepository.find.mockResolvedValueOnce([{ mentorId: 'dismissed-mentor' }]);

      const mockQueryBuilder = {
        leftJoin: jest.fn().mockReturnThis(),
        from: jest.fn().mockReturnThis(),
        select: jest.fn().mockReturnThis(),
        addSelect: jest.fn().mockReturnThis(),
        where: jest.fn().mockReturnThis(),
        andWhere: jest.fn().mockReturnThis(),
        groupBy: jest.fn().mockReturnThis(),
        getRawAndEntities: jest.fn().mockResolvedValueOnce({
          entities: [
            {
              id: 'mentor-1',
              skills: ['TypeScript', 'Node.js'],
              averageRating: 4.8,
              hourlyRate: 80,
              isAvailable: true,
              availabilitySlots: ['slot1', 'slot2'],
              availabilityCount: 5
            },
            {
              id: 'mentor-2',
              skills: [],
              averageRating: 4.0,
              hourlyRate: 150,
              isAvailable: true,
              availabilitySlots: [],
              availabilityCount: 10
            },
            {
              id: 'mentor-3',
              skills: ['Go'],
              averageRating: 4.2,
              hourlyRate: 200,
              isAvailable: false,
              availabilitySlots: [],
              availabilityCount: 0
            },
            {
              id: 'mentor-4',
              skills: ['TypeScript'],
              averageRating: 4.5,
              hourlyRate: 50,
              isAvailable: true,
              availabilitySlots: [],
              availabilityCount: 0
            }
          ],
          raw: [
            { sessionCount: '1' },
            { sessionCount: '0' },
            { sessionCount: '0' },
            { sessionCount: '3' } // >= MAX_PRIOR_SESSIONS (3) -> filtered out
          ]
        })
      };

      mockMentorRepository.createQueryBuilder.mockReturnValue(mockQueryBuilder);
      mockEventRepository.save.mockResolvedValue([]);

      const result = await service.getRecommendations(learnerId);
      expect(result.cacheHit).toBe(false);
      // mentor-4 is filtered out due to max prior sessions
      expect(result.recommendations.length).toBe(3);
      expect(result.recommendations[0].rank).toBe(1);
    });

    it('handles learner without goals/skillGaps and pricePreference strings', async () => {
      const learner = {
        id: learnerId,
        role: UserRole.LEARNER,
        goals: null,
        skillGaps: null,
        budget: null,
        pricePreference: 'budget'
      };
      mockUserRepository.findOne.mockResolvedValueOnce(learner);
      mockEventRepository.find.mockResolvedValueOnce([]);

      const mockQueryBuilder = {
        leftJoin: jest.fn().mockReturnThis(),
        from: jest.fn().mockReturnThis(),
        select: jest.fn().mockReturnThis(),
        addSelect: jest.fn().mockReturnThis(),
        where: jest.fn().mockReturnThis(),
        andWhere: jest.fn().mockReturnThis(),
        groupBy: jest.fn().mockReturnThis(),
        getRawAndEntities: jest.fn().mockResolvedValueOnce({
          entities: [
            {
              id: 'mentor-1',
              skills: ['Python'],
              averageRating: 4.0,
              hourlyRate: 60,
              isAvailable: true
            }
          ],
          raw: [{ sessionCount: '0' }]
        })
      };

      mockMentorRepository.createQueryBuilder.mockReturnValue(mockQueryBuilder);
      mockEventRepository.save.mockRejectedValueOnce(new Error('Logging error'));

      const result = await service.getRecommendations(learnerId);
      expect(result.recommendations.length).toBe(1);
    });

    it('handles learner with no budget or price preference', async () => {
      const learner = {
        id: learnerId,
        role: UserRole.LEARNER,
        goals: ['Python'],
        skillGaps: [],
        budget: null,
        pricePreference: 'unknown_pref'
      };
      mockUserRepository.findOne.mockResolvedValueOnce(learner);
      mockEventRepository.find.mockResolvedValueOnce([]);

      const mockQueryBuilder = {
        leftJoin: jest.fn().mockReturnThis(),
        from: jest.fn().mockReturnThis(),
        select: jest.fn().mockReturnThis(),
        addSelect: jest.fn().mockReturnThis(),
        where: jest.fn().mockReturnThis(),
        andWhere: jest.fn().mockReturnThis(),
        groupBy: jest.fn().mockReturnThis(),
        getRawAndEntities: jest.fn().mockResolvedValueOnce({
          entities: [
            {
              id: 'mentor-1',
              skills: [],
              averageRating: 4.0,
              hourlyRate: 60,
              isAvailable: true
            }
          ],
          raw: [{ sessionCount: '0' }]
        })
      };

      mockMentorRepository.createQueryBuilder.mockReturnValue(mockQueryBuilder);
      mockEventRepository.save.mockResolvedValue([]);

      const result = await service.getRecommendations(learnerId);
      expect(result.recommendations.length).toBe(1);
    });
  });
});
