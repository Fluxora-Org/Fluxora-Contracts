-- Migration: Create recommendation_events table for ML training data
-- Created: 2024-01-15

-- Up migration
CREATE TABLE IF NOT EXISTS recommendation_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    learner_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    mentor_id UUID NOT NULL REFERENCES mentors(id) ON DELETE CASCADE,
    event_type VARCHAR(50) NOT NULL CHECK (event_type IN ('impression', 'click', 'dismiss')),
    score DECIMAL(5,4),
    score_breakdown JSONB,
    session_count INTEGER,
    rank_position INTEGER,
    dismissed_at TIMESTAMP WITH TIME ZONE,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    metadata JSONB
);

-- Indexes for efficient querying
CREATE INDEX IF NOT EXISTS idx_recommendation_events_learner_created 
    ON recommendation_events(learner_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_recommendation_events_mentor_type 
    ON recommendation_events(mentor_id, event_type);

CREATE INDEX IF NOT EXISTS idx_recommendation_events_learner_mentor_type 
    ON recommendation_events(learner_id, mentor_id, event_type);

-- Comments for documentation
COMMENT ON TABLE recommendation_events IS 'Stores recommendation interactions for ML training and analytics';
COMMENT ON COLUMN recommendation_events.event_type IS 'Type of event: impression (shown), click (selected), dismiss (rejected)';
COMMENT ON COLUMN recommendation_events.score_breakdown IS 'JSON containing {skillMatch, rating, availability, priceFit} scores';
COMMENT ON COLUMN recommendation_events.rank_position IS 'Position in recommendation list (1-5)';

-- Down migration (for rollback)
-- DROP TABLE IF EXISTS recommendation_events;
