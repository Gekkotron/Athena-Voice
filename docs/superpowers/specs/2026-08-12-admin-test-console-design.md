# Admin test console — text command over MQTT

Status: approved design, 2026-08-12.

## Purpose

A test-only input field in the admin web UI that sends a typed command
through the real satellite path — the same MQTT topic contract an Android
app or `athena-voice-client` uses — and shows the assistant's answer.
The field keeps a client-side history of the last commands for quick recall.

## Non-goals

- No server-side persistence of commands or answers.
- No streaming/partial answers; the console shows the final answer text only.
- No audio: text in, text out.

## API

New route in `athena-voice-admin`: `POST /api/test-command`.

Request body:

```json
{ "text": "météo à Strasbourg", "locale": "fr" }
```

- `text` — required; trimmed; empty after trim → `400` with a message.
- `locale` — optional; defaults to `"en"`; validated by `Locale::new`.

Responses:

- `200` `{ "answer": "..." }` — the `tts/text` payload for the session.
- `400` — invalid body / empty text.
- `502` — broker unreachable / connection error.
- `503` — admin was started without MQTT configuration.
- `504` — no answer within the timeout (10 s).

## Dependencies

`AdminDeps` gains `mqtt: Option<AdminMqttConfig>` (host, port, username,
password — mirroring the `[mqtt]` config the CLI already loads).
`spawn_admin_ui` in `crates/athena-voice-cli/src/serve.rs` fills it from
`cfg.mqtt`. When `None` (unit-test routers, embedders that opt out), the
endpoint returns `503`.

## MQTT flow (per request)

1. Create a throwaway `rumqttc::AsyncClient` with client id
   `athena-admin-test-<pid>-<sid prefix>` so concurrent requests and the
   runtime's own client never collide.
2. Satellite id `admin-ui`, fresh `SessionId` (UUID v4).
3. Subscribe `athena/sat/admin-ui/session/<sid>/#` (QoS 0).
4. Publish `start` with `{"locale": "<locale>"}`, then `text` with the raw
   UTF-8 utterance.
5. Collect the `tts/text` payload; resolve on `done` (or on `tts/text` if
   `done` never comes but the timeout is near). Overall deadline: 10 s.
6. Publish `end`, disconnect, return.

Topic layout is the existing contract in
`crates/athena-voice-runtime/src/mqtt/topics.rs`; no runtime changes.

## UI

A "Test console" card appended to the admin page (`static/app.js`),
labeled as a test tool in both locales (fr/en):

- One text input + send button; Enter submits; the input and button are
  disabled while a request is in flight.
- The answer — or the error message — renders under the field.
- History: the last 20 distinct commands, stored in `localStorage` under
  `athena-test-history`, most recent first. ArrowUp/ArrowDown cycle
  through the history in the input (shell-style); a small clickable list
  under the card fills the input on click. Submitting a command moves it
  to the front and dedupes.
- The card sends the UI's detected locale (`fr`/`en`) with each request.

## Error handling

- Broker unreachable / connect error → `502` with the connection error
  message (distinct from `504` timeout).
- The handler always attempts to publish `end` and disconnect, including
  on timeout, so the runtime's session is closed rather than left to the
  session-manager's idle reaper.

## Testing

- Admin router unit tests: `503` when `mqtt` is `None`; `400` on empty
  text; `400` on invalid locale.
- The full round trip (broker + runtime) is verified manually with
  `athena-voice serve` and the UI, mirroring how `athena-voice-client`
  is exercised today. No new integration harness.
