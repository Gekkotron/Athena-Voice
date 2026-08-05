# Sensor management UX (admin web UI) — design

Date: 2026-08-05
Status: approved by owner (sections reviewed 2026-08-05)

Sub-project B of two (A = device actions, spec'd separately later; B lands
first because it rebuilds the table and discovery-merge foundations A's
device table will reuse).

## Goal

Make the jeedom skill's sensor table trustworthy and self-explanatory:
verify a sensor works without leaving the page, keep rows in sync with a
changing Jeedom, edit safely with the right widget per column, and see
exactly what you can say to the assistant — including when two sensors
would collide.

## New backend endpoints (athena-voice-admin)

1. `POST /api/skills/jeedom/read/{id}`
   - Host-side single-sensor read using the SAVED merged config (same
     pattern as the existing test/discover endpoints — the API key never
     reaches the browser).
   - Calls `{base}/core/api/jeeApi.php?apikey=…&type=cmd&id={id}`, 5 s
     timeout.
   - Responses: `{"status":"ok","value":"21.5"}` (value normalized like the
     skill does: bare scalar, string-wrapped number, or `{"value":…}`
     envelope), or `{"status":"unconfigured"|"unreachable"|"bad_response"}`.
   - `{id}` is a u64 path param; anything else is a 400.
2. `GET /api/skills/jeedom/phrases`
   - Queries the LOADED skill instance's `pattern_rules(locale)` through the
     registry for every configured locale, so the UI shows the truth (no
     duplicated template logic in JS).
   - Response: `{"phrases": [{"intent": "jeedom.read.142", "locale": "fr",
     "phrases": ["quelle est la temperature salon", …]}, …]}` — raw rules,
     grouped by the UI.
   - When the skill isn't loaded: `{"phrases": []}`.

Re-sync deliberately has NO new endpoint: the UI re-calls the existing
`/api/skills/jeedom/discover` and diffs client-side by sensor id.

## UI behavior (static/app.js + style.css)

Sensor table (jeedom detail view):

- **kind** column becomes a `<select>` (`numeric` / `binary`). Discovery
  already fills it from Jeedom's subtype. `on_label`/`off_label` inputs are
  disabled unless kind = binary (values preserved, just not editable).
- **Lire** button per row: calls the read endpoint, renders the live value
  inline (`21.5 °C` in the ok color) or the status in the danger color.
  Reads one row at a time; no auto-polling.
- **"Vous pouvez dire…"** hint per row: after load and after each save, the
  UI fetches `/phrases`, groups by intent id (`jeedom.read.{id}`), and shows
  that row's phrases as a muted line under the row (collapsed to the first
  two phrases + "+N"). Two warnings decorate the hint:
  - duplicate chip when another sensor shares an identical phrase (name
    collisions like six sensors all called "temperature");
  - symbols chip when the stored name/room contains characters the matcher
    strips (shows the cleaned form actually used).
  The phrase data reflects the SAVED config; after edits and before save
  the hints show a stale marker (·) rather than lying.
- **Re-sync** button next to "Découvrir les capteurs": runs discovery, then
  per existing row (matched by id) compares name/room/unit/kind/labels
  against the freshly discovered values. Differences render as one chip per
  changed field — "Jeedom: room = chambre parentale [appliquer]" — applying
  is per-chip and per-row, never bulk-silent. Rows whose id is absent from
  discovery get a "disparu de Jeedom" badge (row kept; removal stays the
  user's explicit choice). Discovered sensors NOT in the table are offered
  through the existing discovery tree flow, unchanged.

## Wire/contract constraints

- No changes to the skill, the assist protocol, or the satellite protocol.
- No changes to the config storage shape (`sensors` JSON stays as-is).
- The read endpoint reuses the redaction-safe host HTTP path (query-string
  secrets never reach logs — established by the host_fns redaction work).

## Error handling

- Read endpoint: same status taxonomy as test_connection; UI shows the
  status name, never a raw error string.
- Phrases endpoint with a config that fails to produce rules (e.g. sensors
  list empty): empty phrases array; UI shows "aucune phrase — enregistrez
  des capteurs" instead of hints.
- Re-sync with discovery failure: existing discovery error handling (the
  button already surfaces status text); the diff simply doesn't run.

## Testing

- Admin-crate wiremock tests: read endpoint ok / prose-error (unauthorized
  → bad_response mapping decision: prose body = `bad_response` here, since
  a bad key would have failed discovery already) / unreachable /
  unconfigured / non-numeric id 400; value normalization cases (bare
  scalar, string-wrapped, envelope).
- Phrases endpoint test with the real JEEDOM_TEST_WASM loaded via the
  registry: configured sensors produce `jeedom.read.{id}` groups; unloaded
  skill produces empty list.
- UI logic is vanilla JS without a test harness (repo convention); the
  duplicate/symbol detection helpers go into `app.js` as small pure
  functions reviewed by reading, exercised manually during live check.
- Live check on the GEEKOM after ship: Lire on a real sensor, re-sync after
  renaming a room in Jeedom, duplicate chip on the six "temperature" rows.

## Out of scope (→ sub-project A and later)

- Device/action commands, confirm-flagged execution flow.
- Editing phrases per sensor (custom aliases) — future.
- Client-side phrase preview of UNSAVED edits (hints refresh on save only).
