import { DataSource } from 'typeorm';
import { User } from '../models/User';
import { Mentor } from '../models/Mentor';
import { Session } from '../models/Session';
import { RecommendationEvent } from '../models/RecommendationEvent';

export const AppDataSource = new DataSource({
  type: 'postgres',
  host: process.env.DB_HOST || 'localhost',
  port: parseInt(process.env.DB_PORT || '5432'),
  username: process.env.DB_USERNAME || 'postgres',
  password: process.env.DB_PASSWORD || 'password',
  database: process.env.DB_DATABASE || 'mentorsmind',
  synchronize: process.env.NODE_ENV === 'development',
  logging: process.env.DB_LOGGING === 'true',
  entities: [User, Mentor, Session, RecommendationEvent],
  migrations: ['database/migrations/*.ts'],
  subscribers: []
});
