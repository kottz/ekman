use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// Exercise models ------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Exercise {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub archived: bool,
    pub owner: ExerciseOwner,
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
    pub last_day_date: Option<DateTime<Utc>>,
    pub last_day_sets: Vec<SetCompact>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExerciseOwner {
    User,
    Admin,
}

// Set models ------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct SetCompact {
    pub weight: f64,
    pub reps: i32,
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
    pub start: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
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

// Activity & streak models --------------------------------------------------

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ActivityRequest {
    pub start: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ActivityDay {
    pub date: String,
    pub sets_completed: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ActivityResponse {
    pub days: Vec<ActivityDay>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SetForDayRequest {
    pub weight: f64,
    pub reps: i32,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SetForDayResponse {
    pub set_id: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SetForDayItem {
    pub set_id: i64,
    pub set_number: i32,
    pub weight: f64,
    pub reps: i32,
    pub completed_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DayExerciseSetsResponse {
    pub sets: Vec<SetForDayItem>,
}

// Auth -----------------------------------------------------------

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct RegisterRequest {
    pub username: String,
    pub password: String,
    pub totp_secret: String,
    pub totp_code: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
    pub totp: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LoginResponse {
    pub user_id: i64,
    pub username: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MeResponse {
    pub user_id: i64,
    pub username: String,
    pub totp_enabled: bool,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct TotpSetupResponse {
    pub secret: String,
    pub otpauth_url: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct TotpVerifyRequest {
    pub code: String,
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
    fn activity_request_query_round_trip() {
        let start = DateTime::parse_from_rfc3339("2024-03-10T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let end = DateTime::parse_from_rfc3339("2024-03-20T23:59:59Z")
            .unwrap()
            .with_timezone(&Utc);
        let encoded = serde_urlencoded::to_string(ActivityRequest {
            start: Some(start),
            end: Some(end),
        })
        .unwrap();

        let decoded: ActivityRequest = serde_urlencoded::from_str(&encoded).unwrap();
        assert_eq!(decoded.start, Some(start));
        assert_eq!(decoded.end, Some(end));
    }

    #[test]
    fn graph_request_query_serializes_metric_snake_case() {
        let start = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let end = DateTime::parse_from_rfc3339("2024-02-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let encoded = serde_urlencoded::to_string(GraphRequest {
            start: Some(start),
            end: Some(end),
            metric: Some(MetricKind::Est1Rm),
        })
        .unwrap();

        assert!(
            encoded.contains("metric=est_1rm"),
            "expected metric to be serialized as snake_case, got {encoded}"
        );
        assert!(
            encoded.contains("start=2024-01-01T00%3A00%3A00Z"),
            "expected start to serialize as RFC3339, got {encoded}"
        );
        assert!(encoded.contains("end=2024-02-01T00%3A00%3A00Z"));
    }

    #[test]
    fn graph_request_start_end_round_trip() {
        let start = DateTime::parse_from_rfc3339("2025-11-26T14:32:10Z")
            .unwrap()
            .with_timezone(&Utc);
        let end = DateTime::parse_from_rfc3339("2025-12-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let encoded = serde_urlencoded::to_string(GraphRequest {
            start: Some(start),
            end: Some(end),
            metric: None,
        })
        .unwrap();
        let decoded: GraphRequest = serde_urlencoded::from_str(&encoded).unwrap();
        assert_eq!(decoded.start, Some(start));
        assert_eq!(decoded.end, Some(end));
        assert!(decoded.metric.is_none());
    }
}
