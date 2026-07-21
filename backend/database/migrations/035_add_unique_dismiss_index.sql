-- Migration: Add unique constraint to prevent duplicate DISMISS events
-- Created: 2026-07-21

CREATE UNIQUE INDEX IF NOT EXISTS idx_unique_learner_mentor_dismiss 
    ON recommendation_events(learner_id, mentor_id) 
    WHERE event_type = 'dismiss';
