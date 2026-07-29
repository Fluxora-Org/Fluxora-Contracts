Description
contracts/stream/tests/stream_templates.rs (with docs/stream-templates.md) tests pre-configured stream templates. Add a test confirming every template documented in docs/stream-templates.md actually exists and produces the documented parameters when instantiated, so the doc can't silently drift from the code (e.g. a template renamed or its default duration changed without updating the doc).

Requirements
Parse or hand-enumerate the templates documented in docs/stream-templates.md.
For each, instantiate it via the contract's template mechanism and assert the resulting stream parameters match the documented values exactly.
Fail the test if a documented template doesn't exist, or note (don't fail on) an undocumented template that exists in code.
Suggested execution
Review docs/stream-templates.md and the template definitions in code.
Add the per-template cross-check test.
Resolve any found mismatch (update doc or flag code issue) per the finding.
Acceptance criteria

Every documented template is verified against actual instantiated parameters.

Test fails on doc/code drift for documented templates.

Undocumented templates (if any) are surfaced for maintainer review.
Security notes
None directly; incorrect documented defaults could mislead an integrator into misconfiguring a stream's terms.

Guidelines
Minimum 95% test coverage