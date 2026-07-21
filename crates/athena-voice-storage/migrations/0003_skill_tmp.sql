-- skill_tmp: skill-local transient storage
CREATE TABLE IF NOT EXISTS skill_tmp (
    skill TEXT NOT NULL,
    key TEXT NOT NULL,
    value BLOB NOT NULL,
    expires_at INTEGER NOT NULL,
    PRIMARY KEY (skill, key)
);