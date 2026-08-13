# Jeedom shutters — voice-controlled volets + admin UI refresh

Status: approved design, 2026-08-13.

## Purpose

Let the assistant drive Jeedom roller shutters (volets) by voice — "ouvre
le volet du salon", "ferme tous les volets", "stop le volet", "mets le
volet à 50" — building on the on/off action machinery shipped on
2026-08-12. Shutters are discovered and configured in the admin UI like
sensors and on/off devices. Alongside, the admin UI gets a structure and
polish pass: human column labels, a sectioned Jeedom page, and a refined
visual skin.

## Non-goals

- No tilt/orientation control (venetian-blind slats).
- No scenes/scenarios, no per-room shutter groups ("ferme les volets du
  salon") — only the single global all-shutters command.
- No runtime/session changes: confirmation reuses the skill-side
  `tmp_set`/`tmp_get` pending machinery as-is (one field added to the
  stored payload).
- No JS framework or build step for the admin UI — it stays vanilla.

## Skill config (`skills-jeedom`)

New `shutters` config field, a JSON list parallel to `sensors` and
`actions`:

```json
[{ "name": "volet du salon", "room": "salon", "prefix": "du",
   "up_id": 210, "down_id": 211, "stop_id": 212, "slider_id": 213,
   "confirm": false }]
```

- `name` stores the FULL composed spoken name; `room`/`prefix` are
  auxiliary metadata; all spoken fields pass through `clean_spoken`.
- `up_id`/`down_id` required; `stop_id` and `slider_id` optional
  (`Option<u64>`, serde-defaulted to absent). Absent → the corresponding
  intents are simply not registered for that shutter.
- `confirm: true` gates open/close/position behind the existing spoken
  confirmation. Stop is NEVER gated — stopping a moving shutter must be
  immediate. The global all-shutters command is not gated either.

## Intents and phrases

Per configured shutter (`key` = the shutter's `up_id`):

- `jeedom.shutter_open.{key}` — FR: "ouvre le {spoken}", "ouvre la
  {spoken}", "ouvre {spoken}", "monte le {spoken}", "lève le {spoken}";
  EN: "open the {spoken}", "open {spoken}", "raise the {spoken}".
  Executes `up_id`.
- `jeedom.shutter_close.{key}` — FR: "ferme le {spoken}", "ferme la
  {spoken}", "ferme {spoken}", "descends le {spoken}", "baisse le
  {spoken}"; EN: "close the {spoken}", "close {spoken}", "lower the
  {spoken}". Executes `down_id`.
- `jeedom.shutter_stop.{key}` (only when `stop_id` set) — FR: "stop le
  {spoken}", "stop {spoken}", "arrête le {spoken}"; EN: "stop the
  {spoken}". Executes `stop_id`, immediately.
- `jeedom.shutter_pos.{key}` (only when `slider_id` set) — a `position`
  Number slot: FR: "ouvre le {spoken} à {position}", "ouvre le {spoken}
  à {position} pour cent", "mets le {spoken} à {position}", "mets le
  {spoken} à {position} pour cent"; EN: "set the {spoken} to
  {position}", "set the {spoken} to {position} percent", "open the
  {spoken} to {position} percent". Executes `slider_id` with
  `&slider=N` appended to the jeeApi URL; N clamped to 0–100 and
  truncated to an integer. Jeedom FLAP convention: 0 = fermé,
  100 = ouvert.

Global (registered once when at least one shutter is configured):

- `jeedom.shutter_open_all` — FR: "ouvre tous les volets", "ouvre les
  volets"; EN: "open all the shutters", "open the shutters". Runs every
  shutter's `up_id`.
- `jeedom.shutter_close_all` — FR: "ferme tous les volets", "ferme les
  volets"; EN: "close all the shutters", "close the shutters". Runs
  every shutter's `down_id`.

Answers:

- Success → the existing "C'est fait." / "Done.".
- HTTP failure → the existing unreachable phrasing.
- Group partial failure → FR "C'est fait, mais N volet(s) n'ont pas
  répondu." / EN "Done, but N shutter(s) did not respond."; all
  failed → the unreachable phrasing.

## Confirmation flow

Reuses the existing pending machinery (`pending_action` tmp key, 30 s
TTL, overwrite-empty clearing, shared `jeedom.confirm`/`jeedom.cancel`
rules — registered when ANY device or shutter has `confirm: true`).

`Pending` gains `slider: Option<u64>` (serde-defaulted → old stored
payloads and the on/off flow are untouched). Confirm labels: FR
"ouvrir/fermer {name}" and "mettre {name} à N pour cent"; EN
"open/close {name}", "set {name} to N percent".

## Execution

- Open/close/stop reuse `exec_cmd` unchanged.
- Position uses a new `exec_slider(ctx, id, value)` that builds the same
  authenticated URL with `&slider={value}`.

## Admin discovery (`athena-voice-admin/src/jeedom.rs`)

- `pair_shutters()` runs BEFORE `pair_actions()` per equipment and marks
  the command ids it consumes so the on/off pass cannot steal them:
  1. `generic_type` pass: `FLAP_UP` pairs `FLAP_DOWN`; `FLAP_STOP` and
     `FLAP_SLIDER` on the same equipment attach as `stop_id`/`slider_id`.
  2. Name-vocabulary pass (case-insensitive, index-aligned like on/off):
     Monter/Descendre, Ouvrir/Fermer, Up/Down, Monté/Descendu; a command
     named "stop"/"arrêter" attaches as stop; an action command whose
     `subType` is `slider` (or named "position") attaches as slider.
  3. Unpaired leftovers fall through to the on/off pass as today.
- `DiscoveredEquipment` gains `shutters: Vec<DiscoveredShutter>`
  (`up_id`, `down_id`, `stop_id: Option`, `slider_id: Option`).
- `parse_fulldata` additionally records each action command's `subType`
  so the slider heuristic works.

## Admin UI — structure & clarity (`static/app.js`)

- **Human column labels**: a localized label map for list-editor column
  headers (falling back to the raw key): `name` → "Nom parlé"/"Spoken
  name", `id` → "Cmd", `on_id` → "Cmd ON", `off_id` → "Cmd OFF",
  `up_id` → "Cmd monter"/"Up cmd", `down_id` → "Cmd descendre"/"Down
  cmd", `stop_id` → "Cmd stop", `slider_id` → "Cmd position",
  `unit` → "Unité"/"Unit", `room` → "Pièce"/"Room", `prefix` →
  "Liaison"/"Connector", `kind` → "Type", `confirm` →
  "Confirmation", `on_label`/`off_label` → "Libellé ON/OFF".
- **Sectioned Jeedom page**: the detail card splits into titled
  sections — Connexion (URL, clé API, test button), Capteurs, Appareils
  on/off, Volets, Découverte (discover/re-sync buttons + tree). Other
  skills keep the flat field list.
- **Shutters table**: same list editor with the sensors/actions
  affordances — "Vous pouvez dire…" phrase hints (keyed
  `jeedom.shutter_open.{up_id}`), stale marker on edit, prefix
  normalization + name suffix swap, duplicate-phrase detection extended
  to shutter intents.
- **Discovery tree**: shutter rows appear beside sensor/action rows with
  a localized "volet"/"shutter" badge (plus "+ stop"/"+ position" hints
  when attached); "Add selection" composes the spoken name from the
  equipment name + room (like actions) and appends to the shutters
  table. Already-mapped shutters (by `up_id`) show checked + disabled.

## Admin UI — visual polish (`static/style.css`, `index.html`)

Same layout system, refined skin, light + dark preserved:

- Tables: zebra striping, sticky header row, borderless cells with row
  separators, compact inputs that inherit the cell rhythm.
- Buttons: hover/active/focus-visible states; quiet buttons get an
  underline-on-hover; consistent disabled treatment.
- Badges/chips color-coded by type (sensor unit, on/off, volet, warn,
  sync).
- Section headers inside cards (h3 + hairline), more consistent spacing
  scale, subtle card shadow, discovery tree indented per room with the
  room name as a sticky-ish group label.
- Target ≤ ~180 lines of CSS total; no external assets.

## Error handling

- Malformed `shutters` entries are skipped (same tolerance as sensors
  and actions).
- A position intent with a non-numeric or missing slot re-asks: FR
  "À quelle position, en pourcentage ?" / EN "To what position, in
  percent?" (no pending state).
- Unknown shutter key → the existing "je ne connais pas cet appareil"
  apology.

## Testing

- Skill unit tests: `shutters` parsing (defaults, optional ids,
  sanitization); rule generation per capability (no stop/pos rules
  without ids, group rules only when non-empty); position clamping and
  URL value; labels both locales; unknown-device and
  nothing-to-confirm handler paths; `Pending` with/without `slider`
  roundtrip and backward compat.
- Admin unit tests: `pair_shutters` generic-type pass, name-vocab pass,
  stop/slider attachment, precedence over `pair_actions` (a FLAP pair
  never doubles as an on/off device), unpaired fallthrough; fullData
  fixture with a shutter equipment through the discovery endpoint in
  `tests/api.rs`.
- Registry integration test: a saved shutter's "ouvre …" phrase resolves
  to `jeedom.shutter_open.{id}` through the real wasm, mirroring the
  on/off pinned-phrase test.
- Manual (GEEKOM + real Jeedom): open/close/stop/position one shutter by
  voice and via the admin test console; "ferme tous les volets"; browser
  pass over the restyled admin pages in light and dark.
