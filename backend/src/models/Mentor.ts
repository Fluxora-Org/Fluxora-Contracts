import { Entity, PrimaryGeneratedColumn, Column, CreateDateColumn, UpdateDateColumn, Index } from 'typeorm';

@Entity('mentors')
export class Mentor {
  @PrimaryGeneratedColumn('uuid')
  id!: string;

  @Index()
  @Column({ type: 'uuid' })
  userId!: string;

  @Column({ type: 'simple-array', nullable: true })
  skills!: string[] | null;

  @Column({ type: 'decimal', precision: 3, scale: 2, default: 0 })
  averageRating!: number;

  @Column({ type: 'decimal', precision: 10, scale: 2 })
  hourlyRate!: number;

  // Availability can be stored as:
  // 1. Boolean (available/not available)
  // 2. Array of time slots (JSON)
  // 3. Number of available slots
  @Column({ type: 'boolean', default: true })
  isAvailable!: boolean;

  @Column({ type: 'json', nullable: true })
  availabilitySlots!: { day: string; startTime: string; endTime: string }[] | null;

  @Column({ type: 'integer', default: 0 })
  availabilityCount!: number;

  @Column({ type: 'text', nullable: true })
  bio!: string | null;

  @CreateDateColumn()
  createdAt!: Date;

  @UpdateDateColumn()
  updatedAt!: Date;
}

// Public profile type for recommendations (no internal/sensitive data)
export interface MentorPublicProfile {
  id: string;
  userId: string;
  skills: string[] | null;
  averageRating: number;
  hourlyRate: number;
  isAvailable: boolean;
  bio: string | null;
  createdAt: Date;
}
