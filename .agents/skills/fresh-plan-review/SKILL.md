---
name: fresh-plan-review
description: Critique an implementation plan with a context-light, risk-focused review before coding.
---

# Fresh Plan Review

Use this skill when a plan has multiple phases, non-trivial risk, or external dependencies.

## Workflow

1. Read the plan file only.
2. Review critically for:
   - Missing prerequisites and hidden dependencies
   - Risky assumptions
   - Edge cases and failure modes
   - Testing and rollback gaps
   - Over-engineering or missing simplifications
3. Return a structured critique:
   - Findings (ordered by severity)
   - Open questions
   - Required plan edits

## Output standard

- Be direct.
- Include concrete, actionable fixes.
- Prefer file/section references when possible.

