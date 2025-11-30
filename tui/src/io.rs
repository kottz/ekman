//! Background I/O task for network operations.

use color_eyre::eyre::WrapErr;
use ekman_core::models::{
    ActivityRequest, ActivityResponse, GraphRequest, GraphResponse, PopulatedTemplate,
    UpsertSetRequest, UpsertSetResponse,
};
use tokio::sync::mpsc;

const BACKEND_BASE_URL: &str = "http://localhost:3000";
const DAILY_PLANS_PATH: &str = "/api/plans/daily";
const EXERCISES_PATH: &str = "/api/exercises";
const ACTIVITY_PATH: &str = "/api/activity/days";
const SETS_PATH: &str = "/api/sets";

/// Events sent to the background task.
#[derive(Debug)]
#[allow(clippy::enum_variant_names)]
pub enum IoRequest {
    LoadDailyPlans,
    LoadGraph(i64),
    LoadActivityRange(ActivityRequest),
    SaveSet {
        exercise_id: i64,
        set_index: usize,
        request: UpsertSetRequest,
    },
}

/// Responses from the background task.
#[derive(Debug)]
pub enum IoResponse {
    DailyPlans(Result<Vec<PopulatedTemplate>, String>),
    Graph(i64, Result<GraphResponse, String>),
    Activity(Result<ActivityResponse, String>),
    SetSaved {
        exercise_id: i64,
        set_index: usize,
        result: Result<UpsertSetResponse, String>,
    },
}

/// Spawn the background I/O task.
pub fn spawn() -> (mpsc::Sender<IoRequest>, mpsc::Receiver<IoResponse>) {
    let (req_tx, req_rx) = mpsc::channel(16);
    let (resp_tx, resp_rx) = mpsc::channel(16);

    tokio::spawn(run(req_rx, resp_tx));

    (req_tx, resp_rx)
}

async fn run(mut rx: mpsc::Receiver<IoRequest>, tx: mpsc::Sender<IoResponse>) {
    let client = reqwest::Client::new();

    while let Some(request) = rx.recv().await {
        let response = match request {
            IoRequest::LoadDailyPlans => {
                IoResponse::DailyPlans(fetch_daily_plans(&client).await.map_err(|e| e.to_string()))
            }
            IoRequest::LoadGraph(id) => {
                let result = fetch_graph(&client, id).await.map_err(|e| e.to_string());
                IoResponse::Graph(id, result)
            }
            IoRequest::LoadActivityRange(request) => {
                let result = fetch_activity(&client, request)
                    .await
                    .map_err(|e| e.to_string());
                IoResponse::Activity(result)
            }
            IoRequest::SaveSet {
                exercise_id,
                set_index,
                request,
            } => {
                let result = save_set(&client, request).await.map_err(|e| e.to_string());
                IoResponse::SetSaved {
                    exercise_id,
                    set_index,
                    result,
                }
            }
        };

        if tx.send(response).await.is_err() {
            break;
        }
    }
}

async fn fetch_daily_plans(client: &reqwest::Client) -> color_eyre::Result<Vec<PopulatedTemplate>> {
    client
        .get(format!("{BACKEND_BASE_URL}{DAILY_PLANS_PATH}"))
        .send()
        .await
        .wrap_err("request failed")?
        .error_for_status()
        .wrap_err("backend error")?
        .json()
        .await
        .wrap_err("parse error")
}

async fn fetch_graph(client: &reqwest::Client, id: i64) -> color_eyre::Result<GraphResponse> {
    client
        .get(format!("{BACKEND_BASE_URL}{EXERCISES_PATH}/{id}/graph"))
        .query(&GraphRequest {
            start: None,
            end: None,
            metric: None,
        })
        .send()
        .await
        .wrap_err("request failed")?
        .error_for_status()
        .wrap_err("backend error")?
        .json()
        .await
        .wrap_err("parse error")
}

async fn fetch_activity(
    client: &reqwest::Client,
    request: ActivityRequest,
) -> color_eyre::Result<ActivityResponse> {
    client
        .get(format!("{BACKEND_BASE_URL}{ACTIVITY_PATH}"))
        .query(&request)
        .send()
        .await
        .wrap_err("request failed")?
        .error_for_status()
        .wrap_err("backend error")?
        .json()
        .await
        .wrap_err("parse error")
}

async fn save_set(
    client: &reqwest::Client,
    request: UpsertSetRequest,
) -> color_eyre::Result<UpsertSetResponse> {
    client
        .put(format!("{BACKEND_BASE_URL}{SETS_PATH}"))
        .json(&request)
        .send()
        .await
        .wrap_err("request failed")?
        .error_for_status()
        .wrap_err("backend error")?
        .json()
        .await
        .wrap_err("parse error")
}
