use chrono::{DateTime, NaiveDateTime, SecondsFormat, Utc};

use crate::error::{AppError, AppResult};
use turso::{Builder, Database};

pub const MIGRATIONS: &str = r#"
CREATE TABLE IF NOT EXISTS users (
    id INTEGER PRIMARY KEY NOT NULL,
    username TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS exercises (
    id INTEGER PRIMARY KEY NOT NULL,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    description TEXT,
    archived BOOLEAN NOT NULL DEFAULT FALSE,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(name, user_id)
);

CREATE TABLE IF NOT EXISTS workout_templates (
    id INTEGER PRIMARY KEY NOT NULL,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    day_of_week INTEGER,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS template_exercises (
    id INTEGER PRIMARY KEY NOT NULL,
    template_id INTEGER NOT NULL REFERENCES workout_templates(id) ON DELETE CASCADE,
    exercise_id INTEGER NOT NULL REFERENCES exercises(id),
    display_order INTEGER NOT NULL,
    target_sets INTEGER,
    UNIQUE(template_id, display_order)
);

CREATE TABLE IF NOT EXISTS sessions (
    id INTEGER PRIMARY KEY NOT NULL,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    notes TEXT,
    started_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS workout_sets (
    id INTEGER PRIMARY KEY NOT NULL,
    session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    exercise_id INTEGER NOT NULL REFERENCES exercises(id),
    set_number INTEGER NOT NULL,
    weight_kg REAL NOT NULL,
    reps INTEGER NOT NULL,
    notes TEXT,
    completed_at DATETIME NOT NULL,
    UNIQUE(session_id, exercise_id, set_number)
);

CREATE INDEX IF NOT EXISTS idx_sets_session ON workout_sets(session_id);
CREATE INDEX IF NOT EXISTS idx_sets_exercise_time ON workout_sets(exercise_id, completed_at);
"#;

pub async fn init_database(path: &str) -> AppResult<Database> {
    let db = Builder::new_local(path).build().await?;
    let conn = db.connect()?;
    conn.execute_batch(MIGRATIONS).await?;
    Ok(db)
}

pub async fn ensure_default_user(db: &Database, username: &str) -> AppResult<i64> {
    let conn = db.connect()?;
    let password_placeholder = "placeholder-hash";
    conn.execute(
        "INSERT OR IGNORE INTO users (username, password_hash) VALUES (?1, ?2)",
        (username, password_placeholder),
    )
    .await?;

    let mut stmt = conn
        .prepare("SELECT id FROM users WHERE username = ?1")
        .await?;
    let row = stmt.query_row([username]).await?;
    let id: i64 = row.get(0)?;
    Ok(id)
}

pub fn serialize_timestamp(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub fn parse_timestamp(raw: &str) -> AppResult<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(raw) {
        return Ok(dt.with_timezone(&Utc));
    }

    NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S"))
        .map(|naive| DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc))
        .map_err(|err| AppError::BadRequest(format!("invalid timestamp '{raw}': {err}")))
}

pub fn now_utc() -> DateTime<Utc> {
    Utc::now()
}
