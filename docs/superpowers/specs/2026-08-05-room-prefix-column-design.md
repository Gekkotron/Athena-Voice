# Room prefix column — design

Date: 2026-08-05
Status: approved by owner (approach A "with auto-detection", 2026-08-05)

Follow-up to the sensor management UX (spec 2026-08-05). Fixes rooms whose
French connector the article heuristic gets wrong — a bedroom named
"Alicia" currently yields « température du alicia » / « de l'alicia »
instead of « température d'Alicia ».

## Goal

Each sensor row gains an optional **prefix** — the French connector spoken
before the room (« du », « de la », « de l' », « d' », « chez »…). The
skill's generated phrases and the discovery-composed names use it; discovery
auto-fills it from the existing article heuristic so common rooms need no
manual work, and the odd ones become a one-cell fix.

## Data + skill (the ONLY skill change; wasm rebuild required)

- `Sensor` (skills-jeedom) gains `prefix: String` with `#[serde(default)]`,
  sanitized by `clean_spoken` at parse time like name/room (apostrophes and
  hyphens already survive `clean_spoken`).
- `config_schema`'s `sensors.item_fields` gains `prefix` (string, optional)
  right after `room` — the admin table's column set is schema-driven, so the
  UI column appears with no `listEditor` change.
- Spacing rule, used everywhere a prefix is joined to a room: no space when
  the prefix ends with an apostrophe (`'` or `’`) — « d'alicia » — otherwise
  one space — « du salon ».
- `rules_for` (fr only; en ignores the prefix and keeps "in the {room}"):
  - **prefix empty** → exactly today's both-genders enumeration, unchanged.
    Hand-edited rows without a prefix regress nothing.
  - **prefix set** → the de-form phrases
    « {metric} {P}{room} », « quelle est la {metric} {P}{room} »,
    plus locative dans-forms ONLY when the prefix maps to a definite
    article via the fixed table `du → le`, `de la → la`, `de l' → l'`:
    « {metric} dans {art}{room} », « quelle {metric} dans {art}{room} »,
    « quelle est la {metric} dans {art}{room} ». Unmapped prefixes
    (« d' », « chez ») generate no dans-forms — « dans d'Alicia » must never
    exist.
- Everything else in the skill (read path, read_all enumeration, answer
  phrasing, intent names) is untouched.

## Discovery + re-sync (admin UI)

- The article guess inside `composeSensorName` is extracted into a pure
  `guessRoomPrefix(room)` → `'du' | 'de la' | "de l'" | ''` (vowel rule and
  `FR_ROOM_ARTICLES` as today; unknown rooms give `''`).
- The discovery tree's "add selection" fills `prefix: guessRoomPrefix(room)`
  on new rows and composes the name through the same prefix — byte-identical
  names to today for known rooms.
- Re-sync composes the fresh comparison name using the ROW's stored prefix
  when set (falling back to the guess), and **prefix is excluded from
  re-sync diffs**: it is the user's correction, not a Jeedom fact — re-sync
  must never offer to revert « d' » back to « de l' ».

## Name upkeep on prefix edit

When the user edits a row's prefix, the UI recomposes the name silently
ONLY if the current name still ends with the old `prefix + room` suffix
(same spacing rule) — i.e. the name was never customized. Otherwise the
name is left alone; a hand-edited name always wins. Implemented as a small
pure helper (`swapNameSuffix(name, oldPrefix, newPrefix, room)`) reviewed by
reading.

## Wire/contract constraints

- The `sensors` JSON shape change is additive (one optional key); older
  configs load unchanged. No admin endpoint, assist-protocol, or satellite
  changes. The phrases hint column updates for free — the endpoint already
  reflects the saved config after reload.
- The rebuilt jeedom wasm ships through the existing bundled-skill refresh
  path (entrypoint refreshes unmodified bundled skills on image updates).

## Testing

- skills-jeedom unit tests on `rules_for`:
  (a) prefix « d' » + room « alicia » → contains « quelle est la
  température d'alicia » and NO « dans » form and NO « du alicia »;
  (b) prefix « du » + room « salon » → contains « température du salon »
  AND « température dans le salon »;
  (c) empty prefix → the exact legacy phrase set (regression pin);
  (d) parse test: sensors JSON without `prefix` parses with empty default.
- Admin phrases-endpoint test extended with a prefixed sensor asserting the
  elided phrase comes through the registry cache end-to-end.
- JS: `guessRoomPrefix` and `swapNameSuffix` are pure, desk-checked in node
  (salon → « du », alicia → « de l' » guess, suffix swap fires only on
  untouched names).
- Live check (owner, on the GEEKOM): set « d' » on Alicia's row, save, ask
  « quelle est la température d'Alicia ».

## Out of scope

- Auto-detecting person names (the guess stays the dumb article heuristic;
  the column exists precisely because guessing has limits).
- English-locale prefixes.
- Any re-sync chip for prefix (deliberately excluded, see above).
