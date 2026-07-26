# Jeedom discovery, rooms, and connection test — design

Date: 2026-07-26
Status: approved by user (brainstorming session)
Builds on: docs/superpowers/specs/2026-07-24-web-config-design.md (admin web UI)

## Problem

Configuring the Jeedom skill still requires hand-copying numeric command ids
from Jeedom's UI into the sensors table, and a wrong URL/key only surfaces as
a silent voice-query failure. Voice queries also require knowing each
sensor's exact configured name.

## Scope (user decisions)

- Discovery covers **all read-only `info` commands** (numeric and binary
  states) — never action commands.
- Exposure is **opt-in**: discovery shows a checkbox tree; only ticked
  entries land in the skill config.
- Approach **A**: discovery/test are host-side endpoints in the admin crate
  (Jeedom-specific module) reusing the existing config plumbing. A generic
  skill-actions mechanism (approach B) is deliberately deferred until a
  second skill needs it.

Out of scope: voice-triggered Jeedom actions/scenarios; auto-exposure;
discovery for other skills.

## 1. Admin endpoints — `crates/athena-voice-admin/src/jeedom.rs`

Both endpoints sit under the existing `/api` token middleware and operate on
the skill's **saved merged config** (base TOML + DB rows via
`apply_settings`); the UI nudges "save first". The API key is read
server-side and never appears in any request or response body.

- `POST /api/skills/jeedom/test` — GET
  `{base_url}/core/api/jeeApi.php?apikey={key}&type=version`, 5 s timeout.
  Response: `{ "status": "ok", "version": "…" }` or
  `{ "status": "unauthorized" | "unreachable" | "bad_response" | "unconfigured" }`.
  Status codes are machine codes; the UI localizes them (fr/en).
- `POST /api/skills/jeedom/discover` — GET `…&type=fullData`, 10 s timeout,
  response size cap (4 MiB). Filter to `type == "info"` commands. Returns:

```json
{ "rooms": [ { "name": "Salon", "equipments": [ { "name": "Capteur Xiaomi",
  "cmds": [ { "id": 142, "name": "Température", "subtype": "numeric",
              "unit": "°C", "on_label": null, "off_label": null } ] } ] } ] }
```

  Binary commands carry Jeedom's display state labels when present
  (`on_label`/`off_label`), else null. Malformed/oversized payloads yield
  `{ "status": "bad_response" }` — never a 500 with raw upstream body.

The admin crate gains a `reqwest` dependency (workspace dep exists; same
rustls feature set as the runtime).

## 2. UI — jeedom detail form additions (`static/app.js`)

- **"Tester la connexion"** button: calls the test endpoint, renders inline
  ✓ "Jeedom v4.4.19 joignable" / ✗ localized error per status code.
- **"Découvrir les capteurs"** button: renders the checkbox tree grouped by
  room (room → equipment → command with unit/state badges). Commands whose
  id already exists in the sensors table are pre-ticked and stay mapped.
- **"Ajouter la sélection"**: composes an editable spoken name per new tick
  and merges rows into the existing sensors list editor. Composition rule
  (explicit, since Jeedom gives no grammatical gender): a small built-in
  lookup of common French room words picks the article (salon/bureau/garage
  → `du`, chambre/cuisine/salle de bain/terrasse → `de la`, vowel-initial
  like entrée/étage → `de l'`); rooms not in the lookup compose as
  `"<cmd name> <room>"` with no article (e.g. `température salon`) — still
  fuzzy-matchable, and the field is editable anyway. Saving uses the normal
  PUT (validation, `$http_allowlist` derivation, live reload — all
  unchanged).
- All new labels exist in both `fr` and `en` dictionaries. `node --check`
  gate applies (lesson from Task 12).

## 3. Skill — room- and state-aware sensors (`skills-jeedom`)

Sensor entry gains optional, backward-compatible fields (serde defaults;
old configs parse unchanged):

```json
{ "name": "température du salon", "id": 142, "unit": "degrés",
  "room": "salon", "kind": "numeric", "on_label": null, "off_label": null }
```

- `kind`: `"numeric"` (default) | `"binary"`.
- Matching additions, on top of existing exact/fuzzy name matching:
  - **Room queries**: "quelle température dans le salon" — match by
    (metric word ∈ sensor name) + (room word == sensor.room), so the query
    works even when the configured name differs from the spoken phrasing.
  - **Enumeration**: "toutes les températures" / "all temperatures" —
    answers each sensor whose name contains the metric word, one clause per
    sensor ("salon 21 degrés, chambre 19 degrés").
  - **Binary answers**: value 1/0 speaks `on_label`/`off_label` when set
    ("la porte du garage est ouverte"), else "activé"/"désactivé".
- `config_schema()` updated: sensors `item_fields` gain `room` (string),
  `kind` (string), `on_label` (string), `off_label` (string) so the UI
  table shows the new columns. Wasm rebuilt via `build.sh`.

## 4. Error handling

- Jeedom unreachable/slow: endpoints return their status code within the
  timeout; the UI shows the localized message. No hangs, no retries in v1.
- Discovery with unsaved/missing base_url or api_key → `"unconfigured"`.
- fullData entries missing fields (no unit, unnamed cmd) are skipped, not
  fatal.

## 5. Testing

- Admin (`wiremock`, existing workspace dev-dep): fake `version` and
  `fullData` fixtures → test ok/unauthorized/unreachable/bad_response
  paths, the info-only filter, state-label extraction, and the assertion
  that the api_key string appears in **no** response body. Auth-required
  test for both endpoints.
- Skill unit tests: room-query matching, enumeration response shape,
  binary label phrasing (with and without labels), old-config
  backward-compat parse.
- UI: `node --check`; tree-rendering logic kept in pure functions where
  practical.

## Decisions log

| Question | Decision |
|---|---|
| Discovery scope | All read-only info commands (numeric + binary) |
| Exposure | Opt-in ticking in the UI |
| Architecture | A: host-side Jeedom module in admin crate; generic actions deferred |
| Config source for test/discover | Saved merged config only |
| Binary phrasing | Jeedom display labels, fallback activé/désactivé |
