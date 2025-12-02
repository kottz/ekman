use std::collections::{BTreeMap, HashMap, HashSet};

use argon2::{
    Argon2,
    password_hash::{
        PasswordHash, PasswordHasher, PasswordVerifier, SaltString,
        rand_core::{OsRng, RngCore},
    },
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::IntoResponse,
};
use base32::Alphabet;
use base32::encode as base32_encode;
use chrono::{DateTime, Duration as ChronoDuration, NaiveDate, Utc};
use serde::Deserialize;
use totp_rs::{Algorithm, Secret, TOTP};
use turso::{Connection, Value};

use crate::{
    AppState,
    db::{now_utc, parse_timestamp, serialize_timestamp},
    error::{AppError, AppResult},
};
use ekman_core::{
    logic::{SetDataPoint, build_graph_points},
    models::{
        ActivityDay, ActivityRequest, ActivityResponse, CreateExerciseRequest,
        DayExerciseSetsResponse, Exercise, GraphRequest, GraphResponse, LoginRequest,
        LoginResponse, MeResponse, MetricKind, PopulatedExercise, PopulatedTemplate,
        RegisterRequest, SetCompact, SetForDayItem, SetForDayRequest, SetForDayResponse,
        TotpSetupResponse, TotpVerifyRequest, UpdateExerciseRequest,
    },
};

const MAX_GRAPH_POINTS: usize = 50;
const DEFAULT_ACTIVITY_DAYS: i64 = 21;
const SESSION_COOKIE: &str = "ekman_session";
const SESSION_DAYS: i64 = 30;

pub async fn get_daily_plans(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<Json<Vec<PopulatedTemplate>>> {
    let mut conn = state.db.connect()?;
    let user = resolve_user_from_session(&mut conn, &headers).await?;
    let mut rows = conn
        .query(
            "SELECT wt.id, wt.name, wt.day_of_week, te.exercise_id, \
             te.target_sets, e.name \
             FROM workout_templates wt \
             LEFT JOIN template_exercises te ON te.template_id = wt.id \
             LEFT JOIN exercises e ON e.id = te.exercise_id \
             WHERE wt.user_id = ?1 \
             ORDER BY wt.id, te.display_order",
            [user.id],
        )
        .await?;

    let mut templates: BTreeMap<i64, PopulatedTemplate> = BTreeMap::new();
    let mut exercise_ids = HashSet::new();

    while let Some(row) = rows.next().await? {
        let template_id: i64 = row.get(0)?;
        let name: String = row.get(1)?;
        let day_of_week: Option<i64> = row.get(2)?;
        let exercise_id: Option<i64> = row.get(3)?;
        let target_sets: Option<i64> = row.get(4)?;
        let exercise_name: Option<String> = row.get(5)?;

        let entry = templates
            .entry(template_id)
            .or_insert_with(|| PopulatedTemplate {
                id: template_id,
                name: name.clone(),
                day_of_week: day_of_week.map(|d| d as i32),
                exercises: Vec::new(),
            });

        if let Some(ex_id) = exercise_id {
            exercise_ids.insert(ex_id);
            entry.exercises.push(PopulatedExercise {
                exercise_id: ex_id,
                name: exercise_name.unwrap_or_else(|| "Exercise".to_string()),
                target_sets: target_sets.map(|v| v as i32),
                last_day_date: None,
                last_day_sets: Vec::new(),
            });
        }
    }

    let last_days = load_last_days(
        &state,
        user.id,
        &exercise_ids.into_iter().collect::<Vec<_>>(),
    )
    .await?;

    let mut templates_vec: Vec<PopulatedTemplate> = templates.into_values().collect();
    for template in templates_vec.iter_mut() {
        for exercise in template.exercises.iter_mut() {
            if let Some((date, sets)) = last_days.get(&exercise.exercise_id) {
                exercise.last_day_date = date.map(|dt| dt.with_timezone(&Utc));
                exercise.last_day_sets = sets.clone();
            }
        }
    }

    Ok(Json(templates_vec))
}

pub async fn get_activity_days(
    State(state): State<AppState>,
    Query(request): Query<ActivityRequest>,
    headers: HeaderMap,
) -> AppResult<Json<ActivityResponse>> {
    let end_dt = request.end.unwrap_or_else(now_utc);
    let default_start = end_dt - ChronoDuration::days(DEFAULT_ACTIVITY_DAYS - 1);
    let start_dt = request.start.unwrap_or(default_start);

    let start_date = start_dt.date_naive();
    let end_date = end_dt.date_naive();

    if start_date > end_date {
        return Err(AppError::BadRequest(
            "start must be before or equal to end".to_string(),
        ));
    }

    let mut conn = state.db.connect()?;
    let user = resolve_user_from_session(&mut conn, &headers).await?;
    let counts = fetch_set_counts(&conn, user.id, start_date, end_date).await?;

    let total_days = end_date.signed_duration_since(start_date).num_days().max(0);
    let mut days = Vec::new();
    for offset in 0..=total_days {
        let date = start_date + ChronoDuration::days(offset);
        let sets_completed = counts.get(&date).copied().unwrap_or(0);
        days.push(ActivityDay {
            date: date.format("%Y-%m-%d").to_string(),
            sets_completed,
        });
    }

    Ok(Json(ActivityResponse { days }))
}

pub async fn login(
    State(state): State<AppState>,
    Json(payload): Json<LoginRequest>,
) -> AppResult<impl IntoResponse> {
    let mut conn = state.db.connect()?;
    let user = fetch_user(&mut conn, &payload.username).await?;

    verify_password(&payload.password, &user.password_hash, &user.password_salt)?;

    enforce_totp(&user, payload.totp.as_deref())?;

    let expires_at = now_utc() + ChronoDuration::days(SESSION_DAYS);
    let token = generate_token()?;
    create_auth_session(&mut conn, user.id, &token, expires_at).await?;

    let cookie = build_session_cookie(&token, expires_at);
    let mut headers = HeaderMap::new();
    headers.insert(header::SET_COOKIE, cookie);
    let response = LoginResponse {
        user_id: user.id,
        username: user.username,
        expires_at,
    };

    Ok((headers, Json(response)))
}

pub async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<impl IntoResponse> {
    if let Some(token) = extract_session_token(&headers) {
        let mut conn = state.db.connect()?;
        delete_auth_session(&mut conn, token).await?;
    }

    let clearing_cookie = clear_session_cookie();
    let mut headers = HeaderMap::new();
    headers.insert(header::SET_COOKIE, clearing_cookie);
    Ok((headers, StatusCode::NO_CONTENT))
}

pub async fn register(
    State(state): State<AppState>,
    Json(payload): Json<RegisterRequest>,
) -> AppResult<impl IntoResponse> {
    let username = payload.username.trim();
    if username.is_empty() {
        return Err(AppError::BadRequest("username is required".to_string()));
    }
    if payload.password.trim().is_empty() {
        return Err(AppError::BadRequest("password is required".to_string()));
    }
    let totp_secret = payload.totp_secret.trim();
    if totp_secret.is_empty() {
        return Err(AppError::BadRequest("totp_secret is required".to_string()));
    }
    if payload.totp_code.trim().is_empty() {
        return Err(AppError::BadRequest("totp_code is required".to_string()));
    }

    verify_totp(totp_secret, payload.totp_code.trim())?;

    let (hash, salt) = hash_password(&payload.password)?;
    let mut conn = state.db.connect()?;
    let insert_result = conn
        .execute(
            "INSERT INTO users (username, password_hash, password_salt, totp_secret, totp_enabled) \
             VALUES (?1, ?2, ?3, ?4, TRUE)",
            (username, hash.as_str(), salt.as_str(), totp_secret),
        )
        .await;

    if let Err(err) = insert_result {
        if is_unique_violation(&err) {
            return Err(AppError::BadRequest("username already exists".to_string()));
        }
        return Err(err.into());
    }

    let user_id = conn.last_insert_rowid();
    let expires_at = now_utc() + ChronoDuration::days(SESSION_DAYS);
    let token = generate_token()?;
    create_auth_session(&mut conn, user_id, &token, expires_at).await?;

    let cookie = build_session_cookie(&token, expires_at);
    let mut headers = HeaderMap::new();
    headers.insert(header::SET_COOKIE, cookie);
    let response = LoginResponse {
        user_id,
        username: username.to_string(),
        expires_at,
    };

    Ok((headers, Json(response)))
}

pub async fn me(State(state): State<AppState>, headers: HeaderMap) -> AppResult<Json<MeResponse>> {
    let mut conn = state.db.connect()?;
    let user = resolve_user_from_session(&mut conn, &headers).await?;

    Ok(Json(MeResponse {
        user_id: user.id,
        username: user.username,
        totp_enabled: user.totp_enabled,
    }))
}

pub async fn totp_setup(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<Json<TotpSetupResponse>> {
    let mut conn = state.db.connect()?;
    let user = resolve_user_from_session(&mut conn, &headers).await?;

    let (secret, otpauth_url) = generate_totp_secret(&user.username)?;
    conn.execute(
        "UPDATE users SET totp_secret = ?1, totp_enabled = FALSE WHERE id = ?2",
        (secret.as_str(), user.id),
    )
    .await?;

    Ok(Json(TotpSetupResponse {
        secret,
        otpauth_url,
    }))
}

pub async fn totp_enable(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<TotpVerifyRequest>,
) -> AppResult<impl IntoResponse> {
    let mut conn = state.db.connect()?;
    let user = resolve_user_from_session(&mut conn, &headers).await?;

    verify_totp(&user.totp_secret, &payload.code)?;

    conn.execute(
        "UPDATE users SET totp_enabled = TRUE WHERE id = ?1",
        [user.id],
    )
    .await?;

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct SetPathParams {
    date: String,
    exercise_id: i64,
    set_number: i32,
}

#[derive(Deserialize)]
pub struct DayExerciseParams {
    date: String,
    exercise_id: i64,
}

pub async fn upsert_set_for_day(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(params): Path<SetPathParams>,
    Json(payload): Json<SetForDayRequest>,
) -> AppResult<Json<SetForDayResponse>> {
    if params.set_number < 1 {
        return Err(AppError::BadRequest(
            "set_number must be at least 1".to_string(),
        ));
    }
    if payload.reps < 1 {
        return Err(AppError::BadRequest("reps must be at least 1".to_string()));
    }
    if payload.weight < 0.0 {
        return Err(AppError::BadRequest(
            "weight must be zero or greater".to_string(),
        ));
    }

    let day = NaiveDate::parse_from_str(&params.date, "%Y-%m-%d").map_err(|_| {
        AppError::BadRequest("invalid date format, expected YYYY-MM-DD".to_string())
    })?;

    let completed_at = payload.completed_at.unwrap_or_else(|| {
        let naive = day
            .and_hms_opt(12, 0, 0)
            .expect("valid midday for requested date");
        DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc)
    });
    let clamped_time = completed_at.time();
    let clamped_naive = day.and_time(clamped_time);
    let clamped_at = DateTime::<Utc>::from_naive_utc_and_offset(clamped_naive, Utc);

    let mut conn = state.db.connect()?;
    let user = resolve_user_from_session(&mut conn, &headers).await?;
    // Ensure the exercise belongs to the user.
    let _ = fetch_exercise_name(&conn, params.exercise_id, user.id).await?;

    conn.execute(
        "INSERT INTO workout_sets (exercise_id, day, set_number, weight_kg, reps, completed_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
         ON CONFLICT(exercise_id, day, set_number) DO UPDATE SET \
            weight_kg = excluded.weight_kg, reps = excluded.reps, completed_at = excluded.completed_at",
        (
            params.exercise_id,
            day.to_string(),
            params.set_number,
            payload.weight,
            payload.reps,
            serialize_timestamp(clamped_at),
        ),
    )
    .await?;

    let set_id = fetch_set_id(&mut conn, params.exercise_id, day, params.set_number).await?;

    Ok(Json(SetForDayResponse { set_id }))
}

pub async fn delete_set_for_day(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(params): Path<SetPathParams>,
) -> AppResult<impl IntoResponse> {
    if params.set_number < 1 {
        return Err(AppError::BadRequest(
            "set_number must be at least 1".to_string(),
        ));
    }
    let day = NaiveDate::parse_from_str(&params.date, "%Y-%m-%d").map_err(|_| {
        AppError::BadRequest("invalid date format, expected YYYY-MM-DD".to_string())
    })?;

    let mut conn = state.db.connect()?;
    let user = resolve_user_from_session(&mut conn, &headers).await?;

    let deleted = conn
        .execute(
            "DELETE FROM workout_sets \
             WHERE exercise_id = ?1 AND set_number = ?2 AND day = ?3 \
             AND exercise_id IN (SELECT id FROM exercises WHERE user_id = ?4)",
            (
                params.exercise_id,
                params.set_number,
                day.to_string(),
                user.id,
            ),
        )
        .await?;

    if deleted == 0 {
        return Err(AppError::NotFound("set not found".to_string()));
    }

    Ok(StatusCode::NO_CONTENT)
}

pub async fn get_sets_for_day_exercise(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(params): Path<DayExerciseParams>,
) -> AppResult<Json<DayExerciseSetsResponse>> {
    let day = NaiveDate::parse_from_str(&params.date, "%Y-%m-%d").map_err(|_| {
        AppError::BadRequest("invalid date format, expected YYYY-MM-DD".to_string())
    })?;

    let mut conn = state.db.connect()?;
    let user = resolve_user_from_session(&mut conn, &headers).await?;
    let response = fetch_sets_for_day_exercise(&mut conn, user.id, params.exercise_id, day).await?;
    Ok(Json(response))
}

pub async fn get_exercise_graph(
    State(state): State<AppState>,
    Path(exercise_id): Path<i64>,
    Query(request): Query<GraphRequest>,
    headers: HeaderMap,
) -> AppResult<Json<GraphResponse>> {
    let metric = request.metric.unwrap_or(MetricKind::MaxWeight);
    if let (Some(start), Some(end)) = (request.start, request.end)
        && start > end
    {
        return Err(AppError::BadRequest(
            "start must be before or equal to end".to_string(),
        ));
    }

    let mut conn = state.db.connect()?;
    let user = resolve_user_from_session(&mut conn, &headers).await?;
    let exercise_name = fetch_exercise_name(&conn, exercise_id, user.id).await?;
    let sets = fetch_exercise_sets(&conn, exercise_id, user.id, request.start, request.end).await?;

    let points = build_graph_points(sets, metric, MAX_GRAPH_POINTS);

    Ok(Json(GraphResponse {
        exercise_id,
        exercise_name,
        points,
    }))
}

pub async fn list_exercises(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<Json<Vec<Exercise>>> {
    let mut conn = state.db.connect()?;
    let user = resolve_user_from_session(&mut conn, &headers).await?;
    let mut rows = conn
        .query(
            "SELECT id, name, description, archived FROM exercises \
             WHERE user_id = ?1 AND archived = FALSE \
             ORDER BY name",
            [user.id],
        )
        .await?;

    let mut exercises = Vec::new();
    while let Some(row) = rows.next().await? {
        let id: i64 = row.get(0)?;
        let name: String = row.get(1)?;
        let description: Option<String> = row.get(2)?;
        let archived_value: i64 = row.get(3)?;
        exercises.push(Exercise {
            id,
            name,
            description,
            archived: archived_value != 0,
        });
    }

    Ok(Json(exercises))
}

pub async fn create_exercise(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateExerciseRequest>,
) -> AppResult<Json<Exercise>> {
    if payload.name.trim().is_empty() {
        return Err(AppError::BadRequest("name is required".to_string()));
    }

    let name = payload.name.trim().to_string();
    let description = payload.description;
    let mut conn = state.db.connect()?;
    let user = resolve_user_from_session(&mut conn, &headers).await?;
    conn.execute(
        "INSERT INTO exercises (user_id, name, description) VALUES (?1, ?2, ?3)",
        (user.id, name.as_str(), description.as_deref()),
    )
    .await?;

    let id = conn.last_insert_rowid();
    Ok(Json(Exercise {
        id,
        name,
        description,
        archived: false,
    }))
}

pub async fn update_exercise(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(exercise_id): Path<i64>,
    Json(payload): Json<UpdateExerciseRequest>,
) -> AppResult<impl IntoResponse> {
    if payload.name.is_none() && payload.description.is_none() && payload.archived.is_none() {
        return Err(AppError::BadRequest(
            "no fields provided for update".to_string(),
        ));
    }

    let mut sql = String::from("UPDATE exercises SET ");
    let mut params: Vec<Value> = Vec::new();
    let mut parts: Vec<&str> = Vec::new();

    if let Some(name) = payload.name {
        parts.push("name = ?");
        params.push(name.into());
    }
    if let Some(description) = payload.description {
        parts.push("description = ?");
        params.push(description.into());
    }
    if let Some(archived) = payload.archived {
        parts.push("archived = ?");
        let archived_val: i32 = if archived { 1 } else { 0 };
        params.push(archived_val.into());
    }

    sql.push_str(&parts.join(", "));
    sql.push_str(" WHERE id = ? AND user_id = ?");
    params.push(exercise_id.into());
    let mut conn = state.db.connect()?;
    let user = resolve_user_from_session(&mut conn, &headers).await?;
    params.push(user.id.into());

    let updated = conn.execute(&sql, params).await?;
    if updated == 0 {
        return Err(AppError::NotFound("exercise not found".to_string()));
    }

    Ok(StatusCode::NO_CONTENT)
}

pub async fn archive_exercise(
    State(state): State<AppState>,
    Path(exercise_id): Path<i64>,
    headers: HeaderMap,
) -> AppResult<impl IntoResponse> {
    let mut conn = state.db.connect()?;
    let user = resolve_user_from_session(&mut conn, &headers).await?;
    let updated = conn
        .execute(
            "UPDATE exercises SET archived = TRUE WHERE id = ?1 AND user_id = ?2",
            (exercise_id, user.id),
        )
        .await?;

    if updated == 0 {
        return Err(AppError::NotFound("exercise not found".to_string()));
    }

    Ok(StatusCode::NO_CONTENT)
}

async fn load_last_days(
    state: &AppState,
    user_id: i64,
    exercise_ids: &[i64],
) -> AppResult<HashMap<i64, (Option<DateTime<Utc>>, Vec<SetCompact>)>> {
    let conn = state.db.connect()?;
    let mut last_days = HashMap::new();

    for exercise_id in exercise_ids {
        let mut stmt = conn
            .prepare(
                "SELECT ws.day, MAX(ws.completed_at) as last_time \
                 FROM workout_sets ws \
                 JOIN exercises e ON e.id = ws.exercise_id \
                 WHERE ws.exercise_id = ?1 AND e.user_id = ?2 \
                 GROUP BY ws.day \
                 ORDER BY last_time DESC \
                 LIMIT 1",
            )
            .await?;

        match stmt.query_row((*exercise_id, user_id)).await {
            Ok(row) => {
                let day_raw: String = row.get(0)?;
                let last_time_raw: String = row.get(1)?;
                let day = NaiveDate::parse_from_str(&day_raw, "%Y-%m-%d").map_err(|err| {
                    AppError::Internal(format!("failed to parse day '{day_raw}': {err}"))
                })?;
                let last_time = parse_timestamp(&last_time_raw)?;
                let sets = load_sets_for_day(&conn, day, *exercise_id, user_id).await?;
                last_days.insert(*exercise_id, (Some(last_time), sets));
            }
            Err(turso::Error::QueryReturnedNoRows) => {
                last_days.insert(*exercise_id, (None, Vec::new()));
            }
            Err(err) => return Err(err.into()),
        }
    }

    Ok(last_days)
}

async fn load_sets_for_day(
    conn: &Connection,
    day: NaiveDate,
    exercise_id: i64,
    user_id: i64,
) -> AppResult<Vec<SetCompact>> {
    let mut rows = conn
        .query(
            "SELECT ws.weight_kg, ws.reps FROM workout_sets ws \
             JOIN exercises e ON e.id = ws.exercise_id \
             WHERE ws.exercise_id = ?1 AND e.user_id = ?2 AND ws.day = ?3 \
             ORDER BY ws.set_number",
            (exercise_id, user_id, day.to_string()),
        )
        .await?;

    let mut sets = Vec::new();
    while let Some(row) = rows.next().await? {
        let weight: f64 = row.get(0)?;
        let reps: i64 = row.get(1)?;
        sets.push(SetCompact {
            weight,
            reps: reps as i32,
        });
    }

    Ok(sets)
}

async fn fetch_set_counts(
    conn: &Connection,
    user_id: i64,
    start_date: NaiveDate,
    end_date: NaiveDate,
) -> AppResult<HashMap<NaiveDate, i64>> {
    let start_ts = DateTime::<Utc>::from_naive_utc_and_offset(
        start_date
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| AppError::Internal("invalid start date".to_string()))?,
        Utc,
    );
    let end_ts = DateTime::<Utc>::from_naive_utc_and_offset(
        end_date
            .and_hms_opt(23, 59, 59)
            .ok_or_else(|| AppError::Internal("invalid end date".to_string()))?,
        Utc,
    );

    let mut rows = conn
        .query(
            "SELECT DATE(ws.completed_at) as day, COUNT(*) \
             FROM workout_sets ws \
             JOIN exercises e ON e.id = ws.exercise_id \
             WHERE e.user_id = ?1 AND ws.completed_at >= ?2 AND ws.completed_at <= ?3 \
             GROUP BY day \
             ORDER BY day",
            (
                user_id,
                serialize_timestamp(start_ts),
                serialize_timestamp(end_ts),
            ),
        )
        .await?;

    let mut counts = HashMap::new();
    while let Some(row) = rows.next().await? {
        let day_raw: String = row.get(0)?;
        let count: i64 = row.get(1)?;
        let date = NaiveDate::parse_from_str(&day_raw, "%Y-%m-%d").map_err(|err| {
            AppError::Internal(format!("failed to parse date '{day_raw}': {err}"))
        })?;
        counts.insert(date, count);
    }

    Ok(counts)
}

async fn fetch_set_id(
    conn: &mut Connection,
    exercise_id: i64,
    day: NaiveDate,
    set_number: i32,
) -> AppResult<i64> {
    let mut stmt = conn
        .prepare(
            "SELECT id FROM workout_sets \
             WHERE exercise_id = ?1 AND day = ?2 AND set_number = ?3",
        )
        .await?;

    let row = stmt
        .query_row((exercise_id, day.to_string(), set_number))
        .await?;
    let set_id: i64 = row.get(0)?;
    Ok(set_id)
}

async fn fetch_sets_for_day_exercise(
    conn: &mut Connection,
    user_id: i64,
    exercise_id: i64,
    day: NaiveDate,
) -> AppResult<DayExerciseSetsResponse> {
    let day_str = day.format("%Y-%m-%d").to_string();
    let mut rows = conn
        .query(
            "SELECT ws.id, ws.set_number, ws.weight_kg, ws.reps, ws.completed_at \
             FROM workout_sets ws \
             JOIN exercises e ON e.id = ws.exercise_id \
             WHERE e.user_id = ?1 AND ws.exercise_id = ?2 AND ws.day = ?3 \
             ORDER BY ws.set_number",
            (user_id, exercise_id, day_str),
        )
        .await?;

    let mut sets = Vec::new();

    while let Some(row) = rows.next().await? {
        let set_id: i64 = row.get(0)?;
        let set_number: i64 = row.get(1)?;
        let weight: f64 = row.get(2)?;
        let reps: i64 = row.get(3)?;
        let completed_raw: String = row.get(4)?;
        let completed_at = parse_timestamp(&completed_raw)?;

        sets.push(SetForDayItem {
            set_id,
            set_number: set_number as i32,
            weight,
            reps: reps as i32,
            completed_at,
        });
    }

    Ok(DayExerciseSetsResponse { sets })
}

#[derive(Debug, Clone)]
struct AuthUser {
    id: i64,
    username: String,
    password_hash: String,
    password_salt: String,
    totp_secret: String,
    totp_enabled: bool,
}

fn generate_token() -> AppResult<String> {
    let mut bytes = [0_u8; 32];
    let mut rng = OsRng;
    rng.fill_bytes(&mut bytes);
    let mut token = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(token, "{b:02x}");
    }
    Ok(token)
}

fn verify_password(input: &str, stored_hash: &str, _salt: &str) -> AppResult<()> {
    let parsed_hash = PasswordHash::new(stored_hash)
        .map_err(|err| AppError::Internal(format!("invalid password hash: {err}")))?;
    Argon2::default()
        .verify_password(input.as_bytes(), &parsed_hash)
        .map_err(|_| AppError::Unauthorized)
}

fn hash_password(input: &str) -> AppResult<(String, String)> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(input.as_bytes(), &salt)
        .map_err(|err| AppError::Internal(format!("failed to hash password: {err}")))?
        .to_string();
    Ok((hash, salt.to_string()))
}

fn build_session_cookie(token: &str, expires_at: DateTime<Utc>) -> HeaderValue {
    let max_age = expires_at
        .signed_duration_since(now_utc())
        .num_seconds()
        .max(0);
    let value =
        format!("{SESSION_COOKIE}={token}; HttpOnly; SameSite=Lax; Path=/; Max-Age={max_age}");
    HeaderValue::from_str(&value).unwrap_or_else(|_| HeaderValue::from_static(""))
}

fn clear_session_cookie() -> HeaderValue {
    let value = format!("{SESSION_COOKIE}=; Max-Age=0; Path=/; HttpOnly; SameSite=Lax");
    HeaderValue::from_str(&value).unwrap_or_else(|_| HeaderValue::from_static(""))
}

fn extract_session_token(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    for part in raw.split(';') {
        let mut iter = part.trim().splitn(2, '=');
        if let (Some(name), Some(value)) = (iter.next(), iter.next())
            && name == SESSION_COOKIE
        {
            return Some(value.to_string());
        }
    }
    None
}

async fn create_auth_session(
    conn: &mut Connection,
    user_id: i64,
    token: &str,
    expires_at: DateTime<Utc>,
) -> AppResult<()> {
    let now = serialize_timestamp(now_utc());
    conn.execute(
        "INSERT INTO auth_sessions (user_id, token, expires_at, last_used_at) VALUES (?1, ?2, ?3, ?4)",
        (user_id, token, serialize_timestamp(expires_at), now),
    )
    .await?;
    Ok(())
}

async fn delete_auth_session(conn: &mut Connection, token: String) -> AppResult<()> {
    conn.execute("DELETE FROM auth_sessions WHERE token = ?", [token])
        .await?;
    Ok(())
}

async fn fetch_auth_session_user(
    conn: &mut Connection,
    token: &str,
) -> AppResult<Option<(i64, DateTime<Utc>)>> {
    let mut stmt = conn
        .prepare("SELECT user_id, expires_at FROM auth_sessions WHERE token = ?1")
        .await?;
    match stmt.query_row([token]).await {
        Ok(row) => {
            let user_id: i64 = row.get(0)?;
            let expires_raw: String = row.get(1)?;
            let expires_at = parse_timestamp(&expires_raw)?;
            Ok(Some((user_id, expires_at)))
        }
        Err(turso::Error::QueryReturnedNoRows) => Ok(None),
        Err(err) => Err(err.into()),
    }
}

async fn fetch_user(conn: &mut Connection, username: &str) -> AppResult<AuthUser> {
    let mut stmt = conn
        .prepare(
            "SELECT id, username, password_hash, password_salt, totp_secret, totp_enabled \
             FROM users WHERE username = ?1",
        )
        .await?;

    let row = stmt.query_row([username]).await.map_err(|err| match err {
        turso::Error::QueryReturnedNoRows => AppError::Unauthorized,
        other => other.into(),
    })?;

    Ok(AuthUser {
        id: row.get(0)?,
        username: row.get(1)?,
        password_hash: row.get(2)?,
        password_salt: row.get(3)?,
        totp_secret: row.get(4)?,
        totp_enabled: row.get::<bool>(5)?,
    })
}

async fn fetch_user_by_id(conn: &mut Connection, user_id: i64) -> AppResult<AuthUser> {
    let mut stmt = conn
        .prepare(
            "SELECT id, username, password_hash, password_salt, totp_secret, totp_enabled \
             FROM users WHERE id = ?1",
        )
        .await?;
    let row = stmt.query_row([user_id]).await?;
    Ok(AuthUser {
        id: row.get(0)?,
        username: row.get(1)?,
        password_hash: row.get(2)?,
        password_salt: row.get(3)?,
        totp_secret: row.get(4)?,
        totp_enabled: row.get::<bool>(5)?,
    })
}

async fn resolve_user_from_session(
    conn: &mut Connection,
    headers: &HeaderMap,
) -> AppResult<AuthUser> {
    let token = extract_session_token(headers).ok_or(AppError::Unauthorized)?;
    let Some((user_id, expires_at)) = fetch_auth_session_user(conn, &token).await? else {
        return Err(AppError::Unauthorized);
    };

    if expires_at < now_utc() {
        delete_auth_session(conn, token).await?;
        return Err(AppError::Unauthorized);
    }

    fetch_user_by_id(conn, user_id)
        .await
        .map_err(|err| match err {
            AppError::NotFound(_) => AppError::Unauthorized,
            other => other,
        })
}

fn verify_totp(secret_b32: &str, code: &str) -> AppResult<()> {
    let bytes = Secret::Encoded(secret_b32.to_string())
        .to_bytes()
        .map_err(|err| AppError::Internal(format!("invalid TOTP secret: {err}")))?;
    let totp = TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        bytes,
        Some("ekman".to_string()),
        "ekman".to_string(),
    )
    .map_err(|err| AppError::Internal(format!("failed to build TOTP: {err}")))?;
    let valid = totp
        .check_current(code)
        .map_err(|err| AppError::Internal(format!("failed to verify TOTP: {err}")))?;
    if valid {
        Ok(())
    } else {
        Err(AppError::Unauthorized)
    }
}

fn enforce_totp(user: &AuthUser, code: Option<&str>) -> AppResult<()> {
    if !user.totp_enabled {
        return Err(AppError::Unauthorized);
    }

    let Some(code) = code else {
        return Err(AppError::Unauthorized);
    };

    verify_totp(&user.totp_secret, code)
}

fn is_unique_violation(err: &turso::Error) -> bool {
    err.to_string()
        .contains("UNIQUE constraint failed: users.username")
}

fn generate_totp_secret(username: &str) -> AppResult<(String, String)> {
    let mut bytes = [0_u8; 20];
    let mut rng = OsRng;
    rng.fill_bytes(&mut bytes);
    let secret_b32 = base32_encode(Alphabet::Rfc4648 { padding: false }, &bytes);

    let totp = TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        bytes.to_vec(),
        Some("ekman".to_string()),
        username.to_string(),
    )
    .map_err(|err| AppError::Internal(format!("failed to build TOTP: {err}")))?;
    let otpauth_url = totp.get_url();

    Ok((secret_b32, otpauth_url))
}

async fn fetch_exercise_name(
    conn: &Connection,
    exercise_id: i64,
    user_id: i64,
) -> AppResult<String> {
    let mut stmt = conn
        .prepare("SELECT name FROM exercises WHERE id = ?1 AND user_id = ?2")
        .await?;
    let row = stmt
        .query_row((exercise_id, user_id))
        .await
        .map_err(|err| match err {
            turso::Error::QueryReturnedNoRows => {
                AppError::NotFound("exercise not found".to_string())
            }
            other => other.into(),
        })?;

    let name: String = row.get(0)?;
    Ok(name)
}

async fn fetch_exercise_sets(
    conn: &Connection,
    exercise_id: i64,
    user_id: i64,
    start: Option<DateTime<Utc>>,
    end: Option<DateTime<Utc>>,
) -> AppResult<Vec<SetDataPoint>> {
    let mut sql = String::from(
        "SELECT ws.day, ws.weight_kg, ws.reps, ws.completed_at \
         FROM workout_sets ws \
         JOIN exercises e ON e.id = ws.exercise_id \
         WHERE ws.exercise_id = ?1 AND e.user_id = ?2",
    );

    let mut params: Vec<Value> = vec![exercise_id.into(), user_id.into()];
    if let Some(start) = start {
        sql.push_str(&format!(" AND ws.completed_at >= ?{}", params.len() + 1));
        params.push(serialize_timestamp(start).into());
    }
    if let Some(end) = end {
        sql.push_str(&format!(" AND ws.completed_at <= ?{}", params.len() + 1));
        params.push(serialize_timestamp(end).into());
    }
    sql.push_str(" ORDER BY ws.completed_at ASC");

    let mut rows = conn.query(&sql, params).await?;
    let mut sets = Vec::new();
    while let Some(row) = rows.next().await? {
        let day_raw: String = row.get(0)?;
        let weight: f64 = row.get(1)?;
        let reps: i64 = row.get(2)?;
        let completed_raw: String = row.get(3)?;
        let day = NaiveDate::parse_from_str(&day_raw, "%Y-%m-%d")
            .map_err(|err| AppError::Internal(format!("failed to parse day '{day_raw}': {err}")))?;
        let completed_at = parse_timestamp(&completed_raw)?;
        let date = completed_at.date_naive().max(day); // ensure consistency with stored day

        sets.push(SetDataPoint {
            date,
            weight,
            reps: reps as i32,
        });
    }

    Ok(sets)
}
