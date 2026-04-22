import { Entity, PrimaryGeneratedColumn, Column, CreateDateColumn, UpdateDateColumn } from 'typeorm';

export enum UserRole {
  LEARNER = 'learner',
  MENTOR = 'mentor',
  ADMIN = 'admin'
}

@Entity('users')
export class User {
  @PrimaryGeneratedColumn('uuid')
  id!: string;

  @Column({ type: 'varchar', length: 255 })
  email!: string;

  @Column({ type: 'varchar', length: 255 })
  name!: string;

  @Column({ type: 'varchar', length: 255 })
  password!: string;

  @Column({
    type: 'enum',
    enum: UserRole,
    default: UserRole.LEARNER
  })
  role!: UserRole;

  // Learner-specific fields
  @Column({ type: 'simple-array', nullable: true })
  goals!: string[] | null;

  @Column({ type: 'simple-array', nullable: true })
  skillGaps!: string[] | null;

  @Column({ type: 'decimal', precision: 10, scale: 2, nullable: true })
  budget!: number | null;

  @Column({ type: 'varchar', length: 50, nullable: true })
  pricePreference!: string | null;

  @CreateDateColumn()
  createdAt!: Date;

  @UpdateDateColumn()
  updatedAt!: Date;
}

// Type for public profile (no sensitive data)
export interface UserPublicProfile {
  id: string;
  name: string;
  email: string;
  role: UserRole;
  goals: string[] | null;
  skillGaps: string[] | null;
  budget: number | null;
  pricePreference: string | null;
  createdAt: Date;
}
