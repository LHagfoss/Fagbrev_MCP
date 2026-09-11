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

The server has no hidden conversation memory. It keeps the saved browser authentication profile and explicitly saved local drafts. Cross-tool context is passed using stable document IDs, explicit `DocumentationTarget` values, local draft IDs, and source-documentation ID lists.

## Tool groups

### Read tools

- Dashboard: `get_dashboard_overview`, `get_status`
- Learning plan: `list_competency_goals`, `get_competency_goal`, `list_delmal`, `get_delmal`
- Documentation: `list_documentation`, `get_documentation`
- Local composition: `find_reusable_content`, `make_documentation_template`
- Local drafts: `save_draft`, `list_drafts`, `get_draft`, `update_draft`, `delete_draft`
- Bounded assembly: `get_context_bundle`
- Half-year tasks and feedback are read-only tools exposed by the current main branch.

Read tools should return normalized models from `src/data.rs`, not raw browser HTML. Raw page text is for diagnostics and should eventually be limited to `inspect`.

`list_competency_goals` validates that all 21 numbered goals were parsed. The page has rendered its status count both before and after a numbered heading, so the parser accepts either adjacent order and reports a clear error instead of returning an empty success when the UI is still loading or changes.

Documentation table reads use the smallest page-size option exposed by Fagbrev (`6`, `12`, `18`, `24`, `100`, or `200`) that can satisfy a requested bounded context limit. `get_context_bundle` trims that page to its requested maximum; an explicit `list_documentation` page size remains available for callers that intentionally need a larger page.

### Write tools

`submit_documentation` creates a documentation entry through the visible form. `update_documentation` edits an existing record through its visible editor and can replace its visible goal/delmål selection. `request_approval` activates the exact visible approval control when available. `delete_documentation` is draft-only and returns a structured `unsupported` result when the detail page does not expose an exact delete control.

Every mutating workflow requires an explicit `confirm: true`. With confirmation omitted or false, the MCP tool must validate the request and return a preview without opening an edit, detail, or new-entry page. Updates and deletes require the exact `updated_at` value returned by a prior read; deletes additionally require `expected_status: draft`. Never use direct Firebase writes or undocumented backend calls.

The Fagbrev new-documentation form has an important side effect: opening it can create a blank draft. Treat opening that form as a mutation and keep it behind the same confirmation boundary.

Local draft operations are different: they only read or atomically replace JSON files below the OS application-data directory's `drafts/` folder. A `LocalDraft` contains a generated local `draft_id`, title/content, target, source documentation IDs, version, timestamps, and `status: local_only`. It never contains browser profile data, credentials, cookies, or Firebase tokens. `save_draft` creates a draft when `draft_id` is omitted and updates one when its exact `expected_version` is supplied. `update_draft` always requires that version. `list_drafts` returns bounded summaries without content; `delete_draft` removes only the validated local file.

The draft fields are deliberately compatible with `submit_documentation`, but no draft tool submits automatically. A caller must explicitly read/review the draft and pass its data to the separately confirmation-gated Fagbrev write tool.

## Context model

There are three different kinds of context:

1. Authentication context: the saved Chrome profile. It is local sensitive state and never belongs in tool arguments or Git.
2. Conversation context: the MCP host/AI client remembers previous tool results.
3. Domain context: tool arguments and results carry `DocumentationTarget`, document IDs, delmål ordinals, source IDs, warnings, and statuses.

`get_context_bundle` is an explicit snapshot, not memory. It always attempts current dashboard/status reads, optionally includes one selected competency goal and its delmål, and includes at most five documentation summaries by default (maximum twenty). Passing explicit `document_ids` allows selected details; `include_document_content` must also be true before selected document content is returned. The response includes `retrieved_at`, source URLs, applied limits, and warnings. It never injects the whole archive by default.

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
