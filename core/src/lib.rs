//! Core types and logic shared between server and TUI.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

// ============================================================================
// Exercises
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Exercise {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub archived: bool,
    pub owner: Owner,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Owner {
    User,
    Admin,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateExercise {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateExercise {
    pub name: Option<String>,
    pub description: Option<String>,
    pub archived: Option<bool>,
}

// ============================================================================
// Workout sets
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkoutSet {
    pub id: i64,
    pub exercise_id: i64,
    pub day: String,
    pub set_number: i32,
    pub weight: f64,
    pub reps: i32,
    pub completed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetInput {
    pub weight: f64,
    pub reps: i32,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaySets {
    pub sets: Vec<WorkoutSet>,
}

// ============================================================================
// Plans & templates
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Template {
    pub id: i64,
    pub name: String,
    pub day_of_week: Option<i32>,
    pub exercises: Vec<TemplateExercise>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateExercise {
    pub exercise_id: i64,
    pub name: String,
    pub target_sets: Option<i32>,
    pub last_session: Option<LastSession>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LastSession {
    pub date: DateTime<Utc>,
    pub sets: Vec<CompactSet>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompactSet {
    pub weight: f64,
    pub reps: i32,
}

// ============================================================================
// Activity
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityQuery {
    pub start: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Activity {
    pub days: Vec<ActivityDay>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityDay {
    pub date: String,
    pub sets_completed: i64,
}

// ============================================================================
// Graphs
// ============================================================================

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Metric {
    MaxWeight,
    SessionTotalVolume,
    BestSetVolume,
    #[serde(rename = "est_1rm")]
    Est1Rm,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphQuery {
    pub start: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
    pub metric: Option<Metric>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Graph {
    pub exercise_id: i64,
    pub exercise_name: String,
    pub points: Vec<GraphPoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphPoint {
    pub date: String,
    pub value: f64,
}

// ============================================================================
// Auth
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterInput {
    pub username: String,
    pub password: String,
    pub totp_secret: String,
    pub totp_code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginInput {
    pub username: String,
    pub password: String,
    pub totp: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub user_id: i64,
    pub username: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub user_id: i64,
    pub username: String,
    pub totp_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TotpSetup {
    pub secret: String,
    pub otpauth_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TotpVerify {
    pub code: String,
}

// ============================================================================
// Graph computation logic
// ============================================================================

/// Data point for graph computation.
pub struct SetData {
    pub date: NaiveDate,
    pub weight: f64,
    pub reps: i32,
}

/// Estimates 1RM using Epley formula.
pub fn estimate_1rm(weight: f64, reps: i32) -> f64 {
    if reps <= 1 {
        weight
    } else {
        weight * (1.0 + reps as f64 / 30.0)
    }
}

/// Computes metric for a day's sets.
pub fn day_metric(metric: Metric, sets: &[SetData]) -> f64 {
    match metric {
        Metric::MaxWeight => sets.iter().map(|s| s.weight).fold(0.0, f64::max),
        Metric::SessionTotalVolume => sets.iter().map(|s| s.weight * s.reps as f64).sum(),
        Metric::BestSetVolume => sets
            .iter()
            .map(|s| s.weight * s.reps as f64)
            .fold(0.0, f64::max),
        Metric::Est1Rm => sets
            .iter()
            .map(|s| estimate_1rm(s.weight, s.reps))
            .fold(0.0, f64::max),
    }
}

/// Builds graph points from set data, downsampling if needed.
pub fn build_graph(sets: Vec<SetData>, metric: Metric, max_points: usize) -> Vec<GraphPoint> {
    use std::collections::HashMap;

    if sets.is_empty() || max_points == 0 {
        return Vec::new();
    }

    // Group by date
    let mut by_date: HashMap<NaiveDate, Vec<SetData>> = HashMap::new();
    for set in sets {
        by_date.entry(set.date).or_default().push(set);
    }

    // Compute daily values
    let mut points: Vec<_> = by_date
        .into_iter()
        .map(|(date, sets)| (date, day_metric(metric, &sets)))
        .collect();
    points.sort_by_key(|(d, _)| *d);

    // Downsample if needed
    if points.len() <= max_points {
        return points
            .into_iter()
            .map(|(d, v)| GraphPoint {
                date: d.format("%Y-%m-%d").to_string(),
                value: v,
            })
            .collect();
    }

    let bucket_size = points.len().div_ceil(max_points);
    points
        .chunks(bucket_size)
        .map(|chunk| {
            let date = chunk[0].0.format("%Y-%m-%d").to_string();
            let value = match metric {
                Metric::MaxWeight | Metric::Est1Rm => {
                    chunk.iter().map(|(_, v)| *v).fold(0.0, f64::max)
                }
                Metric::SessionTotalVolume | Metric::BestSetVolume => {
                    chunk.iter().map(|(_, v)| *v).sum()
                }
            };
            GraphPoint { date, value }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_estimate_1rm() {
        assert_eq!(estimate_1rm(100.0, 1), 100.0);
        assert_eq!(estimate_1rm(100.0, 0), 100.0);
        assert!((estimate_1rm(100.0, 5) - 116.666).abs() < 0.01);
    }

    #[test]
    fn test_day_metric() {
        let sets = vec![
            SetData {
                date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                weight: 100.0,
                reps: 5,
            },
            SetData {
                date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                weight: 105.0,
                reps: 3,
            },
        ];
        assert_eq!(day_metric(Metric::MaxWeight, &sets), 105.0);
        assert_eq!(day_metric(Metric::SessionTotalVolume, &sets), 815.0);
        assert_eq!(day_metric(Metric::BestSetVolume, &sets), 500.0);
    }

    #[test]
    fn test_build_graph_downsample() {
        let sets: Vec<_> = (1..=6)
            .map(|d| SetData {
                date: NaiveDate::from_ymd_opt(2024, 1, d).unwrap(),
                weight: 50.0 + d as f64,
                reps: 5,
            })
            .collect();

        let points = build_graph(sets, Metric::MaxWeight, 2);
        assert_eq!(points.len(), 2);
    }

    #[test]
    fn test_metric_serde() {
        assert_eq!(
            serde_json::to_string(&Metric::Est1Rm).unwrap(),
            "\"est_1rm\""
        );
        assert_eq!(
            serde_json::to_string(&Metric::MaxWeight).unwrap(),
            "\"max_weight\""
        );
    }
}
