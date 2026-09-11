# Fagbrev MCP

An MCP server for helping a lærling read and manage their own Fagbrev.io training dashboard through a conversational interface.

The first goal is simple: ask questions such as:

- “What competency goals do I still need to work on?”
- “Show me the sub-goals for testing and debugging.”
- “Which documentations are linked to competency goal 10?”
- “Help me draft documentation for this work.”

The assistant should be able to read the dashboard and prepare actions. Any action that changes Fagbrev.io data must require an explicit confirmation first.

## What we observed

The authenticated dashboard currently contains:

- An overview with apprenticeship progress, documentation counts, corrections, and assessment conversations.
- An **Opplæringsplan** for *Læreplan i IT-utviklerfaget*.
- 21 expandable competency goals (**kompetansemål**).
- Suggested work activities/sub-goals inside each competency goal.
- Linked documentation, approval status, approval dates, and a **Ny dokumentasjon** action.
- A second tab, **Halvårsoppgaver og minifagprøve**, with six expandable areas:
  - Etikk, lovverk og yrkesutøvelse
  - Kodeferdigheter og metode
  - Sikkerhet og personvern
  - Infrastruktur og arkitektur
  - Design, interaksjon og brukerdialog
  - Minifagprøve
- A download action for the training plan.

The dashboard is authenticated. Firebase appears to be part of the frontend stack, but that alone does not mean that direct Firebase access is a supported or safe integration. Firebase configuration values shipped to a browser are not server credentials, and the backend security rules still control access.

## Rust-first architecture

The first implementation will be a single Rust binary with subcommands:

```text
fagbrev login     Launch a dedicated browser and wait for the user to log in
fagbrev status    Read a small dashboard summary
fagbrev inspect   Print normalized dashboard data for development
fagbrev mcp       Run the MCP server over stdio
fagbrev logout    Remove the local browser session after confirmation
```

Use the official Rust MCP SDK, `rmcp`, for the stdio server. Its server transport reads and writes MCP JSON-RPC on stdin/stdout, so application logs must go to stderr. Keep the CLI commands separate from the MCP transport so browser/session problems can be tested without an MCP client.

This project does not expose an HTTP MCP endpoint. The only network traffic is the local browser automation talking to Fagbrev.io over HTTPS.

### Login flow

`fagbrev login` should:

1. Launch Chromium/Chrome with a dedicated Fagbrev MCP profile.
2. Open `https://fagbrev.io/l` in that profile.
3. Wait while the user completes the normal Fagbrev login in the browser.
4. Detect the authenticated dashboard by URL and stable visible text, then print a success message.
5. Keep the profile for later commands; do not extract or print cookies, passwords, or Firebase tokens.

Chromium may use an internal random CDP port for browser control. That is separate from MCP and is not exposed as a public service. The observed login page asks for a phone number or email, so the initial browser approach verifies the resulting authenticated page instead of assuming an OAuth-style callback.

Use a dedicated persistent browser profile rather than the user’s everyday Chrome profile. The profile contains sensitive session data and must be stored in an OS-appropriate private application-data directory, excluded from Git, and protected with a lock so two commands cannot use it at the same time.

The browser adapter should be replaceable. Start with CDP-capable Rust browser automation, such as `chromiumoxide`, and keep all Fagbrev-specific selectors and parsing in one module. Zen Browser/Firefox is not the browser controlled by this first adapter; use a dedicated Chromium/Chrome profile for automation so the user’s everyday browser is not taken over. If Fagbrev provides a supported API later, add an API adapter without changing the MCP tool interface.

For development, an existing Chromium-compatible browser can be attached with `FAGBREV_CDP_URL`. `FAGBREV_BROWSER_EXECUTABLE` can point to a non-standard Chromium executable when automatic detection does not find it. These are local development escape hatches, not credentials.

## Recommended approach

Build this in two layers:

1. **MCP interface** — exposes a small set of safe, typed tools to the AI client.
2. **Fagbrev adapter** — talks to the service using an official API if one is available; otherwise operates the authenticated browser profile with the user’s permission.

Prefer an official API or written integration permission from Fagbrev.io. Use browser automation only for the user’s own account and only where an API is unavailable. Do not bypass login, security rules, CAPTCHA, rate limits, or access controls, and do not collect data belonging to other users.

## Initial MCP tools

Already implemented as a first read-only slice:

- `get_dashboard_overview`
- `list_competency_goals`
- `get_competency_goal` — currently returns the expanded page text, including work activities

Next read-only tools:

- `get_status` — a focused status response for progress and documentation states
- `list_delmal` — list the suggested sub-goals/work activities for a competency goal
- `get_delmal` — return one sub-goal with its parent competency goal
- `list_documentation` — list submitted, approved, in-review, and correction-needed documentation
- `list_half_year_tasks`
- `get_half_year_task`

The UI currently shows a status count on each competency goal and documentation state on submitted entries. A “delmål” appears to be suggested work activity text inside a competency goal, so the adapter must verify whether it has a real identifier/status of its own before treating it as an independently deliverable record.

### Documentation workflow

Avoid separate tools that duplicate the same submission mechanism. First determine whether Fagbrev attaches a new documentation entry to a competency goal, a delmål, or both. Then use one typed workflow:

- `draft_documentation` — local draft only, with a target goal/delmål
- `find_reusable_content` — compare existing user-written documentation against a target goal/delmål
- `make_documentation_template` — create an editable draft from approved/relevant source content, preserving source references
- `submit_documentation` — submit a prepared draft after explicit confirmation
- `request_approval` — ask for approval after explicit confirmation

The model should never silently submit documentation or request approval. Before a write, show the exact target, content, source documents reused, and side effect, then require confirmation in the MCP client. Reusing text should help structure the learner’s own evidence; it must not invent work completed or blindly duplicate claims.

## Suggested build steps

1. **Choose the integration boundary**
   - Ask Fagbrev.io whether a documented API or integration access is available.
   - If not, prototype with browser automation against the already-authenticated user session.

2. **Create a small local Rust MCP server**
   - Use `rmcp`, Tokio, Serde, and JSON Schema derivation for typed tool inputs/outputs.
   - Run it over stdio for a local MCP client.
   - Keep credentials, cookies, and session profiles outside Git.

3. **Map the read-only dashboard**
   - Identify stable UI labels and routes rather than relying only on generated CSS selectors.
   - Normalize dashboard data into stable objects: goals, sub-goals, documentation, status, and dates.
   - Handle loading states, expired sessions, empty lists, and permission errors.

4. **Add a local draft workflow**
   - Let the assistant turn the user’s description into a proposed documentation entry.
   - Store drafts locally and let the user edit them before submission.

5. **Add confirmed writes**
   - Implement one write at a time.
   - Display a final preview before creating documentation.
   - Re-read the result after submission and return the created item/status.
   - Make approval requests a separate, explicit confirmation.

6. **Test safely**
   - Begin with read-only tools and fixture data.
   - Use a test account or sandbox if Fagbrev.io provides one.
   - Add audit logging without storing passwords, raw cookies, or unnecessary personal data.
   - Stop and ask the user when the session expires or the UI changes unexpectedly.

## Local development loop

Build the CLI first, before exposing any MCP tools:

```bash
cargo run -- login
cargo run -- status
cargo run -- inspect
```

The first useful result is normalized JSON for the dashboard overview, the 21 competency goals, their expanded sub-goals, and the six half-year task areas. Save sanitized fixtures from that shape and write parser tests against the fixtures.

The dashboard overview should expose structured fields such as `progress_percent`, documentation counts, and assessment-conversation count. Raw page text belongs only to `inspect`, not to the MCP protocol response.

Then run the stdio server directly:

```bash
cargo run -- mcp
```

Use the MCP Inspector to launch the server and verify discovery/calls without configuring a host application:

```bash
cargo build --bin fagbrev-mcp
npx @modelcontextprotocol/inspector ./target/debug/fagbrev-mcp mcp
npx @modelcontextprotocol/inspector --cli ./target/debug/fagbrev-mcp mcp --method tools/list
```

At this stage, expose only `get_dashboard_overview`, `list_competency_goals`, and `get_competency_goal`. Test the browser adapter separately from MCP, and test MCP tools with a fake adapter so an automated test can never submit real documentation.

## Security and privacy

This project handles apprenticeship records and potentially personal or employer-confidential information.

- Never put passwords, one-time codes, cookies, Firebase tokens, or personal dashboard exports in Git.
- Keep the MCP server local by default.
- Use least-privilege access and read-only operations wherever possible.
- Require confirmation for every external write.
- Avoid logging full documentation text unless the user explicitly enables it.
- Respect Fagbrev.io’s terms, robots/access policies, and rate limits.
- If this becomes a shared or hosted service, add proper user consent, isolation, encryption, retention rules, and an authorization review before using it with anyone else’s account.

## Project status

This repository currently contains the initial plan only. The next practical milestone is a read-only prototype that can return the dashboard overview and list the 21 competency goals from the user’s authenticated session.
