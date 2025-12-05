use chrono::{DateTime, NaiveDateTime, SecondsFormat, Utc};
use turso::{Builder, Database};

use crate::{Error, Result};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS users (
    id INTEGER PRIMARY KEY,
    username TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    totp_secret TEXT NOT NULL,
    totp_enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS exercises (
    id INTEGER PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    description TEXT,
    archived INTEGER NOT NULL DEFAULT 0,
    created_at TEXT DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(name, user_id)
);

CREATE TABLE IF NOT EXISTS workout_templates (
    id INTEGER PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    day_of_week INTEGER,
    created_at TEXT DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS template_exercises (
    id INTEGER PRIMARY KEY,
    template_id INTEGER NOT NULL REFERENCES workout_templates(id) ON DELETE CASCADE,
    exercise_id INTEGER NOT NULL REFERENCES exercises(id),
    display_order INTEGER NOT NULL,
    target_sets INTEGER,
    UNIQUE(template_id, display_order)
);

CREATE TABLE IF NOT EXISTS workout_sets (
    id INTEGER PRIMARY KEY,
    exercise_id INTEGER NOT NULL REFERENCES exercises(id),
    day TEXT NOT NULL,
    set_number INTEGER NOT NULL,
    weight_kg REAL NOT NULL,
    reps INTEGER NOT NULL,
    completed_at TEXT NOT NULL,
    UNIQUE(exercise_id, day, set_number)
);

CREATE INDEX IF NOT EXISTS idx_sets_exercise_day ON workout_sets(exercise_id, day);
CREATE INDEX IF NOT EXISTS idx_sets_exercise_time ON workout_sets(exercise_id, completed_at);

CREATE TABLE IF NOT EXISTS sessions (
    id INTEGER PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token TEXT NOT NULL UNIQUE,
    expires_at TEXT NOT NULL,
    created_at TEXT DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_sessions_token ON sessions(token);
"#;

pub async fn init(path: &str) -> Result<Database> {
    let db = Builder::new_local(path).build().await?;
    db.connect()?.execute_batch(SCHEMA).await?;
    Ok(db)
}

pub fn timestamp(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub fn parse_timestamp(s: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .or_else(|_| {
            NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
                .or_else(|_| NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S"))
                .map(|naive| DateTime::from_naive_utc_and_offset(naive, Utc))
        })
        .map_err(|e| Error::BadRequest(format!("invalid timestamp '{s}': {e}")))
}

pub fn now() -> DateTime<Utc> {
    Utc::now()
}
