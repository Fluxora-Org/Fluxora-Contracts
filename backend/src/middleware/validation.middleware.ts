import { Request, Response, NextFunction } from 'express';
import { ValidationError } from '../utils/errors';

/**
 * Validation schemas for different routes
 */
const validationSchemas: Record<string, ValidationRule[]> = {
  updateUser: [
    { field: 'name', type: 'string', required: false, maxLength: 255 },
    { field: 'email', type: 'email', required: false },
    { field: 'goals', type: 'array', required: false, maxItems: 10 },
    { field: 'skillGaps', type: 'array', required: false, maxItems: 10 },
    { field: 'budget', type: 'number', required: false, min: 0 }
  ],
  getUserById: [
    { field: 'userId', type: 'uuid', required: true, param: true }
  ],
  updateLearnerProfile: [
    { field: 'goals', type: 'array', required: false, maxItems: 10 },
    { field: 'skillGaps', type: 'array', required: false, maxItems: 10 },
    { field: 'budget', type: 'number', required: false, min: 0 },
    { field: 'pricePreference', type: 'string', required: false, enum: ['low', 'budget', 'medium', 'standard', 'premium', 'high'] }
  ],
  dismissRecommendation: [
    { field: 'mentorId', type: 'uuid', required: true, param: true }
  ]
};

interface ValidationRule {
  field: string;
  type: 'string' | 'number' | 'boolean' | 'array' | 'email' | 'uuid';
  required: boolean;
  min?: number;
  max?: number;
  minLength?: number;
  maxLength?: number;
  maxItems?: number;
  param?: boolean;
  enum?: string[];
}

/**
 * UUID validation regex
 */
const uuidRegex = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;

/**
 * Email validation regex
 */
const emailRegex = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

/**
 * Validate a single field value against its rule
 */
function validateField(rule: ValidationRule, value: unknown, fieldName: string): void {
  // Check required
  if (rule.required && (value === undefined || value === null || value === '')) {
    throw new ValidationError(`${fieldName} is required`);
  }

  // Skip further validation if not required and value is empty
  if (!rule.required && (value === undefined || value === null || value === '')) {
    return;
  }

  // Type validation
  switch (rule.type) {
    case 'string':
      if (typeof value !== 'string') {
        throw new ValidationError(`${fieldName} must be a string`);
      }
      if (rule.minLength !== undefined && value.length < rule.minLength) {
        throw new ValidationError(`${fieldName} must be at least ${rule.minLength} characters`);
      }
      if (rule.maxLength !== undefined && value.length > rule.maxLength) {
        throw new ValidationError(`${fieldName} must be at most ${rule.maxLength} characters`);
      }
      break;

    case 'number':
      if (typeof value !== 'number' || isNaN(value)) {
        throw new ValidationError(`${fieldName} must be a number`);
      }
      if (rule.min !== undefined && value < rule.min) {
        throw new ValidationError(`${fieldName} must be at least ${rule.min}`);
      }
      if (rule.max !== undefined && value > rule.max) {
        throw new ValidationError(`${fieldName} must be at most ${rule.max}`);
      }
      break;

    case 'boolean':
      if (typeof value !== 'boolean') {
        throw new ValidationError(`${fieldName} must be a boolean`);
      }
      break;

    case 'array':
      if (!Array.isArray(value)) {
        throw new ValidationError(`${fieldName} must be an array`);
      }
      if (rule.maxItems !== undefined && value.length > rule.maxItems) {
        throw new ValidationError(`${fieldName} can have at most ${rule.maxItems} items`);
      }
      break;

    case 'email':
      if (typeof value !== 'string' || !emailRegex.test(value)) {
        throw new ValidationError(`${fieldName} must be a valid email address`);
      }
      break;

    case 'uuid':
      if (typeof value !== 'string' || !uuidRegex.test(value)) {
        throw new ValidationError(`${fieldName} must be a valid UUID`);
      }
      break;
  }

  // Enum validation
  if (rule.enum && typeof value === 'string' && !rule.enum.includes(value)) {
    throw new ValidationError(`${fieldName} must be one of: ${rule.enum.join(', ')}`);
  }
}

/**
 * Validation middleware factory
 * Returns middleware that validates request against the specified schema
 */
export function validate(schemaName: string) {
  const rules = validationSchemas[schemaName];

  if (!rules) {
    throw new Error(`Validation schema '${schemaName}' not found`);
  }

  return (req: Request, res: Response, next: NextFunction): void => {
    try {
      for (const rule of rules) {
        const value = rule.param
          ? req.params[rule.field]
          : req.body?.[rule.field] ?? req.query?.[rule.field];

        validateField(rule, value, rule.field);
      }

      next();
    } catch (error) {
      next(error);
    }
  };
}
