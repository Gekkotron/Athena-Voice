-- Athena-Voice initial schema (v1)

CREATE TABLE sessions (
    session       TEXT PRIMARY KEY NOT NULL,
    satellite     TEXT NOT NULL,
    locale        TEXT NOT NULL,
    started_at    TEXT NOT NULL,
    ended_at      TEXT,
    outcome       TEXT
);

CREATE INDEX idx_sessions_satellite ON sessions(satellite);
CREATE INDEX idx_sessions_started_at ON sessions(started_at);

CREATE TABLE events (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    session   TEXT NOT NULL,
    kind      TEXT NOT NULL,
    payload   TEXT NOT NULL,
    at        TEXT NOT NULL
);

CREATE INDEX idx_events_session ON events(session);
CREATE INDEX idx_events_kind ON events(kind);
CREATE INDEX idx_events_at ON events(at);

CREATE TABLE errors (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    session   TEXT NOT NULL,
    stage     TEXT NOT NULL,
    variant   TEXT NOT NULL,
    message   TEXT NOT NULL,
    at        TEXT NOT NULL
);

CREATE INDEX idx_errors_session ON errors(session);
CREATE INDEX idx_errors_stage ON errors(stage);

CREATE TABLE satellites (
    id             TEXT PRIMARY KEY NOT NULL,
    api_key_hash   TEXT NOT NULL,
    created_at     TEXT NOT NULL,
    last_seen      TEXT
);

CREATE TABLE skill_kv (
  skill TEXT NOT NULL,
  key TEXT NOT NULL,
  timestamp_sec INTEGER NOT NULL,
  value BLOB NOT NULL,
  PRIMARY KEY (skill, key)
);
