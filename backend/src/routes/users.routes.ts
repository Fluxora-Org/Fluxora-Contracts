import { Router } from 'express';
import rateLimit from 'express-rate-limit';
import { authenticate } from '../middleware/auth.middleware';
import { validate } from '../middleware/validation.middleware';
import * as userController from '../controllers/user.controller';
import * as recommendationController from '../controllers/recommendation.controller';

const router = Router();

// Rate limiting configurations
const standardLimiter = rateLimit({
  windowMs: 15 * 60 * 1000, // 15 minutes
  max: 100,
  standardHeaders: true,
  legacyHeaders: false
});

const strictLimiter = rateLimit({
  windowMs: 15 * 60 * 1000, // 15 minutes
  max: 10,
  standardHeaders: true,
  legacyHeaders: false
});

// ============================================================================
// USER ROUTES
// ============================================================================

// Get current user profile
router.get(
  '/me',
  standardLimiter,
  authenticate,
  userController.getCurrentUser
);

// Update user profile
router.patch(
  '/me',
  standardLimiter,
  authenticate,
  validate('updateUser'),
  userController.updateUser
);

// Get user by ID (admin only)
router.get(
  '/:userId',
  standardLimiter,
  authenticate,
  validate('getUserById'),
  userController.getUserById
);

// Delete user account
router.delete(
  '/me',
  strictLimiter,
  authenticate,
  userController.deleteUser
);

// ============================================================================
// LEARNER PROFILE ROUTES
// ============================================================================

// Update learner goals and skill gaps
router.patch(
  '/me/learner-profile',
  standardLimiter,
  authenticate,
  validate('updateLearnerProfile'),
  userController.updateLearnerProfile
);

// Get learner budget/preferences
router.get(
  '/me/budget',
  standardLimiter,
  authenticate,
  userController.getLearnerBudget
);

// ============================================================================
// RECOMMENDATION ROUTES (NEW)
// ============================================================================

// GET /api/v1/recommendations/mentors
// Get mentor recommendations for the authenticated learner
router.get(
  '/recommendations/mentors',
  standardLimiter,
  authenticate,
  recommendationController.getRecommendations
);

// POST /api/v1/recommendations/dismiss/:mentorId
// Dismiss a mentor recommendation
router.post(
  '/recommendations/dismiss/:mentorId',
  standardLimiter,
  authenticate,
  validate('dismissRecommendation'),
  recommendationController.dismissRecommendation
);

// POST /api/v1/recommendations/click/:mentorId
// Log a recommendation click (fire and forget)
router.post(
  '/recommendations/click/:mentorId',
  standardLimiter,
  authenticate,
  recommendationController.logClick
);

// ============================================================================
// ADMIN ROUTES
// ============================================================================

// List all users (admin only)
router.get(
  '/',
  standardLimiter,
  authenticate,
  userController.listUsers
);

export default router;
