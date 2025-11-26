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
    #[serde(rename = "est_1rm")]
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    #[test]
    fn metric_kind_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&MetricKind::MaxWeight).unwrap(),
            "\"max_weight\""
        );
        assert_eq!(
            serde_json::to_string(&MetricKind::SessionTotalVolume).unwrap(),
            "\"session_total_volume\""
        );
        assert_eq!(
            serde_json::to_string(&MetricKind::BestSetVolume).unwrap(),
            "\"best_set_volume\""
        );
        assert_eq!(
            serde_json::to_string(&MetricKind::Est1Rm).unwrap(),
            "\"est_1rm\""
        );
    }

    #[test]
    fn graph_request_query_serializes_metric_snake_case() {
        let encoded = serde_urlencoded::to_string(GraphRequest {
            period: "1m".to_string(),
            metric: Some(MetricKind::Est1Rm),
        })
        .unwrap();

        assert!(
            encoded.contains("metric=est_1rm"),
            "expected metric to be serialized as snake_case, got {encoded}"
        );
    }
}
