//! Background I/O task for network operations.

use color_eyre::eyre::WrapErr;
use ekman_core::models::{
    ActivityRequest, ActivityResponse, GraphRequest, GraphResponse, LoginRequest, LoginResponse,
    PopulatedTemplate, RegisterRequest, UpsertSetRequest, UpsertSetResponse,
};
use tokio::sync::mpsc;

const BACKEND_BASE_URL: &str = "http://localhost:3000";
const REGISTER_PATH: &str = "/api/auth/register";
const LOGIN_PATH: &str = "/api/auth/login";
const DAILY_PLANS_PATH: &str = "/api/plans/daily";
const EXERCISES_PATH: &str = "/api/exercises";
const ACTIVITY_PATH: &str = "/api/activity/days";
const SETS_PATH: &str = "/api/sets";

/// Events sent to the background task.
#[derive(Debug)]
#[allow(clippy::enum_variant_names)]
pub enum IoRequest {
    Login {
        username: String,
        password: String,
        totp: String,
    },
    Register {
        username: String,
        password: String,
        totp_secret: String,
        totp_code: String,
    },
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
    LoggedIn(Result<LoginResponse, String>),
    Registered(Result<LoginResponse, String>),
    DailyPlans(Result<Vec<PopulatedTemplate>, String>),
    Graph(i64, Result<GraphResponse, String>),
    Activity(Result<ActivityResponse, String>),
    SetSaved {
        exercise_id: i64,
        set_index: usize,
        result: Result<UpsertSetResponse, String>,
    },
}

pub fn build_client() -> color_eyre::Result<reqwest::Client> {
    reqwest::Client::builder()
        .cookie_store(true)
        .build()
        .wrap_err("failed to build HTTP client")
}

pub async fn login(
    client: &reqwest::Client,
    username: &str,
    password: &str,
    totp: &str,
) -> color_eyre::Result<LoginResponse> {
    let response = client
        .post(format!("{BACKEND_BASE_URL}{LOGIN_PATH}"))
        .json(&LoginRequest {
            username: username.to_string(),
            password: password.to_string(),
            totp: Some(totp.to_string()),
        })
        .send()
        .await
        .wrap_err("request failed")?
        .error_for_status()
        .wrap_err("login failed")?;

    response.json().await.wrap_err("parse error")
}

pub async fn register(
    client: &reqwest::Client,
    username: &str,
    password: &str,
    totp_secret: &str,
    totp_code: &str,
) -> color_eyre::Result<LoginResponse> {
    let response = client
        .post(format!("{BACKEND_BASE_URL}{REGISTER_PATH}"))
        .json(&RegisterRequest {
            username: username.to_string(),
            password: password.to_string(),
            totp_secret: totp_secret.to_string(),
            totp_code: totp_code.to_string(),
        })
        .send()
        .await
        .wrap_err("request failed")?
        .error_for_status()
        .wrap_err("register failed")?;

    response.json().await.wrap_err("parse error")
}

/// Spawn the background I/O task.
pub fn spawn(client: reqwest::Client) -> (mpsc::Sender<IoRequest>, mpsc::Receiver<IoResponse>) {
    let (req_tx, req_rx) = mpsc::channel(16);
    let (resp_tx, resp_rx) = mpsc::channel(16);

    tokio::spawn(run(client, req_rx, resp_tx));

    (req_tx, resp_rx)
}

async fn run(
    client: reqwest::Client,
    mut rx: mpsc::Receiver<IoRequest>,
    tx: mpsc::Sender<IoResponse>,
) {
    while let Some(request) = rx.recv().await {
        let response = match request {
            IoRequest::Login {
                username,
                password,
                totp,
            } => {
                let result = login(&client, &username, &password, &totp)
                    .await
                    .map_err(|e| e.to_string());
                IoResponse::LoggedIn(result)
            }
            IoRequest::Register {
                username,
                password,
                totp_secret,
                totp_code,
            } => {
                let result = register(&client, &username, &password, &totp_secret, &totp_code)
                    .await
                    .map_err(|e| e.to_string());
                IoResponse::Registered(result)
            }
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
