-- Admin/web-UI state: per-skill settings (secrets included — this file
-- lives outside the repo), enable flags, and the admin token hash.
CREATE TABLE IF NOT EXISTS skill_settings (
    skill      TEXT NOT NULL,
    key        TEXT NOT NULL,
    value      TEXT NOT NULL,
    is_secret  INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    PRIMARY KEY (skill, key)
);

CREATE TABLE IF NOT EXISTS skill_state (
    skill   TEXT PRIMARY KEY,
    enabled INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS admin_auth (
    id         INTEGER PRIMARY KEY CHECK (id = 1),
    token_hash TEXT NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
);
