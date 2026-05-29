/**
 * Unit tests for src/utils/response.ts
 */

import { toDecimalString } from '../../src/utils/response';

describe('toDecimalString', () => {
  it('converts a bigint to decimal string', () => {
    expect(toDecimalString(123456789012345678901234567890n)).toBe(
      '123456789012345678901234567890'
    );
  });

  it('converts a number to decimal string', () => {
    expect(toDecimalString(100)).toBe('100');
    expect(toDecimalString(0)).toBe('0');
    expect(toDecimalString(3.14)).toBe('3.14');
  });

  it('passes through a valid decimal string unchanged', () => {
    expect(toDecimalString('99999999999999999999')).toBe('99999999999999999999');
    expect(toDecimalString('1.5')).toBe('1.5');
  });

  it('throws for a non-finite number', () => {
    expect(() => toDecimalString(Infinity)).toThrow(TypeError);
    expect(() => toDecimalString(NaN)).toThrow(TypeError);
  });

  it('throws for an invalid decimal string', () => {
    expect(() => toDecimalString('abc')).toThrow(TypeError);
    expect(() => toDecimalString('1e10')).toThrow(TypeError);
    expect(() => toDecimalString('')).toThrow(TypeError);
  });
});
