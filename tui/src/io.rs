//! Background I/O task for network operations.

use color_eyre::eyre::WrapErr;
use ekman_core::models::{GraphRequest, GraphResponse, PopulatedTemplate};
use tokio::sync::mpsc;

const BACKEND_BASE_URL: &str = "http://localhost:3000";
const DAILY_PLANS_PATH: &str = "/api/plans/daily";
const EXERCISES_PATH: &str = "/api/exercises";

/// Events sent to the background task.
#[derive(Debug)]
pub enum IoRequest {
    LoadDailyPlans,
    LoadGraph(i64),
}

/// Responses from the background task.
#[derive(Debug)]
pub enum IoResponse {
    DailyPlans(Result<Vec<PopulatedTemplate>, String>),
    Graph(i64, Result<GraphResponse, String>),
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
