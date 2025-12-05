use std::collections::{BTreeMap, HashMap};

use axum::{
    Json, Router,
    extract::{Path, Query, State as AxumState},
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
    routing::{delete, get, post, put},
};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use turso::{Connection, Value};

use ekman_core::{
    self as core, Activity, ActivityDay, ActivityQuery, CompactSet, CreateExercise, DaySets,
    Exercise, Graph, GraphQuery, LastSession, LoginInput, Metric, Owner, RegisterInput, Session,
    SetData, SetInput, Template, TemplateExercise, TotpSetup, TotpVerify, UpdateExercise, User,
    WorkoutSet,
};

use crate::{Error, Result, State, auth, db};

const MAX_GRAPH_POINTS: usize = 50;
const ACTIVITY_DAYS: i64 = 21;

pub fn api() -> Router<State> {
    Router::new()
        // Auth
        .route("/api/auth/register", post(register))
        .route("/api/auth/login", post(login))
        .route("/api/auth/logout", post(logout))
        .route("/api/auth/me", get(me))
        .route("/api/auth/totp/setup", get(totp_setup))
        .route("/api/auth/totp/enable", post(totp_enable))
        // Plans
        .route("/api/plans", post(create_plan))
        .route("/api/plans/daily", get(daily_plans))
        .route(
            "/api/plans/{template_id}/exercises",
            post(add_exercise_to_plan),
        )
        .route(
            "/api/plans/{template_id}/exercises/{exercise_id}",
            delete(remove_exercise_from_plan),
        )
        // Activity
        .route("/api/activity/days", get(activity))
        // Exercises
        .route("/api/exercises", get(list_exercises).post(create_exercise))
        .route(
            "/api/exercises/{id}",
            get(get_exercise).patch(update_exercise),
        )
        .route("/api/exercises/{id}/archive", post(archive_exercise))
        .route("/api/exercises/{id}/graph", get(exercise_graph))
        // Sets
        .route(
            "/api/days/{date}/exercises/{exercise_id}/sets",
            get(day_sets),
        )
        .route(
            "/api/days/{date}/exercises/{exercise_id}/sets/{set_number}",
            put(upsert_set).delete(delete_set),
        )
}

// ============================================================================
// Auth handlers
// ============================================================================

async fn register(
    AxumState(state): AxumState<State>,
    Json(input): Json<RegisterInput>,
) -> Result<impl IntoResponse> {
    let username = input.username.trim();
    if username.is_empty() {
        return Err(Error::BadRequest("username required".into()));
    }
    if input.password.is_empty() {
        return Err(Error::BadRequest("password required".into()));
    }
    if input.totp_secret.is_empty() || input.totp_code.is_empty() {
        return Err(Error::BadRequest("totp required".into()));
    }

    auth::verify_totp(&input.totp_secret, &input.totp_code)?;

    let hash = auth::hash_password(&input.password)?;
    let mut conn = state.db.connect()?;

    let result = conn
        .execute(
            "INSERT INTO users (username, password_hash, totp_secret) VALUES (?, ?, ?)",
            (username, hash.as_str(), input.totp_secret.as_str()),
        )
        .await;

    if let Err(e) = result {
        if e.to_string().contains("UNIQUE constraint") {
            return Err(Error::BadRequest("username taken".into()));
        }
        return Err(e.into());
    }

    let user_id = conn.last_insert_rowid();
    let (token, expires_at) = auth::create_session(&mut conn, user_id).await?;

    let mut headers = HeaderMap::new();
    headers.insert(header::SET_COOKIE, auth::session_cookie(&token, expires_at));

    Ok((
        headers,
        Json(Session {
            user_id,
            username: username.into(),
            expires_at,
        }),
    ))
}

async fn login(
    AxumState(state): AxumState<State>,
    Json(input): Json<LoginInput>,
) -> Result<impl IntoResponse> {
    let mut conn = state.db.connect()?;
    let user = auth::user_by_username(&mut conn, &input.username).await?;

    auth::verify_password(&input.password, &user.password_hash)?;

    if !user.totp_enabled {
        return Err(Error::Unauthorized);
    }
    let code = input.totp.as_deref().ok_or(Error::Unauthorized)?;
    auth::verify_totp(&user.totp_secret, code)?;

    let (token, expires_at) = auth::create_session(&mut conn, user.id).await?;

    let mut headers = HeaderMap::new();
    headers.insert(header::SET_COOKIE, auth::session_cookie(&token, expires_at));

    Ok((
        headers,
        Json(Session {
            user_id: user.id,
            username: user.username,
            expires_at,
        }),
    ))
}

async fn logout(
    AxumState(state): AxumState<State>,
    headers: HeaderMap,
) -> Result<impl IntoResponse> {
    if let Some(token) = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| {
            s.split(';')
                .find_map(|p| p.trim().strip_prefix("ekman_session="))
        })
    {
        let mut conn = state.db.connect()?;
        auth::delete_session(&mut conn, token).await?;
    }

    let mut resp_headers = HeaderMap::new();
    resp_headers.insert(header::SET_COOKIE, auth::clear_cookie());
    Ok((resp_headers, StatusCode::NO_CONTENT))
}

async fn me(AxumState(state): AxumState<State>, headers: HeaderMap) -> Result<Json<User>> {
    let mut conn = state.db.connect()?;
    let user = auth::user_from_headers(&mut conn, &headers).await?;

    Ok(Json(User {
        user_id: user.id,
        username: user.username,
        totp_enabled: user.totp_enabled,
    }))
}

async fn totp_setup(
    AxumState(state): AxumState<State>,
    headers: HeaderMap,
) -> Result<Json<TotpSetup>> {
    let mut conn = state.db.connect()?;
    let user = auth::user_from_headers(&mut conn, &headers).await?;

    let (secret, url) = auth::generate_totp_secret(&user.username)?;

    conn.execute(
        "UPDATE users SET totp_secret = ?, totp_enabled = 0 WHERE id = ?",
        (secret.as_str(), user.id),
    )
    .await?;

    Ok(Json(TotpSetup {
        secret,
        otpauth_url: url,
    }))
}

async fn totp_enable(
    AxumState(state): AxumState<State>,
    headers: HeaderMap,
    Json(input): Json<TotpVerify>,
) -> Result<impl IntoResponse> {
    let mut conn = state.db.connect()?;
    let user = auth::user_from_headers(&mut conn, &headers).await?;

    auth::verify_totp(&user.totp_secret, &input.code)?;

    conn.execute("UPDATE users SET totp_enabled = 1 WHERE id = ?", [user.id])
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

// ============================================================================
// Plans
// ============================================================================

async fn daily_plans(
    AxumState(state): AxumState<State>,
    headers: HeaderMap,
) -> Result<Json<Vec<Template>>> {
    let mut conn = state.db.connect()?;
    let user = auth::user_from_headers(&mut conn, &headers).await?;

    let mut rows = conn
        .query(
            "SELECT wt.id, wt.name, wt.day_of_week, te.exercise_id, te.target_sets, e.name
             FROM workout_templates wt
             LEFT JOIN template_exercises te ON te.template_id = wt.id
             LEFT JOIN exercises e ON e.id = te.exercise_id
             WHERE wt.user_id = ?
             ORDER BY wt.id, te.display_order",
            [user.id],
        )
        .await?;

    let mut templates: BTreeMap<i64, Template> = BTreeMap::new();
    let mut exercise_ids = Vec::new();

    while let Some(row) = rows.next().await? {
        let template_id: i64 = row.get(0)?;
        let name: String = row.get(1)?;
        let day_of_week: Option<i64> = row.get(2)?;
        let exercise_id: Option<i64> = row.get(3)?;
        let target_sets: Option<i64> = row.get(4)?;
        let exercise_name: Option<String> = row.get(5)?;

        let template = templates.entry(template_id).or_insert_with(|| Template {
            id: template_id,
            name,
            day_of_week: day_of_week.map(|d| d as i32),
            exercises: Vec::new(),
        });

        if let Some(ex_id) = exercise_id {
            exercise_ids.push(ex_id);
            template.exercises.push(TemplateExercise {
                exercise_id: ex_id,
                name: exercise_name.unwrap_or_default(),
                target_sets: target_sets.map(|t| t as i32),
                last_session: None,
            });
        }
    }

    // Load last sessions
    let last_sessions = load_last_sessions(&conn, user.id, &exercise_ids).await?;

    let mut result: Vec<Template> = templates.into_values().collect();
    for template in &mut result {
        for ex in &mut template.exercises {
            ex.last_session = last_sessions.get(&ex.exercise_id).cloned();
        }
    }

    Ok(Json(result))
}

#[derive(serde::Deserialize)]
struct CreatePlanInput {
    name: String,
    day_of_week: Option<i32>,
}

#[derive(serde::Serialize)]
struct CreatedPlan {
    id: i64,
    name: String,
    day_of_week: Option<i32>,
}

async fn create_plan(
    AxumState(state): AxumState<State>,
    headers: HeaderMap,
    Json(input): Json<CreatePlanInput>,
) -> Result<impl IntoResponse> {
    let mut conn = state.db.connect()?;
    let user = auth::user_from_headers(&mut conn, &headers).await?;

    // Check if a plan for this weekday already exists
    if let Some(day) = input.day_of_week {
        let mut stmt = conn
            .prepare("SELECT id FROM workout_templates WHERE user_id = ? AND day_of_week = ?")
            .await?;
        if stmt.query_row((user.id, day)).await.is_ok() {
            return Err(Error::BadRequest(
                "A plan for this day already exists".into(),
            ));
        }
    }

    conn.execute(
        "INSERT INTO workout_templates (user_id, name, day_of_week) VALUES (?, ?, ?)",
        (user.id, input.name.as_str(), input.day_of_week),
    )
    .await?;

    let id = conn.last_insert_rowid();

    Ok((
        StatusCode::CREATED,
        Json(CreatedPlan {
            id,
            name: input.name,
            day_of_week: input.day_of_week,
        }),
    ))
}

async fn load_last_sessions(
    conn: &Connection,
    user_id: i64,
    exercise_ids: &[i64],
) -> Result<HashMap<i64, LastSession>> {
    let mut result = HashMap::new();

    for &ex_id in exercise_ids {
        let mut stmt = conn
            .prepare(
                "SELECT ws.day, MAX(ws.completed_at)
                 FROM workout_sets ws
                 JOIN exercises e ON e.id = ws.exercise_id
                 WHERE ws.exercise_id = ? AND e.user_id = ?
                 GROUP BY ws.day
                 ORDER BY MAX(ws.completed_at) DESC
                 LIMIT 1",
            )
            .await?;

        let Ok(row) = stmt.query_row((ex_id, user_id)).await else {
            continue;
        };

        let day: String = row.get(0)?;
        let completed: String = row.get(1)?;
        let date = db::parse_timestamp(&completed)?;

        let sets = load_day_sets(conn, ex_id, user_id, &day).await?;
        result.insert(ex_id, LastSession { date, sets });
    }

    Ok(result)
}

async fn load_day_sets(
    conn: &Connection,
    exercise_id: i64,
    user_id: i64,
    day: &str,
) -> Result<Vec<CompactSet>> {
    let mut rows = conn
        .query(
            "SELECT ws.weight_kg, ws.reps
             FROM workout_sets ws
             JOIN exercises e ON e.id = ws.exercise_id
             WHERE ws.exercise_id = ? AND e.user_id = ? AND ws.day = ?
             ORDER BY ws.set_number",
            (exercise_id, user_id, day),
        )
        .await?;

    let mut sets = Vec::new();
    while let Some(row) = rows.next().await? {
        sets.push(CompactSet {
            weight: row.get(0)?,
            reps: row.get::<i64>(1)? as i32,
        });
    }
    Ok(sets)
}

// ============================================================================
// Activity
// ============================================================================

async fn activity(
    AxumState(state): AxumState<State>,
    Query(query): Query<ActivityQuery>,
    headers: HeaderMap,
) -> Result<Json<Activity>> {
    let end = query.end.unwrap_or_else(db::now);
    let start = query
        .start
        .unwrap_or_else(|| end - Duration::days(ACTIVITY_DAYS - 1));

    let start_date = start.date_naive();
    let end_date = end.date_naive();

    if start_date > end_date {
        return Err(Error::BadRequest("start must be before end".into()));
    }

    let mut conn = state.db.connect()?;
    let user = auth::user_from_headers(&mut conn, &headers).await?;

    // Query counts
    let start_ts = start_date.and_hms_opt(0, 0, 0).unwrap();
    let end_ts = end_date.and_hms_opt(23, 59, 59).unwrap();

    let mut rows = conn
        .query(
            "SELECT DATE(ws.completed_at), COUNT(*)
             FROM workout_sets ws
             JOIN exercises e ON e.id = ws.exercise_id
             WHERE e.user_id = ? AND ws.completed_at >= ? AND ws.completed_at <= ?
             GROUP BY DATE(ws.completed_at)",
            (
                user.id,
                db::timestamp(DateTime::from_naive_utc_and_offset(start_ts, Utc)),
                db::timestamp(DateTime::from_naive_utc_and_offset(end_ts, Utc)),
            ),
        )
        .await?;

    let mut counts: HashMap<String, i64> = HashMap::new();
    while let Some(row) = rows.next().await? {
        let day: String = row.get(0)?;
        let count: i64 = row.get(1)?;
        counts.insert(day, count);
    }

    // Build full range
    let total_days = end_date.signed_duration_since(start_date).num_days();
    let days = (0..=total_days)
        .map(|offset| {
            let date = start_date + Duration::days(offset);
            let date_str = date.format("%Y-%m-%d").to_string();
            ActivityDay {
                sets_completed: counts.get(&date_str).copied().unwrap_or(0),
                date: date_str,
            }
        })
        .collect();

    Ok(Json(Activity { days }))
}

// ============================================================================
// Exercises
// ============================================================================

async fn list_exercises(
    AxumState(state): AxumState<State>,
    headers: HeaderMap,
) -> Result<Json<Vec<Exercise>>> {
    let mut conn = state.db.connect()?;
    let user = auth::user_from_headers(&mut conn, &headers).await?;

    let mut rows = conn
        .query(
            "SELECT id, name, description, archived, user_id
             FROM exercises
             WHERE (user_id = ? OR user_id IS NULL) AND archived = 0
             ORDER BY name",
            [user.id],
        )
        .await?;

    let mut exercises = Vec::new();
    while let Some(row) = rows.next().await? {
        exercises.push(Exercise {
            id: row.get(0)?,
            name: row.get(1)?,
            description: row.get(2)?,
            archived: row.get::<i64>(3)? != 0,
            owner: if row.get::<Option<i64>>(4)?.is_some() {
                Owner::User
            } else {
                Owner::Admin
            },
        });
    }

    Ok(Json(exercises))
}

async fn get_exercise(
    AxumState(state): AxumState<State>,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> Result<Json<Exercise>> {
    let mut conn = state.db.connect()?;
    let user = auth::user_from_headers(&mut conn, &headers).await?;
    Ok(Json(fetch_exercise(&conn, id, user.id).await?))
}

async fn create_exercise(
    AxumState(state): AxumState<State>,
    headers: HeaderMap,
    Json(input): Json<CreateExercise>,
) -> Result<Json<Exercise>> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err(Error::BadRequest("name required".into()));
    }

    let mut conn = state.db.connect()?;
    let user = auth::user_from_headers(&mut conn, &headers).await?;

    conn.execute(
        "INSERT INTO exercises (user_id, name, description) VALUES (?, ?, ?)",
        (user.id, name, input.description.as_deref()),
    )
    .await?;

    Ok(Json(Exercise {
        id: conn.last_insert_rowid(),
        name: name.into(),
        description: input.description,
        archived: false,
        owner: Owner::User,
    }))
}

async fn update_exercise(
    AxumState(state): AxumState<State>,
    Path(id): Path<i64>,
    headers: HeaderMap,
    Json(input): Json<UpdateExercise>,
) -> Result<Json<Exercise>> {
    if input.name.is_none() && input.description.is_none() && input.archived.is_none() {
        return Err(Error::BadRequest("no fields to update".into()));
    }

    let mut conn = state.db.connect()?;
    let user = auth::user_from_headers(&mut conn, &headers).await?;

    let mut sql = String::from("UPDATE exercises SET ");
    let mut params: Vec<Value> = Vec::new();
    let mut parts = Vec::new();

    if let Some(name) = input.name {
        parts.push("name = ?");
        params.push(name.into());
    }
    if let Some(desc) = input.description {
        parts.push("description = ?");
        params.push(desc.into());
    }
    if let Some(archived) = input.archived {
        parts.push("archived = ?");
        params.push((archived as i32).into());
    }

    sql.push_str(&parts.join(", "));
    sql.push_str(" WHERE id = ? AND user_id = ?");
    params.push(id.into());
    params.push(user.id.into());

    let updated = conn.execute(&sql, params).await?;
    if updated == 0 {
        return Err(Error::NotFound("exercise".into()));
    }

    Ok(Json(fetch_exercise(&conn, id, user.id).await?))
}

async fn archive_exercise(
    AxumState(state): AxumState<State>,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> Result<Json<Exercise>> {
    let mut conn = state.db.connect()?;
    let user = auth::user_from_headers(&mut conn, &headers).await?;

    let updated = conn
        .execute(
            "UPDATE exercises SET archived = 1 WHERE id = ? AND user_id = ?",
            (id, user.id),
        )
        .await?;

    if updated == 0 {
        return Err(Error::NotFound("exercise".into()));
    }

    Ok(Json(fetch_exercise(&conn, id, user.id).await?))
}

async fn exercise_graph(
    AxumState(state): AxumState<State>,
    Path(id): Path<i64>,
    Query(query): Query<GraphQuery>,
    headers: HeaderMap,
) -> Result<Json<Graph>> {
    let metric = query.metric.unwrap_or(Metric::MaxWeight);

    if let (Some(start), Some(end)) = (query.start, query.end)
        && start > end
    {
        return Err(Error::BadRequest("start must be before end".into()));
    }

    let mut conn = state.db.connect()?;
    let user = auth::user_from_headers(&mut conn, &headers).await?;
    let exercise = fetch_exercise(&conn, id, user.id).await?;

    // Build query
    let mut sql = String::from(
        "SELECT ws.day, ws.weight_kg, ws.reps
         FROM workout_sets ws
         JOIN exercises e ON e.id = ws.exercise_id
         WHERE ws.exercise_id = ? AND (e.user_id = ? OR e.user_id IS NULL)",
    );

    let mut params: Vec<Value> = vec![id.into(), user.id.into()];

    if let Some(start) = query.start {
        sql.push_str(" AND ws.completed_at >= ?");
        params.push(db::timestamp(start).into());
    }
    if let Some(end) = query.end {
        sql.push_str(" AND ws.completed_at <= ?");
        params.push(db::timestamp(end).into());
    }

    sql.push_str(" ORDER BY ws.completed_at");

    let mut rows = conn.query(&sql, params).await?;
    let mut sets = Vec::new();

    while let Some(row) = rows.next().await? {
        let day: String = row.get(0)?;
        let date = NaiveDate::parse_from_str(&day, "%Y-%m-%d")
            .map_err(|e| Error::Internal(format!("bad date: {e}")))?;

        sets.push(SetData {
            date,
            weight: row.get(1)?,
            reps: row.get::<i64>(2)? as i32,
        });
    }

    let points = core::build_graph(sets, metric, MAX_GRAPH_POINTS);

    Ok(Json(Graph {
        exercise_id: id,
        exercise_name: exercise.name,
        points,
    }))
}

async fn fetch_exercise(conn: &Connection, id: i64, user_id: i64) -> Result<Exercise> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name, description, archived, user_id
             FROM exercises
             WHERE id = ? AND (user_id = ? OR user_id IS NULL)",
        )
        .await?;

    let row = stmt
        .query_row((id, user_id))
        .await
        .map_err(|_| Error::NotFound("exercise".into()))?;

    Ok(Exercise {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        archived: row.get::<i64>(3)? != 0,
        owner: if row.get::<Option<i64>>(4)?.is_some() {
            Owner::User
        } else {
            Owner::Admin
        },
    })
}

// ============================================================================
// Sets
// ============================================================================

#[derive(serde::Deserialize)]
pub struct SetPath {
    date: String,
    exercise_id: i64,
}

#[derive(serde::Deserialize)]
pub struct SetPathFull {
    date: String,
    exercise_id: i64,
    set_number: i32,
}

async fn day_sets(
    AxumState(state): AxumState<State>,
    Path(path): Path<SetPath>,
    headers: HeaderMap,
) -> Result<Json<DaySets>> {
    let day = parse_day(&path.date)?;

    let mut conn = state.db.connect()?;
    let user = auth::user_from_headers(&mut conn, &headers).await?;

    // Verify exercise ownership
    let _ = fetch_exercise(&conn, path.exercise_id, user.id).await?;

    let mut rows = conn
        .query(
            "SELECT id, set_number, weight_kg, reps, completed_at
             FROM workout_sets
             WHERE exercise_id = ? AND day = ?
             ORDER BY set_number",
            (path.exercise_id, day.to_string()),
        )
        .await?;

    let mut sets = Vec::new();
    while let Some(row) = rows.next().await? {
        sets.push(WorkoutSet {
            id: row.get(0)?,
            exercise_id: path.exercise_id,
            day: path.date.clone(),
            set_number: row.get::<i64>(1)? as i32,
            weight: row.get(2)?,
            reps: row.get::<i64>(3)? as i32,
            completed_at: db::parse_timestamp(&row.get::<String>(4)?)?,
        });
    }

    Ok(Json(DaySets { sets }))
}

async fn upsert_set(
    AxumState(state): AxumState<State>,
    Path(path): Path<SetPathFull>,
    headers: HeaderMap,
    Json(input): Json<SetInput>,
) -> Result<Json<WorkoutSet>> {
    if path.set_number < 1 {
        return Err(Error::BadRequest("set_number must be >= 1".into()));
    }
    if input.reps < 1 {
        return Err(Error::BadRequest("reps must be >= 1".into()));
    }
    if input.weight < 0.0 {
        return Err(Error::BadRequest("weight must be >= 0".into()));
    }

    let day = parse_day(&path.date)?;

    let completed_at = input.completed_at.unwrap_or_else(|| {
        let noon = day.and_hms_opt(12, 0, 0).unwrap();
        DateTime::from_naive_utc_and_offset(noon, Utc)
    });

    // Clamp time to the day
    let clamped = day.and_time(completed_at.time());
    let completed_at = DateTime::from_naive_utc_and_offset(clamped, Utc);

    let mut conn = state.db.connect()?;
    let user = auth::user_from_headers(&mut conn, &headers).await?;

    // Verify ownership
    let _ = fetch_exercise(&conn, path.exercise_id, user.id).await?;

    conn.execute(
        "INSERT INTO workout_sets (exercise_id, day, set_number, weight_kg, reps, completed_at)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(exercise_id, day, set_number) DO UPDATE SET
            weight_kg = excluded.weight_kg,
            reps = excluded.reps,
            completed_at = excluded.completed_at",
        (
            path.exercise_id,
            day.to_string(),
            path.set_number,
            input.weight,
            input.reps,
            db::timestamp(completed_at),
        ),
    )
    .await?;

    // Get the id
    let mut stmt = conn
        .prepare("SELECT id FROM workout_sets WHERE exercise_id = ? AND day = ? AND set_number = ?")
        .await?;

    let row = stmt
        .query_row((path.exercise_id, day.to_string(), path.set_number))
        .await?;

    Ok(Json(WorkoutSet {
        id: row.get(0)?,
        exercise_id: path.exercise_id,
        day: day.to_string(),
        set_number: path.set_number,
        weight: input.weight,
        reps: input.reps,
        completed_at,
    }))
}

async fn delete_set(
    AxumState(state): AxumState<State>,
    Path(path): Path<SetPathFull>,
    headers: HeaderMap,
) -> Result<impl IntoResponse> {
    if path.set_number < 1 {
        return Err(Error::BadRequest("set_number must be >= 1".into()));
    }

    let day = parse_day(&path.date)?;

    let mut conn = state.db.connect()?;
    let user = auth::user_from_headers(&mut conn, &headers).await?;

    // Verify ownership
    let _ = fetch_exercise(&conn, path.exercise_id, user.id).await?;

    let deleted = conn
        .execute(
            "DELETE FROM workout_sets WHERE exercise_id = ? AND day = ? AND set_number = ?",
            (path.exercise_id, day.to_string(), path.set_number),
        )
        .await?;

    if deleted == 0 {
        return Err(Error::NotFound("set".into()));
    }

    Ok(StatusCode::NO_CONTENT)
}

fn parse_day(s: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|_| Error::BadRequest("invalid date, expected YYYY-MM-DD".into()))
}

// ============================================================================
// Plan management
// ============================================================================

#[derive(serde::Deserialize)]
struct AddExerciseInput {
    exercise_id: i64,
}

async fn add_exercise_to_plan(
    AxumState(state): AxumState<State>,
    Path(template_id): Path<i64>,
    headers: HeaderMap,
    Json(input): Json<AddExerciseInput>,
) -> Result<impl IntoResponse> {
    let mut conn = state.db.connect()?;
    let user = auth::user_from_headers(&mut conn, &headers).await?;

    // Verify template belongs to user
    let mut stmt = conn
        .prepare("SELECT id FROM workout_templates WHERE id = ? AND user_id = ?")
        .await?;
    stmt.query_row((template_id, user.id))
        .await
        .map_err(|_| Error::NotFound("template".into()))?;

    // Verify exercise exists and belongs to user
    let _ = fetch_exercise(&conn, input.exercise_id, user.id).await?;

    // Get max display order
    let mut stmt = conn
        .prepare(
            "SELECT COALESCE(MAX(display_order), 0) FROM template_exercises WHERE template_id = ?",
        )
        .await?;
    let row = stmt.query_row([template_id]).await?;
    let max_order: i64 = row.get(0)?;

    // Insert
    conn.execute(
        "INSERT INTO template_exercises (template_id, exercise_id, display_order) VALUES (?, ?, ?)",
        (template_id, input.exercise_id, max_order + 1),
    )
    .await?;

    Ok(StatusCode::CREATED)
}

async fn remove_exercise_from_plan(
    AxumState(state): AxumState<State>,
    Path((template_id, exercise_id)): Path<(i64, i64)>,
    headers: HeaderMap,
) -> Result<impl IntoResponse> {
    let mut conn = state.db.connect()?;
    let user = auth::user_from_headers(&mut conn, &headers).await?;

    // Verify template belongs to user
    let mut stmt = conn
        .prepare("SELECT id FROM workout_templates WHERE id = ? AND user_id = ?")
        .await?;
    stmt.query_row((template_id, user.id))
        .await
        .map_err(|_| Error::NotFound("template".into()))?;

    let deleted = conn
        .execute(
            "DELETE FROM template_exercises WHERE template_id = ? AND exercise_id = ?",
            (template_id, exercise_id),
        )
        .await?;

    if deleted == 0 {
        return Err(Error::NotFound("exercise not in plan".into()));
    }

    Ok(StatusCode::NO_CONTENT)
}
