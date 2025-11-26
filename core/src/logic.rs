use std::collections::HashMap;

use chrono::{NaiveDate, Utc};

use crate::models::{GraphPoint, MetricKind};

#[derive(Debug, Clone)]
pub struct SetDataPoint {
    pub session_id: i64,
    pub date: NaiveDate,
    pub weight: f64,
    pub reps: i32,
}

/// Estimates 1RM using the Epley formula.
pub fn estimate_one_rm(weight: f64, reps: i32) -> f64 {
    if reps <= 1 {
        return weight;
    }
    weight * (1.0 + reps as f64 / 30.0)
}

/// Computes the specific metric for a list of sets (usually belonging to one session).
pub fn compute_session_metric(metric: MetricKind, sets: &[SetDataPoint]) -> f64 {
    match metric {
        MetricKind::MaxWeight => sets.iter().fold(0.0_f64, |acc, set| acc.max(set.weight)),
        MetricKind::SessionTotalVolume => sets.iter().map(|set| set.weight * set.reps as f64).sum(),
        MetricKind::BestSetVolume => sets
            .iter()
            .fold(0.0_f64, |acc, set| acc.max(set.weight * set.reps as f64)),
        MetricKind::Est1Rm => sets.iter().fold(0.0_f64, |acc, set| {
            acc.max(estimate_one_rm(set.weight, set.reps))
        }),
    }
}

pub fn build_graph_points(
    sets: Vec<SetDataPoint>,
    metric: MetricKind,
    max_points: usize,
) -> Vec<GraphPoint> {
    if sets.is_empty() || max_points == 0 {
        return Vec::new();
    }

    let mut grouped: HashMap<i64, Vec<SetDataPoint>> = HashMap::new();
    for set in sets {
        grouped.entry(set.session_id).or_default().push(set);
    }

    let mut points: Vec<(NaiveDate, f64)> = grouped
        .into_values()
        .map(|session_sets| {
            let date = session_sets
                .first()
                .map(|s| s.date)
                .unwrap_or_else(|| Utc::now().date_naive());
            let value = compute_session_metric(metric, &session_sets);
            (date, value)
        })
        .collect();

    points.sort_by_key(|(date, _)| *date);
    downsample_graph_points(points, max_points, metric)
}

/// Downsamples a list of date/value pairs into a maximum number of points.
///
/// * `points`: Sorted list of (Date, Value)
/// * `max_points`: Target size
/// * `metric`: Used to determine aggregation strategy (Max vs Sum)
pub fn downsample_graph_points(
    points: Vec<(NaiveDate, f64)>,
    max_points: usize,
    metric: MetricKind,
) -> Vec<GraphPoint> {
    if points.is_empty() || max_points == 0 {
        return Vec::new();
    }

    if points.len() <= max_points {
        return points
            .into_iter()
            .map(|(date, value)| GraphPoint {
                date: date.format("%Y-%m-%d").to_string(),
                value,
            })
            .collect();
    }

    let bucket_size = points.len().div_ceil(max_points);
    let mut reduced = Vec::new();
    for chunk in points.chunks(bucket_size) {
        let date = chunk
            .first()
            .map(|(d, _)| *d)
            .unwrap_or_else(|| Utc::now().date_naive());
        let aggregated = match metric {
            MetricKind::MaxWeight | MetricKind::Est1Rm => chunk
                .iter()
                .fold(0.0_f64, |acc, (_, value)| acc.max(*value)),
            MetricKind::SessionTotalVolume | MetricKind::BestSetVolume => {
                chunk.iter().map(|(_, value)| *value).sum::<f64>()
            }
        };
        reduced.push(GraphPoint {
            date: date.format("%Y-%m-%d").to_string(),
            value: aggregated,
        });
    }

    reduced
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_sets() -> Vec<SetDataPoint> {
        vec![
            SetDataPoint {
                session_id: 1,
                date: NaiveDate::from_ymd_opt(2024, 5, 1).unwrap(),
                weight: 100.0,
                reps: 5,
            },
            SetDataPoint {
                session_id: 1,
                date: NaiveDate::from_ymd_opt(2024, 5, 1).unwrap(),
                weight: 105.0,
                reps: 3,
            },
            SetDataPoint {
                session_id: 2,
                date: NaiveDate::from_ymd_opt(2024, 5, 8).unwrap(),
                weight: 110.0,
                reps: 2,
            },
        ]
    }

    #[test]
    fn estimates_one_rm_with_epley_formula() {
        assert_eq!(estimate_one_rm(100.0, 1), 100.0);
        assert_eq!(estimate_one_rm(120.0, 0), 120.0);
        assert!((estimate_one_rm(100.0, 5) - 116.666).abs() < 0.01);
    }

    #[test]
    fn computes_session_metrics() {
        let sets = sample_sets();
        assert_eq!(
            compute_session_metric(MetricKind::MaxWeight, &sets[..2]),
            105.0
        );
        assert_eq!(
            compute_session_metric(MetricKind::SessionTotalVolume, &sets[..2]),
            100.0 * 5.0 + 105.0 * 3.0
        );
        assert_eq!(
            compute_session_metric(MetricKind::BestSetVolume, &sets[..2]),
            100.0 * 5.0
        );
        assert!(compute_session_metric(MetricKind::Est1Rm, &sets[..2]) > 105.0);
    }

    #[test]
    fn builds_and_downsamples_graph_points() {
        let mut sets = Vec::new();
        for day in 1..=6_i64 {
            sets.push(SetDataPoint {
                session_id: day,
                date: NaiveDate::from_ymd_opt(2024, 6, day as u32).unwrap(),
                weight: 50.0 + day as f64,
                reps: 5,
            });
        }

        let points = build_graph_points(sets, MetricKind::MaxWeight, 2);
        assert_eq!(points.len(), 2);
        assert_eq!(points[0].date, "2024-06-01");
        assert!(points[1].value > points[0].value);
    }
}
