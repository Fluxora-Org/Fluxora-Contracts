/**
 * Contrast ratio utilities for WCAG 2.1 compliance.
 *
 * Implements the relative luminance and contrast ratio formulas from
 * https://www.w3.org/TR/WCAG21/#dfn-relative-luminance and
 * https://www.w3.org/TR/WCAG21/#dfn-contrast-ratio.
 */

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export type WCAGLevel = "AAA" | "AA" | "AALarge" | "Fail";

export interface ContrastOptions {
  /** WCAG level to check against. Defaults to "AA". */
  level?: WCAGLevel;
  /** Whether the text is large (>= 18pt or >= 14pt bold). Defaults to false. */
  isLargeText?: boolean;
}

export interface ColorPairValidation {
  foreground: string;
  background: string;
  ratio: number;
  level: WCAGLevel;
  passes: boolean;
}

export interface ThemeValidation {
  pairs: ColorPairValidation[];
  allPass: boolean;
  failures: string[];
}

// ---------------------------------------------------------------------------
// WCAG contrast ratio computation
// ---------------------------------------------------------------------------

const HEX3_REGEX = /^#([0-9a-f])([0-9a-f])([0-9a-f])$/i;
const HEX6_REGEX = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i;
const HEX8_REGEX =
  /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i;

/**
 * Parse a hex color string to [r, g, b] values in the range [0, 255].
 *
 * Supports #RGB, #RRGGBB, and #RRGGBBAA formats.  The alpha channel in
 * #RRGGBBAA is ignored for luminance computation (treated as fully opaque).
 *
 * @throws {Error} if the hex string is malformed.
 */
export function parseHex(hex: string): [number, number, number] {
  if (typeof hex !== "string") {
    throw new Error(`Expected hex color string, got ${typeof hex}`);
  }

  const trimmed = hex.trim();

  let match = HEX3_REGEX.exec(trimmed);
  if (match) {
    return [
      parseInt(match[1] + match[1], 16),
      parseInt(match[2] + match[2], 16),
      parseInt(match[3] + match[3], 16),
    ];
  }

  match = HEX6_REGEX.exec(trimmed);
  if (match) {
    return [
      parseInt(match[1], 16),
      parseInt(match[2], 16),
      parseInt(match[3], 16),
    ];
  }

  match = HEX8_REGEX.exec(trimmed);
  if (match) {
    return [
      parseInt(match[1], 16),
      parseInt(match[2], 16),
      parseInt(match[3], 16),
    ];
  }

  throw new Error(`Invalid hex color: ${hex}`);
}

/**
 * Compute the sRGB component value used in the relative luminance formula.
 *
 * Each 8-bit sRGB channel is converted to a linear value in [0, 1]:
 *   - value <= 0.04045  →  value / 12.92
 *   - value >  0.04045  →  ((value + 0.055) / 1.055) ^ 2.4
 */
function linearize(srgb: number): number {
  const v = srgb / 255;
  return v <= 0.04045 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4);
}

/**
 * Compute the relative luminance of an sRGB color.
 *
 * Returns a value in [0, 1] where 0 is black and 1 is white.
 * https://www.w3.org/TR/WCAG21/#dfn-relative-luminance
 */
export function relativeLuminance(hex: string): number {
  const [r, g, b] = parseHex(hex);
  return 0.2126 * linearize(r) + 0.7152 * linearize(g) + 0.0722 * linearize(b);
}

/**
 * Compute the WCAG 2.1 contrast ratio between two hex colors.
 *
 * Returns a value in [1, 21].  A ratio of 1 means the colors are identical;
 * 21 is the maximum (black vs white).
 *
 * https://www.w3.org/TR/WCAG21/#dfn-contrast-ratio
 */
export function contrastRatio(foreground: string, background: string): number {
  const l1 = relativeLuminance(foreground);
  const l2 = relativeLuminance(background);
  const lighter = Math.max(l1, l2);
  const darker = Math.min(l1, l2);
  return (lighter + 0.05) / (darker + 0.05);
}

// ---------------------------------------------------------------------------
// WCAG level classification
// ---------------------------------------------------------------------------

/**
 * WCAG 2.1 contrast ratio thresholds.
 *
 * https://www.w3.org/WAI/WCAG21/Understanding/contrast-minimum.html
 */
export const WCAG_THRESHOLDS = {
  AAA: 7,
  AA: 4.5,
  AALarge: 3,
} as const;

/**
 * Classify a contrast ratio into a WCAG compliance level.
 *
 * - AAA:      ratio >= 7   (enhanced)
 * - AA:       ratio >= 4.5 (minimum for normal text)
 * - AALarge:  ratio >= 3   (minimum for large text, >= 18pt or >= 14pt bold)
 * - Fail:     ratio < 3
 */
export function classifyContrast(ratio: number): WCAGLevel {
  if (ratio >= WCAG_THRESHOLDS.AAA) return "AAA";
  if (ratio >= WCAG_THRESHOLDS.AA) return "AA";
  if (ratio >= WCAG_THRESHOLDS.AALarge) return "AALarge";
  return "Fail";
}

/**
 * Check whether a contrast ratio meets a specific WCAG level.
 *
 * The hierarchy is AAA > AA > AALarge > Fail.  A ratio that meets AAA
 * also meets AA and AALarge.
 */
export function meetsWCAG(
  ratio: number,
  level: WCAGLevel = "AA",
): boolean {
  switch (level) {
    case "AAA":
      return ratio >= WCAG_THRESHOLDS.AAA;
    case "AA":
      return ratio >= WCAG_THRESHOLDS.AA;
    case "AALarge":
      return ratio >= WCAG_THRESHOLDS.AALarge;
    case "Fail":
      return true; // everything "passes" a failing grade
  }
}

// ---------------------------------------------------------------------------
// Pair and theme validation
// ---------------------------------------------------------------------------

/**
 * Validate a single foreground/background color pair.
 *
 * Returns a structured result with the computed ratio, classification, and
 * pass/fail status against the requested WCAG level.
 */
export function validateColorPair(
  foreground: string,
  background: string,
  options: ContrastOptions = {},
): ColorPairValidation {
  const { level = "AA", isLargeText = false } = options;
  const ratio = contrastRatio(foreground, background);
  const effectiveLevel: WCAGLevel = isLargeText ? "AALarge" : level;
  const passes = meetsWCAG(ratio, effectiveLevel);

  return {
    foreground,
    background,
    ratio,
    level: classifyContrast(ratio),
    passes,
  };
}

/**
 * Validate all color pairs in a theme definition.
 *
 * The theme is a record of named color pairs.  Returns a summary with per-pair
 * results, an overall pass/fail flag, and human-readable failure messages.
 */
export function validateTheme(
  theme: Record<string, { foreground: string; background: string }>,
  options: ContrastOptions = {},
): ThemeValidation {
  const pairs: ColorPairValidation[] = [];
  const failures: string[] = [];

  for (const [name, { foreground, background }] of Object.entries(theme)) {
    const result = validateColorPair(foreground, background, options);
    pairs.push(result);
    if (!result.passes) {
      failures.push(
        `${name}: contrast ratio ${result.ratio.toFixed(2)} (${result.level}) does not meet ${options.level ?? "AA"}`,
      );
    }
  }

  return {
    pairs,
    allPass: failures.length === 0,
    failures,
  };
}

// ---------------------------------------------------------------------------
// Theme-aware helpers
// ---------------------------------------------------------------------------

/**
 * Common Fluxora brand palette.  Used as the reference set for regression
 * tests that guard against accidental contrast regressions.
 */
export const FLUXORA_PALETTE = {
  /** Primary brand blue on white background. */
  primaryOnWhite: { foreground: "#1a56db", background: "#ffffff" },
  /** Primary brand blue on dark background. */
  primaryOnDark: { foreground: "#60a5fa", background: "#111827" },
  /** Status success green on white. */
  successOnWhite: { foreground: "#047857", background: "#ffffff" },
  /** Status error red on white. */
  errorOnWhite: { foreground: "#dc2626", background: "#ffffff" },
  /** Muted text on white. */
  mutedOnWhite: { foreground: "#6b7280", background: "#ffffff" },
  /** Body text on white. */
  bodyOnWhite: { foreground: "#1f2937", background: "#ffffff" },
  /** Body text on dark. */
  bodyOnDark: { foreground: "#e5e7eb", background: "#111827" },
  /** Link blue on white. */
  linkOnWhite: { foreground: "#2563eb", background: "#ffffff" },
} as const;

/**
 * Validate the Fluxora brand palette against WCAG AA.
 *
 * This is the primary regression guard: if a color change accidentally
 * degrades a pair below AA, this function surfaces it.
 */
export function validateFluxoraPalette(
  options: ContrastOptions = {},
): ThemeValidation {
  return validateTheme(
    FLUXORA_PALETTE as Record<
      string,
      { foreground: string; background: string }
    >,
    options,
  );
}
