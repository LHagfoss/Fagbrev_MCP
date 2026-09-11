# Fagbrev MCP

Fagbrev MCP lets an AI assistant read and manage your own Fagbrev.io apprenticeship dashboard.

It is a local Rust program. It uses a separate Chrome profile for your Fagbrev login and talks to an MCP client over `stdio` (standard input/output). It does not run a public web server.

## Quick start

```bash
cargo run -- login    # opens Chrome; log in normally
cargo run -- status   # print a small dashboard summary
cargo run -- mcp      # start the MCP server for your MCP client
```

The login profile is stored in your operating system's application-data folder, outside this repository. Your password, cookies, and tokens are never printed or committed.

## How it works

1. `login` opens a dedicated Chrome profile.
2. You log in to Fagbrev.io yourself.
3. The MCP server reuses that saved browser session.
4. Read tools open the dashboard and return structured JSON.
5. Write tools show a preview first and need `confirm: true` before changing anything.

The AI client keeps the conversation context. The server keeps the login session, but does not currently keep conversation memory. Tool calls pass IDs, targets, and draft content explicitly.

## Available tools

### Dashboard and learning plan

- `get_dashboard_overview` and `get_status`
- `list_competency_goals` and `get_competency_goal`
- `list_delmal` and `get_delmal`
- `list_half_year_tasks` and `get_half_year_task`

### Documentation

- `list_documentation` and `get_documentation`
- `list_feedback` and `get_feedback`
- `find_reusable_content`
- `make_documentation_template`
- `submit_documentation`
- `request_approval`

The competency goals and delmål are read-only learning-plan data. Documentation is the user-owned data that can be created or changed.

## Safe writing

`submit_documentation` and `request_approval` are preview-only by default. They only use visible Fagbrev.io UI controls when called with `confirm: true`. The new-documentation form may create a blank draft when opened, so that warning is included in the preview.

## Development

```bash
cargo fmt --all -- --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo run -- inspect
```

See [`AGENTS.md`](AGENTS.md) for the technical architecture, data flow, tool contracts, and implementation notes.

## Security and privacy

This project handles apprenticeship records and potentially personal or employer-confidential information.

- Never put passwords, one-time codes, cookies, Firebase tokens, or personal dashboard exports in Git.
- Keep the MCP server local by default.
- Use least-privilege access and read-only operations wherever possible.
- Require confirmation for every external write.
- Avoid logging full documentation text unless the user explicitly enables it.
- Respect Fagbrev.io's terms, robots/access policies, and rate limits.
- If this becomes a shared or hosted service, add proper user consent, isolation, encryption, retention rules, and an authorization review before using it with anyone else's account.
