import { Request, Response, NextFunction } from 'express';
import { recommendationService } from '../services/recommendation.service';
import { NotFoundError, ValidationError } from '../utils/errors';
import logger from '../utils/logger';

// Extend Express Request to include user from auth middleware
declare global {
  namespace Express {
    interface Request {
      user?: {
        id: string;
        email: string;
        role: string;
      };
    }
  }
}

/**
 * Validate UUID format
 */
function isValidUUID(value: string): boolean {
  const uuidRegex = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
  return uuidRegex.test(value);
}

/**
 * GET /api/v1/recommendations/mentors
 * Get mentor recommendations for the authenticated learner
 */
export async function getRecommendations(
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> {
  try {
    // Extract learnerId from authenticated user
    const learnerId = req.user?.id;
    if (!learnerId) {
      throw new ValidationError('User not authenticated');
    }

    const result = await recommendationService.getRecommendations(learnerId);

    res.status(200).json({
      success: true,
      data: {
        recommendations: result.recommendations,
        meta: {
          cachedAt: result.cachedAt.toISOString(),
          cacheHit: result.cacheHit,
          count: result.recommendations.length
        }
      }
    });
  } catch (error) {
    if (error instanceof NotFoundError) {
      res.status(404).json({
        success: false,
        error: {
          message: error.message,
          code: 'LEARNER_NOT_FOUND'
        }
      });
      return;
    }
    next(error);
  }
}

/**
 * POST /api/v1/recommendations/dismiss/:mentorId
 * Dismiss a mentor recommendation
 */
export async function dismissRecommendation(
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> {
  try {
    // Extract learnerId from authenticated user
    const learnerId = req.user?.id;
    if (!learnerId) {
      throw new ValidationError('User not authenticated');
    }

    // Extract and validate mentorId from params
    const { mentorId } = req.params;
    if (!mentorId || !isValidUUID(mentorId)) {
      throw new ValidationError('Invalid mentor ID format');
    }

    await recommendationService.dismissRecommendation(learnerId, mentorId);

    // Return 204 No Content on success
    res.status(204).send();
  } catch (error) {
    if (error instanceof NotFoundError) {
      res.status(404).json({
        success: false,
        error: {
          message: error.message,
          code: 'MENTOR_NOT_FOUND'
        }
      });
      return;
    }
    if (error instanceof ValidationError) {
      res.status(400).json({
        success: false,
        error: {
          message: error.message,
          code: 'VALIDATION_ERROR'
        }
      });
      return;
    }
    next(error);
  }
}

/**
 * POST /api/v1/recommendations/click/:mentorId
 * Log a recommendation click (fire and forget)
 */
export async function logClick(
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> {
  try {
    // Extract learnerId from authenticated user
    const learnerId = req.user?.id;
    if (!learnerId) {
      // Return 401 but still fire the logging if possible
      // Actually, we can't log without learnerId, so just return 401
      res.status(401).json({
        success: false,
        error: {
          message: 'User not authenticated',
          code: 'UNAUTHORIZED'
        }
      });
      return;
    }

    // Extract and validate mentorId from params
    const { mentorId } = req.params;
    if (!mentorId || !isValidUUID(mentorId)) {
      res.status(400).json({
        success: false,
        error: {
          message: 'Invalid mentor ID format',
          code: 'VALIDATION_ERROR'
        }
      });
      return;
    }

    // Extract optional rank from body
    const rank = req.body?.rank;
    const rankNumber = typeof rank === 'number' && rank >= 1 && rank <= 5
      ? Math.floor(rank)
      : null;

    // Fire and forget - do not await
    recommendationService.logRecommendationClick(learnerId, mentorId, rankNumber)
      .catch(error => {
        logger.error('Click logging failed silently', { learnerId, mentorId, error });
      });

    // Return 204 immediately - do not wait for logging
    res.status(204).send();
  } catch (error) {
    // Never return error to client for click logging
    // Log the error and return 204 anyway
    logger.error('Error in logClick controller', { error });
    res.status(204).send();
  }
}
