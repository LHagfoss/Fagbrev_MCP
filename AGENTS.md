# Fagbrev MCP technical notes

This file explains how the project works under the hood. The public README is intentionally shorter and more beginner-friendly.

## Runtime architecture

The binary has two layers:

- CLI layer: `login`, `status`, `inspect`, `mcp`, and `logout`.
- Adapter/MCP layer: `src/browser.rs` drives the authenticated Chrome profile and `src/mcp.rs` exposes typed `rmcp` tools.

`cargo run -- mcp` starts an MCP server over `rmcp::transport::stdio`. MCP JSON-RPC uses stdin/stdout; logs must stay on stderr. There is no HTTP MCP endpoint.

The browser adapter uses `chromiumoxide` and a persistent, dedicated Chrome profile. `login` is headful so the user can authenticate. Normal reads run headless against the same profile. `FAGBREV_CDP_URL` and `FAGBREV_BROWSER_EXECUTABLE` are local development overrides.

## Data flow

```text
MCP client / AI conversation
        │ tool arguments
        ▼
rmcp tool router (src/mcp.rs)
        │ typed request
        ▼
Fagbrev browser adapter (src/browser.rs)
        │ visible UI + authenticated profile
        ▼
Fagbrev.io
        │ normalized result
        ▼
structured JSON returned to the MCP client
```

The server is currently stateless between tool calls except for the saved browser authentication profile. The AI client owns conversational context. Cross-tool context is passed using stable document IDs, explicit `DocumentationTarget` values, and source-documentation ID lists.

## Tool groups

### Read tools

- Dashboard: `get_dashboard_overview`, `get_status`
- Learning plan: `list_competency_goals`, `get_competency_goal`, `list_delmal`, `get_delmal`
- Documentation: `list_documentation`, `get_documentation`
- Local composition: `find_reusable_content`, `make_documentation_template`
- Planned/extended reads: half-year tasks and feedback tools are added in feature branches as their PRs land.

Read tools should return normalized models from `src/data.rs`, not raw browser HTML. Raw page text is for diagnostics and should eventually be limited to `inspect`.

### Write tools

`submit_documentation` creates a documentation entry through the visible form. `request_approval` activates the exact visible approval control when available.

Both require an explicit `confirm: true`. With confirmation omitted or false, the MCP tool must validate the request and return a preview without opening a mutating page. Never use direct Firebase writes or undocumented backend calls.

The Fagbrev new-documentation form has an important side effect: opening it can create a blank draft. Treat opening that form as a mutation and keep it behind the same confirmation boundary.

## Context model

There are three different kinds of context:

1. Authentication context: the saved Chrome profile. It is local sensitive state and never belongs in tool arguments or Git.
2. Conversation context: the MCP host/AI client remembers previous tool results.
3. Domain context: tool arguments and results carry `DocumentationTarget`, document IDs, delmål ordinals, source IDs, warnings, and statuses.

The current server does not persist local drafts or conversation history. A future draft store should use a local `draft_id`, version number, target, source IDs, and content, with no credentials. Context bundles should be bounded so a whole documentation archive is not injected into every model turn.

## Identity and safety rules

- Documentation IDs come from `/l/dokumentasjon/{id}` links.
- Delmål currently use a parent goal number plus UI-local ordinal/title because the rendered UI does not expose an independent ID. Never invent a Firebase ID.
- Competency goals and delmål are plan/catalog data and should remain read-only.
- User documentation is mutable, but updates/deletes must be confirmation-gated and should use an expected version/timestamp to prevent overwriting newer edits.
- Approved documentation should not be deleted or rewritten unless the UI explicitly permits that state transition.
- Browser selectors should use stable labels/routes and tolerate loading, empty results, expired sessions, and UI changes.

## Testing

Use fixture tests for parsers and pure local logic. Do not open the new-documentation form in automated or exploratory read-only checks because it may create a draft. Live checks should use existing pages and read-only tools only.

```bash
cargo fmt --all -- --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo build
```

MCP Inspector can verify discovery and calls:

```bash
cargo build --bin fagbrev-mcp
npx @modelcontextprotocol/inspector --cli ./target/debug/fagbrev-mcp mcp --method tools/list
```
