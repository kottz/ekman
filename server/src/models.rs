use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub struct PopulatedTemplate {
    pub id: i64,
    pub name: String,
    pub day_of_week: Option<i32>,
    pub exercises: Vec<PopulatedExercise>,
}

#[derive(Serialize)]
pub struct PopulatedExercise {
    pub exercise_id: i64,
    pub name: String,
    pub target_sets: Option<i32>,
    pub last_session_date: Option<NaiveDateTime>,
    pub last_session_sets: Vec<SetCompact>,
}

#[derive(Serialize, Clone)]
pub struct SetCompact {
    pub weight: f64,
    pub reps: i32,
}

#[derive(Deserialize)]
pub struct GraphRequest {
    pub period: String,
    pub metric: Option<String>,
}

#[derive(Serialize, Clone)]
pub struct GraphPoint {
    pub date: String,
    pub value: f64,
}

#[derive(Serialize)]
pub struct GraphResponse {
    pub exercise_id: i64,
    pub exercise_name: String,
    pub points: Vec<GraphPoint>,
}

#[derive(Deserialize)]
pub struct LogSetRequest {
    pub exercise_id: i64,
    pub weight: f64,
    pub reps: i32,
    pub notes: Option<String>,
    pub completed_at: Option<NaiveDateTime>,
}

#[derive(Deserialize)]
pub struct CreateSessionRequest {
    pub notes: Option<String>,
    pub sets: Vec<LogSetRequest>,
}

#[derive(Serialize)]
pub struct CreateSessionResponse {
    pub session_id: i64,
}

#[derive(Deserialize)]
pub struct UpdateSetRequest {
    pub weight: Option<f64>,
    pub reps: Option<i32>,
    pub notes: Option<String>,
    pub completed_at: Option<NaiveDateTime>,
}

#[derive(Deserialize)]
pub struct CreateExerciseRequest {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Deserialize)]
pub struct UpdateExerciseRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub archived: Option<bool>,
}

#[derive(Serialize)]
pub struct ExerciseResponse {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub archived: bool,
}
