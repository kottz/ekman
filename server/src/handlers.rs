use std::collections::{BTreeMap, HashMap, HashSet};

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use chrono::{DateTime, Utc};
use turso::{Connection, Value};

use crate::{
    AppState,
    db::{now_utc, parse_timestamp, serialize_timestamp},
    error::{AppError, AppResult},
};
use ekman_core::{
    logic::{SetDataPoint, build_graph_points},
    models::{
        CreateExerciseRequest, CreateSessionRequest, CreateSessionResponse, Exercise, GraphRequest,
        GraphResponse, MetricKind, PopulatedExercise, PopulatedTemplate, SetCompact,
        UpdateExerciseRequest, UpdateSetRequest,
    },
};

const MAX_GRAPH_POINTS: usize = 50;

pub async fn get_daily_plans(
    State(state): State<AppState>,
) -> AppResult<Json<Vec<PopulatedTemplate>>> {
    let conn = state.db.connect()?;
    let mut rows = conn
        .query(
            "SELECT wt.id, wt.name, wt.day_of_week, te.exercise_id, \
             te.target_sets, e.name \
             FROM workout_templates wt \
             LEFT JOIN template_exercises te ON te.template_id = wt.id \
             LEFT JOIN exercises e ON e.id = te.exercise_id \
             WHERE wt.user_id = ?1 \
             ORDER BY wt.id, te.display_order",
            [state.default_user_id],
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
                last_session_date: None,
                last_session_sets: Vec::new(),
            });
        }
    }

    let last_sessions =
        load_last_sessions(&state, &exercise_ids.into_iter().collect::<Vec<_>>()).await?;

    let mut templates_vec: Vec<PopulatedTemplate> = templates.into_values().collect();
    for template in templates_vec.iter_mut() {
        for exercise in template.exercises.iter_mut() {
            if let Some((date, sets)) = last_sessions.get(&exercise.exercise_id) {
                exercise.last_session_date = date.map(|dt| dt.with_timezone(&Utc));
                exercise.last_session_sets = sets.clone();
            }
        }
    }

    Ok(Json(templates_vec))
}

pub async fn get_exercise_graph(
    State(state): State<AppState>,
    Path(exercise_id): Path<i64>,
    Query(request): Query<GraphRequest>,
) -> AppResult<Json<GraphResponse>> {
    let metric = request.metric.unwrap_or(MetricKind::MaxWeight);
    if let (Some(start), Some(end)) = (request.start, request.end) && start > end {
        return Err(AppError::BadRequest(
            "start must be before or equal to end".to_string(),
        ));
    }

    let conn = state.db.connect()?;
    let exercise_name = fetch_exercise_name(&conn, exercise_id, state.default_user_id).await?;
    let sets = fetch_exercise_sets(
        &conn,
        exercise_id,
        state.default_user_id,
        request.start,
        request.end,
    )
    .await?;

    let points = build_graph_points(sets, metric, MAX_GRAPH_POINTS);

    Ok(Json(GraphResponse {
        exercise_id,
        exercise_name,
        points,
    }))
}

pub async fn create_session(
    State(state): State<AppState>,
    Json(payload): Json<CreateSessionRequest>,
) -> AppResult<Json<CreateSessionResponse>> {
    if payload.sets.is_empty() {
        return Err(AppError::BadRequest(
            "session must include at least one set".to_string(),
        ));
    }

    let mut conn = state.db.connect()?;
    let tx = conn.transaction().await?;

    tx.execute(
        "INSERT INTO sessions (user_id, notes) VALUES (?1, ?2)",
        (state.default_user_id, payload.notes.clone()),
    )
    .await?;
    let session_id = tx.last_insert_rowid();

    let mut next_set_number: HashMap<i64, i32> = HashMap::new();
    for set in payload.sets {
        let counter = next_set_number.entry(set.exercise_id).or_insert(0);
        *counter += 1;

        let completed_at = set.completed_at.unwrap_or_else(now_utc);
        tx.execute(
            "INSERT INTO workout_sets (session_id, exercise_id, set_number, weight_kg, reps, \
             notes, completed_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            (
                session_id,
                set.exercise_id,
                *counter,
                set.weight,
                set.reps,
                set.notes.as_deref(),
                serialize_timestamp(completed_at),
            ),
        )
        .await?;
    }

    tx.commit().await?;

    Ok(Json(CreateSessionResponse { session_id }))
}

pub async fn update_set(
    State(state): State<AppState>,
    Path(set_id): Path<i64>,
    Json(payload): Json<UpdateSetRequest>,
) -> AppResult<impl IntoResponse> {
    if payload.weight.is_none()
        && payload.reps.is_none()
        && payload.notes.is_none()
        && payload.completed_at.is_none()
    {
        return Err(AppError::BadRequest(
            "no fields provided for update".to_string(),
        ));
    }

    let mut sql = String::from("UPDATE workout_sets SET ");
    let mut params: Vec<Value> = Vec::new();
    let mut parts: Vec<&str> = Vec::new();

    if let Some(weight) = payload.weight {
        parts.push("weight_kg = ?");
        params.push(weight.into());
    }
    if let Some(reps) = payload.reps {
        parts.push("reps = ?");
        params.push(reps.into());
    }
    if let Some(notes) = payload.notes {
        parts.push("notes = ?");
        params.push(notes.into());
    }
    if let Some(completed_at) = payload.completed_at {
        parts.push("completed_at = ?");
        params.push(serialize_timestamp(completed_at).into());
    }

    sql.push_str(&parts.join(", "));
    sql.push_str(" WHERE id = ?");
    params.push(set_id.into());

    let conn = state.db.connect()?;
    let updated = conn.execute(&sql, params).await?;
    if updated == 0 {
        return Err(AppError::NotFound("set not found".to_string()));
    }

    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_set(
    State(state): State<AppState>,
    Path(set_id): Path<i64>,
) -> AppResult<impl IntoResponse> {
    let conn = state.db.connect()?;
    let deleted = conn
        .execute("DELETE FROM workout_sets WHERE id = ?", [set_id])
        .await?;

    if deleted == 0 {
        return Err(AppError::NotFound("set not found".to_string()));
    }

    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_exercises(State(state): State<AppState>) -> AppResult<Json<Vec<Exercise>>> {
    let conn = state.db.connect()?;
    let mut rows = conn
        .query(
            "SELECT id, name, description, archived FROM exercises \
             WHERE user_id = ?1 AND archived = FALSE \
             ORDER BY name",
            [state.default_user_id],
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
    Json(payload): Json<CreateExerciseRequest>,
) -> AppResult<Json<Exercise>> {
    if payload.name.trim().is_empty() {
        return Err(AppError::BadRequest("name is required".to_string()));
    }

    let name = payload.name.trim().to_string();
    let description = payload.description;
    let conn = state.db.connect()?;
    conn.execute(
        "INSERT INTO exercises (user_id, name, description) VALUES (?1, ?2, ?3)",
        (state.default_user_id, name.as_str(), description.as_deref()),
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
    params.push(state.default_user_id.into());

    let conn = state.db.connect()?;
    let updated = conn.execute(&sql, params).await?;
    if updated == 0 {
        return Err(AppError::NotFound("exercise not found".to_string()));
    }

    Ok(StatusCode::NO_CONTENT)
}

pub async fn archive_exercise(
    State(state): State<AppState>,
    Path(exercise_id): Path<i64>,
) -> AppResult<impl IntoResponse> {
    let conn = state.db.connect()?;
    let updated = conn
        .execute(
            "UPDATE exercises SET archived = TRUE WHERE id = ?1 AND user_id = ?2",
            (exercise_id, state.default_user_id),
        )
        .await?;

    if updated == 0 {
        return Err(AppError::NotFound("exercise not found".to_string()));
    }

    Ok(StatusCode::NO_CONTENT)
}

async fn load_last_sessions(
    state: &AppState,
    exercise_ids: &[i64],
) -> AppResult<HashMap<i64, (Option<DateTime<Utc>>, Vec<SetCompact>)>> {
    let conn = state.db.connect()?;
    let mut last_sessions = HashMap::new();

    for exercise_id in exercise_ids {
        let mut stmt = conn
            .prepare(
                "SELECT ws.session_id, s.started_at, MAX(ws.completed_at) as last_time \
                 FROM workout_sets ws \
                 JOIN sessions s ON s.id = ws.session_id \
                 WHERE ws.exercise_id = ?1 AND s.user_id = ?2 \
                 GROUP BY ws.session_id \
                 ORDER BY last_time DESC \
                 LIMIT 1",
            )
            .await?;

        match stmt.query_row((*exercise_id, state.default_user_id)).await {
            Ok(row) => {
                let session_id: i64 = row.get(0)?;
                let started_at_raw: String = row.get(1)?;
                let started_at = parse_timestamp(&started_at_raw)?;
                let sets = load_sets_for_session(&conn, session_id, *exercise_id).await?;
                last_sessions.insert(*exercise_id, (Some(started_at), sets));
            }
            Err(turso::Error::QueryReturnedNoRows) => {
                last_sessions.insert(*exercise_id, (None, Vec::new()));
            }
            Err(err) => return Err(err.into()),
        }
    }

    Ok(last_sessions)
}

async fn load_sets_for_session(
    conn: &Connection,
    session_id: i64,
    exercise_id: i64,
) -> AppResult<Vec<SetCompact>> {
    let mut rows = conn
        .query(
            "SELECT weight_kg, reps FROM workout_sets \
             WHERE session_id = ?1 AND exercise_id = ?2 \
             ORDER BY set_number",
            (session_id, exercise_id),
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
        "SELECT ws.session_id, s.started_at, ws.weight_kg, ws.reps \
         FROM workout_sets ws \
         JOIN sessions s ON s.id = ws.session_id \
         WHERE ws.exercise_id = ?1 AND s.user_id = ?2",
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
        let session_id: i64 = row.get(0)?;
        let started_raw: String = row.get(1)?;
        let weight: f64 = row.get(2)?;
        let reps: i64 = row.get(3)?;
        let started_at = parse_timestamp(&started_raw)?;

        sets.push(SetDataPoint {
            session_id,
            date: started_at.date_naive(),
            weight,
            reps: reps as i32,
        });
    }

    Ok(sets)
}
