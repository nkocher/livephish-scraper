# AGENTS.md

Repository instructions for Codex.

## Working defaults

- Use profile `fastest` unless the task is high-risk; then switch to `safe`.
- Read `CLAUDE.md` before making changes.
- Prefer `rg`/`rg --files` for search.

## Quality bar

- Explore first, then edit.
- Keep edits scoped to the request.
- Verify before claiming completion.
- For reviews: report findings first, ordered by severity, with file/line references.

## Verification

- Python changes: run `uv run pytest -v` (or targeted test files if a full run is too slow).
- If tests are skipped, explicitly state why and what was not validated.

## Guardrails

- Never run destructive git commands (`git reset --hard`, `git checkout --`, etc.) unless explicitly requested.
- Do not run interactive commands that will hang unattended sessions.
- For this repo, avoid running `uv run livephish`/`uv run livephish browse` during automated work because it launches an interactive TUI.

## Research

- For version-sensitive library/API behavior, use live docs before implementation.
- Prefer primary sources (official docs/changelogs/specs).

## Planning

- For non-trivial plans, run `$fresh-plan-review <path-to-plan.md>` before implementation.

