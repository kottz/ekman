use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

// Exercise models ------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Exercise {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub archived: bool,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct CreateExerciseRequest {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct UpdateExerciseRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub archived: Option<bool>,
}

// Plan & template models ----------------------------------------------------

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PopulatedTemplate {
    pub id: i64,
    pub name: String,
    pub day_of_week: Option<i32>,
    pub exercises: Vec<PopulatedExercise>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PopulatedExercise {
    pub exercise_id: i64,
    pub name: String,
    pub target_sets: Option<i32>,
    pub last_session_date: Option<NaiveDateTime>,
    pub last_session_sets: Vec<SetCompact>,
}

// Session & set models ------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct SetCompact {
    pub weight: f64,
    pub reps: i32,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct CreateSessionRequest {
    pub notes: Option<String>,
    pub sets: Vec<LogSetRequest>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CreateSessionResponse {
    pub session_id: i64,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LogSetRequest {
    pub exercise_id: i64,
    pub weight: f64,
    pub reps: i32,
    pub notes: Option<String>,
    pub completed_at: Option<NaiveDateTime>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct UpdateSetRequest {
    pub weight: Option<f64>,
    pub reps: Option<i32>,
    pub notes: Option<String>,
    pub completed_at: Option<NaiveDateTime>,
}

// Graph & analytics models --------------------------------------------------

#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MetricKind {
    MaxWeight,
    SessionTotalVolume,
    BestSetVolume,
    Est1Rm,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GraphRequest {
    pub period: String,
    pub metric: Option<MetricKind>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GraphPoint {
    pub date: String,
    pub value: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GraphResponse {
    pub exercise_id: i64,
    pub exercise_name: String,
    pub points: Vec<GraphPoint>,
}
