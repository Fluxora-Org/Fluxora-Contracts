import { Entity, PrimaryGeneratedColumn, Column, CreateDateColumn, Index } from 'typeorm';

export enum EventType {
  IMPRESSION = 'impression',
  CLICK = 'click',
  DISMISS = 'dismiss'
}

export interface ScoreBreakdown {
  skillMatch: number;
  rating: number;
  availability: number;
  priceFit: number;
}

@Entity('recommendation_events')
@Index(['learnerId', 'createdAt'])
@Index(['mentorId', 'eventType'])
@Index(['learnerId', 'mentorId', 'eventType'])
export class RecommendationEvent {
  @PrimaryGeneratedColumn('uuid')
  id!: string;

  @Index()
  @Column({ type: 'uuid' })
  learnerId!: string;

  @Index()
  @Column({ type: 'uuid' })
  mentorId!: string;

  @Column({
    type: 'enum',
    enum: EventType
  })
  eventType!: EventType;

  @Column({ type: 'decimal', precision: 5, scale: 4, nullable: true })
  score!: number | null;

  @Column({ type: 'jsonb', nullable: true })
  scoreBreakdown!: ScoreBreakdown | null;

  @Column({ type: 'integer', nullable: true })
  sessionCount!: number | null;

  @Column({ type: 'integer', nullable: true })
  rankPosition!: number | null;

  @Column({ type: 'timestamp with time zone', nullable: true })
  dismissedAt!: Date | null;

  @CreateDateColumn()
  createdAt!: Date;

  @Column({ type: 'jsonb', nullable: true })
  metadata!: Record<string, unknown> | null;
}
