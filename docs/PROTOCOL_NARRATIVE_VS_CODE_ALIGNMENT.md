DEPRECATED: Protocol Narrative vs Code Alignment
⚠️ This file has been deprecated and removed.

The file PROTOCOL_NARRATIVE_VS_CODE_ALIGNMENT.md (all-caps) previously located at this path has been removed because it was:

Stale: The line numbers cited were 1,400+ lines off from actual code locations
Duplicate: A near-identical file with better content existed (protocol-narrative-code-alignment.md, kebab-case)
Unmaintained: Last updated 2026-03-27, while the kebab-case version was updated 2026-04-23
What to Use Instead
Use protocol-narrative-code-alignment.md (kebab-case) as the authoritative alignment verification document.

This file contains:

✅ Accurate, verified line numbers for all entrypoints
✅ Complete authorization boundary mappings
✅ Verified state transition semantics
✅ Current event emission locations
✅ Up-to-date error code references
Why This Was Removed
Having two files with almost-identical names covering the same subject was a documentation hazard:

Contributors searching for "protocol narrative alignment" had a coin-flip chance of opening the stale one
The stale file's init function citation was at line 665, but the actual function is at line 2061
This 1,400+ line discrepancy undermined the very claim the document exists to make
Verification
All references in the codebase have been verified to point to protocol-narrative-code-alignment.md.

File History
This file was removed as part of documentation hygiene maintenance to ensure contributors always find the current, accurate alignment verification.

Last updated: 2026-07-29
Replacement: protocol-narrative-code-alignment.md