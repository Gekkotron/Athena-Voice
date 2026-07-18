CREATE TABLE scheduled_events (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    skill          TEXT NOT NULL,
    fires_at_ms    INTEGER NOT NULL,
    mqtt_topic     TEXT NOT NULL,
    payload        BLOB NOT NULL,
    created_at_ms  INTEGER NOT NULL
);
CREATE INDEX idx_scheduled_events_fires_at_ms ON scheduled_events(fires_at_ms);
