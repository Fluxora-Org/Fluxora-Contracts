import { AppDataSource } from '../config/database';
import { User, UserRole } from '../models/User';
import { Mentor, MentorPublicProfile } from '../models/Mentor';
import { Session } from '../models/Session';
import { RecommendationEvent, EventType, ScoreBreakdown } from '../models/RecommendationEvent';
import { NotFoundError } from '../utils/errors';
import cache from '../utils/cache';
import logger from '../utils/logger';

// ============================================================================
// SCORING WEIGHTS - Sum must equal 1.0
// ============================================================================
const WEIGHT_SKILL_MATCH = 0.40;
const WEIGHT_RATING = 0.30;
const WEIGHT_AVAILABILITY = 0.20;
const WEIGHT_PRICE_FIT = 0.10;
// Verify: 0.40 + 0.30 + 0.20 + 0.10 = 1.0 ✓

const MIN_RATING_THRESHOLD = 4.0;
const MAX_PRIOR_SESSIONS = 3;
const TOP_N_RESULTS = 5;
const CACHE_TTL_SECONDS = 3600; // 1 hour

// ============================================================================
// TYPES
// ============================================================================

export interface MentorScore {
  mentorId: string;
  mentor: MentorPublicProfile;
  totalScore: number; // 0.0 – 1.0
  scoreBreakdown: {
    skillMatch: number; // 0.0 – 1.0, weight 40%
    rating: number; // 0.0 – 1.0, weight 30%
    availability: number; // 0.0 – 1.0, weight 20%
    priceFit: number; // 0.0 – 1.0, weight 10%
  };
  sessionCount: number; // sessions learner has had with this mentor
  rank: number; // 1–5
}

export interface RecommendationResult {
  recommendations: MentorScore[];
  cachedAt: Date;
  cacheHit: boolean;
}

interface CandidateMentor {
  mentor: Mentor;
  sessionCount: number;
}

// ============================================================================
// SERVICE CLASS
// ============================================================================

export class RecommendationService {
  private userRepository = AppDataSource.getRepository(User);
  private mentorRepository = AppDataSource.getRepository(Mentor);
  private eventRepository = AppDataSource.getRepository(RecommendationEvent);

  /**
   * Get mentor recommendations for a learner
   * Implements weighted scoring algorithm with caching and event logging
   */
  async getRecommendations(learnerId: string): Promise<RecommendationResult> {
    const cacheKey = `recommendations:${learnerId}`;

    // Step 1: Cache check
    const cachedResult = await cache.get<RecommendationResult>(cacheKey);
    if (cachedResult) {
      logger.debug('Cache hit for recommendations', { learnerId, cacheKey });
      return {
        ...cachedResult,
        cacheHit: true
      };
    }
    logger.debug('Cache miss for recommendations', { learnerId, cacheKey });

    // Step 2: Fetch learner profile
    const learner = await this.userRepository.findOne({
      where: { id: learnerId, role: UserRole.LEARNER }
    });

    if (!learner) {
      logger.warn('Learner not found', { learnerId });
      throw new NotFoundError('Learner');
    }

    // Step 3: Fetch candidate mentors with session counts
    const candidates = await this.fetchCandidateMentors(learnerId);

    // Step 4: Score each candidate
    const scoredMentors = this.scoreMentors(candidates, learner);

    // Step 5: Rank and trim
    const topRecommendations = scoredMentors
      .sort((a, b) => b.totalScore - a.totalScore)
      .slice(0, TOP_N_RESULTS)
      .map((mentor, index) => ({
        ...mentor,
        rank: index + 1
      }));

    // Step 6: Prepare result
    const result: RecommendationResult = {
      recommendations: topRecommendations,
      cachedAt: new Date(),
      cacheHit: false
    };

    // Step 7: Cache result
    await cache.set(cacheKey, result, CACHE_TTL_SECONDS);

    // Step 8: Log impressions (fire and forget)
    this.logImpressions(learnerId, topRecommendations).catch(error => {
      logger.error('Failed to log impression events', { learnerId, error });
    });

    logger.info('Recommendations generated', {
      learnerId,
      count: topRecommendations.length,
      cachedAt: result.cachedAt
    });

    return result;
  }

  /**
   * Dismiss a recommendation
   * Idempotent - no error if already dismissed
   */
  async dismissRecommendation(learnerId: string, mentorId: string): Promise<void> {
    // Verify mentor exists
    const mentor = await this.mentorRepository.findOne({ where: { id: mentorId } });
    if (!mentor) {
      throw new NotFoundError('Mentor');
    }

    // Check if already dismissed (idempotent)
    const existingDismissal = await this.eventRepository.findOne({
      where: {
        learnerId,
        mentorId,
        eventType: EventType.DISMISS
      }
    });

    if (existingDismissal) {
      logger.debug('Duplicate dismissal ignored', { learnerId, mentorId });
      return;
    }

    // Insert dismissal event
    const event = new RecommendationEvent();
    event.learnerId = learnerId;
    event.mentorId = mentorId;
    event.eventType = EventType.DISMISS;
    event.dismissedAt = new Date();
    event.score = null;
    event.scoreBreakdown = null;
    event.sessionCount = null;
    event.rankPosition = null;

    await this.eventRepository.save(event);

    // Invalidate cache
    const cacheKey = `recommendations:${learnerId}`;
    await cache.del(cacheKey);

    logger.info('Recommendation dismissed', { learnerId, mentorId });
  }

  /**
   * Log a recommendation click
   * Fire and forget - never blocks response
   */
  async logRecommendationClick(
    learnerId: string,
    mentorId: string,
    rank: number | null
  ): Promise<void> {
    try {
      const event = new RecommendationEvent();
      event.learnerId = learnerId;
      event.mentorId = mentorId;
      event.eventType = EventType.CLICK;
      event.rankPosition = rank;
      event.score = null;
      event.scoreBreakdown = null;
      event.sessionCount = null;

      await this.eventRepository.save(event);

      logger.debug('Click logged', { learnerId, mentorId, rank });
    } catch (error) {
      // Silently catch - never let logging block the response
      logger.error('Failed to log click', { learnerId, mentorId, error });
    }
  }

  // ============================================================================
  // PRIVATE METHODS
  // ============================================================================

  /**
   * Fetch candidate mentors with session counts in a single query
   * Excludes: dismissed mentors, mentors with too many sessions, low-rated mentors
   */
  private async fetchCandidateMentors(learnerId: string): Promise<CandidateMentor[]> {
    // Get list of dismissed mentor IDs
    const dismissedMentors = await this.eventRepository.find({
      where: {
        learnerId,
        eventType: EventType.DISMISS
      },
      select: ['mentorId']
    });
    const dismissedIds = dismissedMentors.map((e: RecommendationEvent) => e.mentorId);

    // Build query with session count subquery
    const query = this.mentorRepository
      .createQueryBuilder('mentor')
      .leftJoin(
        (qb: ReturnType<typeof this.mentorRepository.createQueryBuilder>) => qb
          .from(Session, 'session')
          .select('session.mentorId', 'mentorId')
          .addSelect('COUNT(*)', 'sessionCount')
          .where('session.learnerId = :learnerId', { learnerId })
          .groupBy('session.mentorId'),
        'sessions',
        'sessions.mentorId = mentor.id'
      )
      .where('mentor.averageRating >= :minRating', { minRating: MIN_RATING_THRESHOLD })
      .andWhere('mentor.isAvailable = :isAvailable', { isAvailable: true });

    if (dismissedIds.length > 0) {
      query.andWhere('mentor.id NOT IN (:...dismissedIds)', { dismissedIds });
    }

    const mentors = await query.getRawAndEntities();

    // Map results with session counts
    return mentors.entities.map((mentor: Mentor, index: number) => {
      const raw = mentors.raw[index] as { sessionCount: string } | undefined;
      const sessionCount = parseInt(raw?.sessionCount || '0', 10);

      // Exclude mentors with too many prior sessions
      if (sessionCount >= MAX_PRIOR_SESSIONS) {
        return null;
      }

      return {
        mentor,
        sessionCount
      };
    }).filter((c: CandidateMentor | null): c is CandidateMentor => c !== null);
  }

  /**
   * Score mentors based on learner profile
   */
  private scoreMentors(candidates: CandidateMentor[], learner: User): MentorScore[] {
    return candidates.map(candidate => {
      const { mentor, sessionCount } = candidate;

      // Calculate skill match score using Jaccard similarity
      const skillMatchScore = this.calculateSkillMatch(
        learner.goals,
        learner.skillGaps,
        mentor.skills
      );

      // Calculate rating score (normalized)
      const ratingScore = this.calculateRatingScore(mentor.averageRating);

      // Calculate availability score
      const availabilityScore = this.calculateAvailabilityScore(mentor);

      // Calculate price fit score
      const priceFitScore = this.calculatePriceFit(
        learner.budget,
        learner.pricePreference,
        mentor.hourlyRate
      );

      // Calculate weighted total score
      const totalScore =
        (skillMatchScore * WEIGHT_SKILL_MATCH) +
        (ratingScore * WEIGHT_RATING) +
        (availabilityScore * WEIGHT_AVAILABILITY) +
        (priceFitScore * WEIGHT_PRICE_FIT);

      // Clamp total score to [0, 1]
      const clampedTotalScore = Math.max(0, Math.min(1, totalScore));

      return {
        mentorId: mentor.id,
        mentor: this.toPublicProfile(mentor),
        totalScore: clampedTotalScore,
        scoreBreakdown: {
          skillMatch: skillMatchScore,
          rating: ratingScore,
          availability: availabilityScore,
          priceFit: priceFitScore
        },
        sessionCount,
        rank: 0 // Will be assigned after sorting
      };
    });
  }

  /**
   * Calculate skill match using Jaccard similarity
   * Score = |intersection| / |union|
   */
  private calculateSkillMatch(
    goals: string[] | null,
    skillGaps: string[] | null,
    mentorSkills: string[] | null
  ): number {
    // Build learner skills set from goals and skillGaps
    const learnerSkills = new Set<string>();

    if (goals?.length) {
      goals.forEach(g => learnerSkills.add(g.toLowerCase().trim()));
    }

    if (skillGaps?.length) {
      skillGaps.forEach(s => learnerSkills.add(s.toLowerCase().trim()));
    }

    // If learner has no skills defined: score = 0.5 (neutral)
    if (learnerSkills.size === 0) {
      return 0.5;
    }

    // If mentor has no skills defined: score = 0.0
    if (!mentorSkills || mentorSkills.length === 0) {
      return 0.0;
    }

    const mentorSkillSet = new Set(
      mentorSkills.map(s => s.toLowerCase().trim())
    );

    // Calculate Jaccard similarity
    const intersection = new Set(
      [...learnerSkills].filter(x => mentorSkillSet.has(x))
    );

    const union = new Set([...learnerSkills, ...mentorSkillSet]);

    if (union.size === 0) {
      return 0.0;
    }

    return intersection.size / union.size;
  }

  /**
   * Calculate rating score (normalized)
   * 5.0 rating → 1.0, 4.0 rating → 0.0
   */
  private calculateRatingScore(averageRating: number): number {
    const normalized = (averageRating - MIN_RATING_THRESHOLD) / (5.0 - MIN_RATING_THRESHOLD);
    return Math.max(0, Math.min(1, normalized));
  }

  /**
   * Calculate availability score based on mentor's availability model
   */
  private calculateAvailabilityScore(mentor: Mentor): number {
    // If boolean availability: 1.0 if available, 0.0 if not
    if (!mentor.isAvailable) {
      return 0.0;
    }

    // If availability slots exist: normalize by count
    if (mentor.availabilitySlots && mentor.availabilitySlots.length > 0) {
      // Max expected slots per week: ~20 (4 slots per day × 5 days)
      const maxSlots = 20;
      return Math.min(1, mentor.availabilitySlots.length / maxSlots);
    }

    // If availability count field exists
    if (mentor.availabilityCount > 0) {
      const maxCount = 40; // Max expected sessions per month
      return Math.min(1, mentor.availabilityCount / maxCount);
    }

    // Default: if isAvailable is true, return neutral-high score
    return 0.7;
  }

  /**
   * Calculate price fit score
   */
  private calculatePriceFit(
    learnerBudget: number | null,
    pricePreference: string | null,
    mentorPrice: number
  ): number {
    // If learner has no budget preference: score = 0.5 (neutral)
    if (!learnerBudget && !pricePreference) {
      return 0.5;
    }

    // Use price preference to estimate budget if explicit budget not set
    let effectiveBudget = learnerBudget;
    if (!effectiveBudget && pricePreference) {
      effectiveBudget = this.parsePricePreference(pricePreference);
    }

    // If still no budget: neutral score
    if (!effectiveBudget) {
      return 0.5;
    }

    // If mentor price <= learner budget: score = 1.0
    if (mentorPrice <= effectiveBudget) {
      return 1.0;
    }

    // If mentor price > learner budget: score decreases
    // score = learner budget / mentor price
    const score = effectiveBudget / mentorPrice;
    return Math.max(0, Math.min(1, score));
  }

  /**
   * Parse price preference string to numeric budget
   */
  private parsePricePreference(preference: string): number | null {
    const ranges: Record<string, number> = {
      'low': 25,
      'budget': 50,
      'medium': 75,
      'standard': 100,
      'premium': 150,
      'high': 200
    };

    const normalized = preference.toLowerCase().trim();
    return ranges[normalized] || null;
  }

  /**
   * Convert Mentor entity to public profile
   */
  private toPublicProfile(mentor: Mentor): MentorPublicProfile {
    return {
      id: mentor.id,
      userId: mentor.userId,
      skills: mentor.skills,
      averageRating: mentor.averageRating,
      hourlyRate: mentor.hourlyRate,
      isAvailable: mentor.isAvailable,
      bio: mentor.bio,
      createdAt: mentor.createdAt
    };
  }

  /**
   * Log impression events in bulk (fire and forget)
   */
  private async logImpressions(
    learnerId: string,
    recommendations: MentorScore[]
  ): Promise<void> {
    try {
      const events = recommendations.map(rec => {
        const event = new RecommendationEvent();
        event.learnerId = learnerId;
        event.mentorId = rec.mentorId;
        event.eventType = EventType.IMPRESSION;
        event.score = rec.totalScore;
        event.scoreBreakdown = rec.scoreBreakdown as ScoreBreakdown;
        event.sessionCount = rec.sessionCount;
        event.rankPosition = rec.rank;
        return event;
      });

      // Bulk insert
      await this.eventRepository.save(events);

      logger.debug('Impression events logged', {
        learnerId,
        count: events.length
      });
    } catch (error) {
      // Silently catch - never let logging block the response
      logger.error('Failed to log impression events', { learnerId, error });
    }
  }
}

// Export singleton instance
export const recommendationService = new RecommendationService();
