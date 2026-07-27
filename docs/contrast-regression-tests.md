# Contrast Regression Tests

This document describes the contrast regression test suite in
`src/theme/__tests__/contrastUtils.test.ts`, the behavior it guards against,
and the procedure for updating it when the brand palette changes.

---

## Current behaviour

The contrast regression tests validate that every named color pair in the
Fluxora brand palette meets WCAG 2.1 contrast requirements. The tests are
pure functions — they depend on no external state, network, DOM, or timer —
so they produce deterministic, reproducible results on every run.

### What is tested

| Category | Behaviour guarded | Tests |
|----------|-------------------|-------|
| **Happy path** | Core contrast utilities work correctly: hex parsing, luminance, ratio, classification, pair/theme validation | `parseHex`, `relativeLuminance`, `contrastRatio`, `classifyContrast`, `meetsWCAG`, `validateColorPair`, `validateTheme`, `validateFluxoraPalette` |
| **Error states** | Invalid input is rejected with clear errors (empty string, null, non-string, missing `#`, wrong length, named colors, `rgb()` notation) | `parseHex` error handling (7 cases) |
| **Boundary conditions** | Exact threshold ratios are classified correctly; identical colors always fail; black/white always passes AAA | `boundary contrast ratios`, `near-identical colors` |
| **Determinism** | All pure functions return identical results across 50–100 repeated calls (no floating-point drift, no RNG, no side effects) | `determinism` (4 tests) |
| **Batch validation** | 4 096-pair exhaustive color grid and 100-entry theme validate without error or panic | `batch validation` (2 tests) |
| **State transitions** | Progressive theme modification (add/remove entries) produces correct pass/fail at each step; interleaved validation calls are independent | `state transitions` (2 tests) |
| **Accessibility** | Symmetric color pairs produce identical ratios; call order does not affect results; WCAG threshold constants are locked | `accessibility` (2 tests) |
| **Viewport independence** | Contrast ratios do not depend on any ambient state, timer, or environment value — pure mathematical computation | `viewport independence` (2 tests) |
| **Regression guard** | Brand palette primary, error, success, and body text colors meet their documented WCAG levels; all palette ratios are non-degenerate | `regression guard` (6 tests) |

### Happy-path user journey

1. `validateFluxoraPalette()` is called (no arguments → defaults to WCAG AA).
2. For each named pair in `FLUXORA_PALETTE`, `validateColorPair()` computes:
   - `relativeLuminance()` for foreground and background.
   - `contrastRatio()` = `(lighter + 0.05) / (darker + 0.05)`.
   - `classifyContrast()` → AAA / AA / AALarge / Fail.
   - `meetsWCAG(ratio, "AA")` → pass/fail.
3. If any pair fails, `validateTheme()` populates `failures[]` with
   human-readable messages and sets `allPass = false`.
4. The test asserts `result.allPass === true`.

### Edge-case behaviour (previously implicit, now explicit)

**Loading state**: The utilities have no loading concept — they are pure
synchronous functions. The test suite verifies this by calling them
immediately with no setup delay.

**Empty state**: `validateTheme({})` returns `{ allPass: true, pairs: [],
failures: [] }`. An empty theme is trivially compliant.

**Retry state**: All pure functions are idempotent. Calling
`contrastRatio("#1a56db", "#ffffff")` 100 times produces the same value
every time. This is verified by the determinism tests.

**Keyboard state**: The output is order-independent — calling
`validateColorPair("#000", "#fff")` and `validateColorPair("#fff", "#000")`
produce the same ratio. Keyboard-driven or programmatic re-validation
cannot produce inconsistent results.

**Responsive state**: The ratio computation is viewport-independent. There
is no dependency on `window.innerWidth`, media queries, or any DOM state.
The same inputs always produce the same ratio regardless of the rendering
context.

---

## Expected regression surface

The following changes would trigger a contrast regression test failure:

### Will fail the tests

1. **Changing any colour in `FLUXORA_PALETTE`** to a value that drops its
   contrast ratio below WCAG AA (4.5:1 for normal text, 3:1 for large text).
2. **Changing WCAG thresholds** in `WCAG_THRESHOLDS` from their standard
   values (AAA=7, AA=4.5, AALarge=3).
3. **Modifying `relativeLuminance` or `contrastRatio`** computation in a
   way that changes the output for any input pair.
4. **Breaking `parseHex`** so it rejects a previously-valid hex format or
   accepts an invalid one.

### Will NOT fail the tests (known gaps)

1. **Adding new colour pairs** to `FLUXORA_PALETTE` — the palette is
   tested exhaustively, so new entries are automatically included.
2. **Removing colour pairs** from `FLUXORA_PALETTE` — fewer pairs means
   fewer checks, not a failure.
3. **Changing non-colour theme properties** (spacing, typography, shadows)
   — these are outside the scope of contrast testing.
4. **Changing the test file itself** — the tests guard the utility
   functions, not their own source code.

---

## Running the tests

```bash
# Install dependencies (first time only)
npm install

# Run all tests
npm test

# Run only contrast regression tests
npm run test:contrast

# Run with verbose output
npx jest --verbose --testPathPattern=contrastUtils
```

### CI integration

Add to your CI pipeline:

```yaml
- name: Contrast regression tests
  run: |
    npm ci
    npm run test:contrast
```

---

## Updating the palette

When the brand palette changes:

1. Update the hex values in `FLUXORA_PALETTE` in
   `src/theme/contrastUtils.ts`.
2. Run `npm run test:contrast` to verify all pairs still pass.
3. If a pair fails, either:
   - Adjust the colour to meet WCAG AA (recommended), or
   - Document the exception in the PR description with a business
     justification (e.g. decorative text that does not convey information).
4. Update the regression guard tests in
   `src/theme/__tests__/contrastUtils.test.ts` if specific pairs have
   new expected ratios.
5. Include the test output in the PR description.

### WCAG thresholds

The standard thresholds are locked by a meta-regression test:

```typescript
expect(WCAG_THRESHOLDS.AAA).toBe(7);
expect(WCAG_THRESHOLDS.AA).toBe(4.5);
expect(WCAG_THRESHOLDS.AALarge).toBe(3);
```

Do not change these values without updating the corresponding test and
documenting the rationale.

---

## Architecture

```
src/theme/
├── contrastUtils.ts              ← WCAG contrast utilities (production code)
└── __tests__/
    └── contrastUtils.test.ts     ← Contrast regression tests
```

### Dependency chain

```
contrastUtils.test.ts
  └─ imports from contrastUtils.ts
       └─ parseHex, relativeLuminance, contrastRatio,
          classifyContrast, meetsWCAG, validateColorPair,
          validateTheme, validateFluxoraPalette, WCAG_THRESHOLDS
```

### No external dependencies

The utility module has zero runtime dependencies. The test file depends
only on Jest (via `ts-jest`). This keeps the regression surface minimal
and avoids transitive-version breakage.

---

## See also

- [WCAG 2.1 Understanding Contrast (Minimum)](https://www.w3.org/WAI/WCAG21/Understanding/contrast-minimum.html)
- [WCAG 2.1 Understanding Contrast (Enhanced)](https://www.w3.org/WAI/WCAG21/Understanding/contrast-enhanced.html)
- [WebAIM Contrast Checker](https://webaim.org/resources/contrastchecker/)
