---
name: verifier
description: Run focused post-change verification and report PASS/FAIL with concrete evidence.
---

# Verifier

Use this skill after implementation and before handoff.

## Workflow

1. Inspect changed files with `git diff --stat` and `git diff --name-only`.
2. Run relevant checks:
   - Python: `uv run pytest -v` (or targeted files first, then full suite when feasible)
   - Formatting/lint/type checks if configured in the repo
3. Confirm no accidental sensitive file edits.
4. Report:
   - `PASS` or `FAIL`
   - Commands run
   - Failing command output summary
   - Residual risk if any checks were skipped

## Hard rules

- Do not edit files.
- Do not commit.
- Do not run blocking interactive commands.

