# Codex Workflow (LivePhish)

This repo is configured for Codex-first execution with Claude-like quality guardrails.

## Daily commands

- `cx` - start Codex with live web search enabled
- `cxf` - start Codex with `fastest` profile
- `cxs` - start Codex with `safe` profile
- `cxi` - start Codex inline (no alternate screen)
- `cxr` - run Codex code review mode
- `cxe` - run Codex non-interactively
- `cxl` - resume last Codex session

After shell updates, reload once:

```bash
source ~/.zshrc
```

## UI quality tips

- Use `/mcp` to confirm active MCP tools in-session.
- Use `/skills` (or `$`) to explicitly invoke a skill.
- Use `/model` to switch model quickly.
- Use `codex resume --last` and `codex fork --last` to keep continuity.
- For a richer interface similar to Claude desktop workflows, use `codex app`.

## Installed MCP integrations

- `context7` (`npx -y @upstash/context7-mcp`)
- `chrome-devtools` (`npx chrome-devtools-mcp@latest`)
- `prompts-chat` (`https://prompts.chat/api/mcp`)

Check status:

```bash
codex mcp list
```

## Installed curated skills

- `gh-address-comments`
- `gh-fix-ci`
- `playwright`
- `screenshot`
- `security-best-practices`

## Repo-local custom skills

Located in `.agents/skills`:

- `fresh-plan-review`
- `learn`
- `verifier`

## Optional MCP add-ons (full-stack)

- Sentry: issue/log triage workflows
- GitHub MCP: PR/issues workflow beyond local git
- Atlassian MCP: Jira/Confluence workflows
- Stripe MCP: billing/event debugging
- Shopify MCP: store/admin workflows

Start with only tools you actively use to reduce noise.

Install pattern:

```bash
codex mcp add <name> --url <https-mcp-endpoint>
# or
codex mcp add <name> -- <local-or-npx-command>
```
