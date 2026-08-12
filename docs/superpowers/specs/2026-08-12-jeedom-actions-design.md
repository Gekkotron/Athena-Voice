# Jeedom on/off actions — voice-triggered device control

Status: approved design, 2026-08-12.

## Purpose

Let the assistant EXECUTE Jeedom action commands — "allume la lumière du
salon" / "turn off the garage light" — not just read sensors. Devices are
discovered and configured in the admin UI like sensors are today, with an
optional per-device confirmation step.

## Non-goals

- Only on/off pairs. No dimmers, sliders, colors, or scenarios.
- No runtime/session changes: the confirmation flow lives entirely in the
  skill via the existing `tmp_set`/`tmp_get` host KV.
- No changes to the read-sensor feature.

## How execution works

Jeedom's HTTP API runs an action command with the same GET the skill
already uses for reads: `core/api/jeeApi.php?apikey=…&type=cmd&id=<id>`.
For an action-type command this call executes it. No new host functions.

One host-function behavior fix is required: Jeedom answers an action
execution with plain text or an empty body, and the existing
`http_get_json` host function currently treats a non-JSON body as an
error. `fetch_json` (runtime `wasm/host_fns.rs`) is changed to fall back
to wrapping a non-JSON 2xx body as a JSON string instead of erroring;
transport and HTTP-status failures still error. This also improves
reads of plain-string sensors.

## Skill config (`skills-jeedom`)

New `actions` config field, a JSON list parallel to `sensors`:

```json
[{ "name": "lumière du salon", "room": "salon", "prefix": "du",
   "on_id": 124, "off_id": 125, "confirm": false }]
```

- Like sensors, `name` stores the FULL composed spoken name ("lumière du
  salon") — the discovery UI composes it client-side exactly as it does
  for sensors; `room`/`prefix` are auxiliary metadata.
- All spoken fields pass through the existing `clean_spoken`.
- `confirm: true` marks the device as requiring spoken confirmation. It
  is a real JSON boolean, backed by a new `FieldKind::Bool` item-field
  kind (SDK schema + admin validation + a checkbox cell in the admin
  list editor).

## Intents and phrases

Registered per configured device (`key` = the device's `on_id`):

- `jeedom.turn_on.{key}` — FR: "allume {spoken}", "active {spoken}";
  EN: "turn on {spoken}", "switch on {spoken}".
- `jeedom.turn_off.{key}` — FR: "éteins {spoken}", "coupe {spoken}",
  "désactive {spoken}"; EN: "turn off {spoken}", "switch off {spoken}".

Global (registered once, only when at least one device has
`confirm: true`):

- `jeedom.confirm` — FR: "oui", "confirme", "c'est confirmé";
  EN: "yes", "confirm".
- `jeedom.cancel` — FR: "non", "annule"; EN: "no", "cancel".

Answers:

- Direct execution → FR "C'est fait." / EN "Done."
- HTTP failure → reuse the sensor error phrasing ("je n'arrive pas à
  joindre Jeedom" / "I can't reach Jeedom").

## Confirmation flow (skill-side state)

- On a `turn_on`/`turn_off` intent whose device has `confirm: true`:
  store `{"cmd_id": …, "label": "<action verbale>"}` under tmp key
  `pending_action` with a 30 s TTL, and answer
  FR "Tu confirmes : allumer lumière du salon ?" /
  EN "Confirm: turn on living room light?" — no execution yet.
- `jeedom.confirm`: read `pending_action`. Present → execute the stored
  command id, clear the key, answer "C'est fait." Absent/expired → FR
  "Rien à confirmer." / EN "Nothing to confirm."
- Clearing convention: the tmp KV has no delete, so "clear" = overwrite
  with an EMPTY payload (1 s TTL), and every reader treats an empty
  payload the same as an absent key.
- `jeedom.cancel`: clear the key; pending → FR "Annulé." / EN
  "Cancelled."; nothing pending → same "Rien à confirmer."
- A new turn_on/turn_off while one is pending simply overwrites the
  pending action.

## Admin discovery (`athena-voice-admin`)

- `parse_fulldata` additionally collects `type == "action"` commands per
  equipment (id, name, `generic_type` when present).
- Pairing into devices, per equipment:
  1. `generic_type` pairs first: a command ending `_ON` pairs with the
     same-prefix `_OFF` (e.g. `LIGHT_ON`/`LIGHT_OFF`, `ENERGY_ON`/
     `ENERGY_OFF`).
  2. Remaining commands pair by name (case-insensitive): On/Off,
     Allumer/Éteindre, Marche/Arrêt, Activer/Désactiver.
  3. Unpaired action commands are ignored (out of scope).
- The discovery API response gains an `actions` array per equipment:
  `{ on_id, off_id }`. The spoken name is composed CLIENT-side from the
  equipment name + room, exactly like sensors (`composeSensorName`).
- UI: the discovery tree shows an action row per paired device beside the
  sensor rows, with (a) an include checkbox, (b) an editable spoken-name
  field pre-filled with `suggested_name`, (c) a "confirmation" checkbox
  (default off). Saving writes the `actions` config value through the
  existing put_config + reload path.
- The "You can say…" phrase preview lists the turn_on/turn_off phrases
  for saved devices (via the existing phrases endpoint, which reads the
  wasm's registered rules — no special-casing).
- Column headers follow the existing list-editor convention (raw item
  keys, like the sensors table); the discovery row badge is localized
  (en "on/off device" / fr "appareil on/off").

## Error handling

- Malformed `actions` config entries are skipped (same tolerance as
  `parse_sensors`).
- Executing against an id Jeedom rejects → same unreachable/error phrase
  as sensor reads; the pending key is still cleared on a confirmed
  execution attempt (no retry loop).

## Testing

- Skill unit tests: `actions` parsing + sanitization; pattern generation
  for both locales; confirm flow against the SDK's `for_testing` host
  (pending set → confirm executes; expired/absent → "rien à confirmer";
  cancel clears).
- Admin unit tests: pairing over a canned `fullData` JSON covering
  generic_type pairs, FR name pairs, and an unpaired leftover; discovery
  response shape.
- Registry integration test: a saved action device's "allume …" phrase
  resolves to `jeedom.turn_on.{id}` through the real wasm, mirroring the
  existing prefixed-sensor-phrase test.
- Manual: real Jeedom on the GEEKOM — flip a plug on/off by voice and via
  the admin test console, with and without the confirmation checkbox.
