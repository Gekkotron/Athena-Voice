# Web configuration interface — design

Date: 2026-07-24
Status: approved by user (brainstorming session)

## Problem

Configuring Athena-Voice means hand-editing TOML, which is error-prone
(JSON-in-a-string sensors, allowlist host format) and has already caused a
real leak: the Jeedom API key was committed and pushed to the public repo.
The project's goal is "anyone can run this", so configuration must move to a
web interface and secrets must become impossible to commit by accident.

## Scope

- Web UI to **configure and install skills**: enable/disable, edit settings,
  upload `.wasm`, pick bundled prebuilt skills.
- Web-edited config (including secrets) stored in **SQLite**, never in
  tracked files. TOML keeps bootstrap infra only (server, MQTT, providers,
  locales) and stays secret-free.
- **Live apply** for skill changes; infra changes still require a restart and
  the UI says so.
- **Token auth**, localhost bind by default.
- Forms driven by a **skill-declared config schema**; free-form key/value
  fallback for skills without one.

Out of scope: editing infra TOML from the browser, multi-user accounts,
remote (non-LAN) access, a skill marketplace.

## Part 0 — Secrets remediation (first, independent of the UI)

1. **User action**: rotate the Jeedom API key in Jeedom. The old key is in
   pushed history and must be treated as burned.
2. Strip the key and inline Jeedom config from `athena.voice.toml` (and any
   other tracked TOML); leave a documented placeholder pointing at the web
   UI.
3. `.gitignore`: `*.secrets.toml`, SQLite database files.
4. No git history rewrite — rotation makes the leaked value worthless.
5. Optional CI guard: grep tracked `*.toml` for high-entropy strings.

## Architecture

New crate `crates/athena-voice-admin`:

- An axum `Router` spawned by `serve` alongside the MQTT runtime, binding
  the previously unused `[server] host`/`port`.
- Serves the JSON API and a static frontend embedded via `include_dir`
  (plain HTML/CSS/JS — no npm, no bundler; the workspace stays pure Rust and
  the binary stays self-contained).
- Talks to the system through two seams:
  - **athena-voice-storage**: new tables (see Data model).
  - **runtime handle**: "reload skill X now" — reuses the existing hot-reload
    path in the WASM registry.
- Skill uploads are written into the configured skills dir; the existing
  filesystem watcher loads them. A failed load keeps the previous plugin
  (existing behavior) and the error is surfaced to the UI.

### Config precedence

TOML `[skills.<name>]` values remain valid (backward compatible). Values
edited in the UI are stored in SQLite and override TOML **key-by-key**.
Bootstrap infra is TOML-only.

### Data model (SQLite, via athena-voice-storage)

- `skill_settings(skill, key, value, is_secret, updated_at)` — UI-edited
  config, overrides TOML per key.
- `skill_state(skill, enabled)` — enable/disable flag the registry respects.
- `admin_auth(token_hash, created_at)` — argon2 hash of the admin token.

## Skill config schema

The skill SDK gains an optional export `config_schema()` returning JSON:

```json
{ "fields": [
  { "key": "base_url", "label": "Jeedom URL", "type": "url",
    "required": true, "secret": false, "help": "…", "default": "" },
  { "key": "api_key", "label": "API key", "type": "secret", "required": true },
  { "key": "sensors", "label": "Sensors", "type": "list",
    "item_fields": [
      { "key": "name", "type": "string" },
      { "key": "id",   "type": "number" },
      { "key": "unit", "type": "string" } ] }
] }
```

- Field types: `string`, `number`, `secret`, `url`, `host` (feeds the HTTP
  allowlist), `list` (typed item fields; serializes to the JSON string the
  skill already reads via `host_config_get`, preserving JSON types —
  Jeedom's `id` stays a number).
- The registry calls the export at load time and caches it.
- Skills without the export get the free-form key/value editor.
- All four bundled skills (weather, jeedom, timer, home) ship schemas as the
  reference examples.

## API and auth

First run: generate an admin token, print it to the terminal once, store
only its argon2 hash in SQLite. All `/api/*` routes require
`Authorization: Bearer <token>`; the frontend prompts once and keeps it in
`localStorage`. Default bind is `127.0.0.1`; a LAN bind is an explicit
`[server] host` choice and the token is required either way.

Endpoints:

| Route | Purpose |
|---|---|
| `GET /api/skills` | Discovered skills: load state, enabled flag, schema, config with secrets masked ("set"/"not set" — values never echoed) |
| `PUT /api/skills/:name/config` | Validate, write to SQLite, live-reload that skill |
| `POST /api/skills/:name/enable` / `disable` | Toggle `skill_state.enabled` |
| `POST /api/skills/upload` | Multipart `.wasm`, size-capped, written to skills dir |
| `GET /api/status` | Runtime health for the UI header |

Validation on write: list fields must parse as their item schema; `host`
fields must be bare hosts (reject schemes/paths — the `http://` paste
mistake).

## Frontend

Embedded static app, three screens:

1. **Skill list** — name, enabled toggle, load status, "needs config" hint.
2. **Skill detail** — form rendered from the schema; secrets are write-only
   password fields showing only set/not-set; `list` fields render as
   add/remove rows.
3. **Upload** — drag-and-drop `.wasm` plus a picker listing the bundled
   skills' prebuilt `.wasm` artifacts shipped next to the binary (built from
   the repo's `skills-*` crates at release time).

Labels localized in French and English, matching `locales`.

## Error handling

- Config validation errors return structured JSON and block the write.
- Failed skill reload keeps the previous plugin loaded; the error text is
  returned to the UI (no silent no-op).
- Unauthenticated requests get 401 with no detail.

## Testing

- Unit: DB-over-TOML merge precedence; schema JSON parsing; host/list
  validators.
- Integration (axum + in-memory SQLite): every endpoint, auth rejection,
  secret masking on read-back.
- End-to-end: upload a test `.wasm`, confirm the watcher loads it and
  `GET /api/skills` reflects it.

## Decisions log

| Question | Decision |
|---|---|
| UI scope | Configure + install skills |
| Config store | SQLite (secrets never in tracked files) |
| Apply mode | Live for skills; restart for infra |
| Auth | First-run token, localhost default bind |
| Skill forms | Skill-declared schema, key/value fallback |
| Implementation | Axum + embedded no-build frontend (A) |
| History rewrite | No — rotate the key instead |
