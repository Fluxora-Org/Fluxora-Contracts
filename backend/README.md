# MentorsMind Backend API

Mentor recommendation engine API with caching, event logging, and ML-ready data collection.

## Features

- **Weighted Scoring Algorithm**: Skill match (40%), rating (30%), availability (20%), price fit (10%)
- **Caching**: Redis or in-memory cache with 1-hour TTL
- **Event Logging**: Impressions, clicks, dismissals stored for ML training
- **RESTful API**: Express.js with TypeORM and PostgreSQL

## Quick Start

```bash
# Install dependencies
npm install

# Setup environment
cp .env.example .env
# Edit .env with your database credentials

# Run migrations
npm run migrate

# Start development server
npm run dev
```

## API Endpoints

### Recommendations

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/v1/users/recommendations/mentors` | Get top 5 mentor recommendations |
| POST | `/api/v1/users/recommendations/dismiss/:mentorId` | Dismiss a recommendation |
| POST | `/api/v1/users/recommendations/click/:mentorId` | Log a click event |

### Example Response

```json
{
  "success": true,
  "data": {
    "recommendations": [
      {
        "mentorId": "uuid",
        "mentor": { ... },
        "totalScore": 0.89,
        "scoreBreakdown": {
          "skillMatch": 0.85,
          "rating": 0.95,
          "availability": 0.90,
          "priceFit": 0.75
        },
        "sessionCount": 1,
        "rank": 1
      }
    ],
    "meta": {
      "cachedAt": "2024-01-15T10:30:00Z",
      "cacheHit": false,
      "count": 5
    }
  }
}
```

## Architecture

```
backend/
├── src/
│   ├── config/          # Database configuration
│   ├── controllers/     # Request handlers
│   ├── middleware/      # Auth, validation
│   ├── models/          # TypeORM entities
│   ├── routes/          # API routes
│   ├── services/        # Business logic
│   └── utils/           # Logger, cache, errors
├── database/migrations/ # SQL migrations
└── package.json
```

## Scoring Algorithm

1. **Skill Match (40%)**: Jaccard similarity between learner goals/gaps and mentor skills
2. **Rating (30%)**: Normalized from 4.0-5.0 scale
3. **Availability (20%)**: Based on schedule slots or boolean flag
4. **Price Fit (10%)**: Budget-to-price ratio, capped at 1.0

## Database Schema

### recommendation_events
Stores interaction data for ML training:
- `learner_id` - User receiving recommendations
- `mentor_id` - Recommended mentor
- `event_type` - impression, click, or dismiss
- `score` - Computed recommendation score
- `score_breakdown` - JSON of component scores
- `session_count` - Prior sessions between learner/mentor
- `rank_position` - Position in recommendation list

## Development

```bash
# Type check
npx tsc --noEmit

# Build for production
npm run build

# Start production
npm start
```

## Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `NODE_ENV` | Environment | development |
| `PORT` | Server port | 3000 |
| `DB_HOST` | PostgreSQL host | localhost |
| `DB_PORT` | PostgreSQL port | 5432 |
| `USE_REDIS` | Enable Redis cache | false |
| `JWT_SECRET` | JWT signing key | - |
