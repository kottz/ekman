//! Background I/O task for network operations.

use chrono::NaiveDate;
use color_eyre::eyre::WrapErr;
use ekman_core::models::{
    ActivityRequest, ActivityResponse, DayExerciseSetsResponse, GraphRequest, GraphResponse,
    LoginRequest, LoginResponse, MeResponse, PopulatedTemplate, RegisterRequest, SetForDayRequest,
    SetForDayResponse,
};
use reqwest::Url;
use reqwest::cookie::{CookieStore, Jar};
use std::{fs, path::Path, sync::Arc};
use tokio::sync::mpsc;

const BACKEND_BASE_URL: &str = "http://localhost:3000";
const REGISTER_PATH: &str = "/api/auth/register";
const LOGIN_PATH: &str = "/api/auth/login";
const ME_PATH: &str = "/api/auth/me";
const DAILY_PLANS_PATH: &str = "/api/plans/daily";
const EXERCISES_PATH: &str = "/api/exercises";
const ACTIVITY_PATH: &str = "/api/activity/days";
const DAYS_PATH: &str = "/api/days";

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
    CheckSession,
    LoadDailyPlans,
    LoadGraph(i64),
    LoadActivityRange(ActivityRequest),
    LoadSetsForDayExercise {
        day: NaiveDate,
        exercise_id: i64,
    },
    SaveSet {
        exercise_id: i64,
        set_number: i32,
        day: NaiveDate,
        request: SetForDayRequest,
    },
    DeleteSet {
        exercise_id: i64,
        set_number: i32,
        day: NaiveDate,
    },
}

/// Responses from the background task.
#[derive(Debug)]
pub enum IoResponse {
    LoggedIn(Result<LoginResponse, String>),
    Registered(Result<LoginResponse, String>),
    SessionChecked(Result<MeResponse, String>),
    DailyPlans(Result<Vec<PopulatedTemplate>, String>),
    Graph(i64, Result<GraphResponse, String>),
    Activity(Result<ActivityResponse, String>),
    SetSaved {
        exercise_id: i64,
        set_number: i32,
        day: NaiveDate,
        result: Result<SetForDayResponse, String>,
    },
    SetsLoaded {
        exercise_id: i64,
        day: NaiveDate,
        result: Result<DayExerciseSetsResponse, String>,
    },
    SetDeleted {
        exercise_id: i64,
        set_number: i32,
        day: NaiveDate,
        result: Result<(), String>,
    },
}

pub fn build_client_with_store(path: &Path) -> color_eyre::Result<(reqwest::Client, Arc<Jar>)> {
    let jar = Arc::new(Jar::default());
    let url = backend_url()?;
    load_session_cookie(path, &jar, &url)?;
    let client = reqwest::Client::builder()
        .cookie_provider(Arc::clone(&jar))
        .build()
        .wrap_err("failed to build HTTP client")?;
    Ok((client, jar))
}

fn backend_url() -> color_eyre::Result<Url> {
    Url::parse(BACKEND_BASE_URL).wrap_err("invalid backend base url")
}

fn load_session_cookie(path: &Path, jar: &Arc<Jar>, url: &Url) -> color_eyre::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let data = fs::read_to_string(path).wrap_err("failed to read session cookie")?;
    if data.trim().is_empty() {
        return Ok(());
    }
    jar.add_cookie_str(data.trim(), url);
    Ok(())
}

pub fn save_session_cookie(path: &Path, jar: &Arc<Jar>) -> color_eyre::Result<()> {
    let url = backend_url()?;
    let Some(header) = jar.cookies(&url) else {
        return Ok(());
    };
    let value = header
        .to_str()
        .map_err(|_| color_eyre::eyre::eyre!("invalid cookie header"))?
        .to_string();
    if value.is_empty() {
        return Ok(());
    }
    let persisted = format!("{value}; Path=/");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).wrap_err("failed to create cookie dir")?;
    }
    fs::write(path, persisted).wrap_err("failed to write session cookie")
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

pub async fn check_session(client: &reqwest::Client) -> color_eyre::Result<MeResponse> {
    client
        .get(format!("{BACKEND_BASE_URL}{ME_PATH}"))
        .send()
        .await
        .wrap_err("request failed")?
        .error_for_status()
        .wrap_err("session check failed")?
        .json()
        .await
        .wrap_err("parse error")
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
            IoRequest::CheckSession => {
                let result = check_session(&client).await.map_err(|e| e.to_string());
                IoResponse::SessionChecked(result)
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
            IoRequest::LoadSetsForDayExercise { day, exercise_id } => {
                let result = fetch_sets_for_day_exercise(&client, day, exercise_id)
                    .await
                    .map_err(|e| e.to_string());
                IoResponse::SetsLoaded {
                    exercise_id,
                    day,
                    result,
                }
            }
            IoRequest::SaveSet {
                exercise_id,
                set_number,
                day,
                request,
            } => {
                let result = save_set(&client, day, exercise_id, set_number, request)
                    .await
                    .map_err(|e| e.to_string());
                IoResponse::SetSaved {
                    exercise_id,
                    set_number,
                    day,
                    result,
                }
            }
            IoRequest::DeleteSet {
                exercise_id,
                set_number,
                day,
            } => {
                let result = delete_set(&client, day, exercise_id, set_number)
                    .await
                    .map_err(|e| e.to_string());
                IoResponse::SetDeleted {
                    exercise_id,
                    set_number,
                    day,
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

async fn fetch_sets_for_day_exercise(
    client: &reqwest::Client,
    day: NaiveDate,
    exercise_id: i64,
) -> color_eyre::Result<DayExerciseSetsResponse> {
    let url = format!(
        "{BACKEND_BASE_URL}{DAYS_PATH}/{day}/exercises/{exercise_id}/sets",
        day = day.format("%Y-%m-%d"),
    );
    client
        .get(url)
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
    day: NaiveDate,
    exercise_id: i64,
    set_number: i32,
    request: SetForDayRequest,
) -> color_eyre::Result<SetForDayResponse> {
    let url = format!(
        "{BACKEND_BASE_URL}{DAYS_PATH}/{day}/exercises/{exercise_id}/sets/{set_number}",
        day = day.format("%Y-%m-%d"),
    );
    client
        .put(url)
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

async fn delete_set(
    client: &reqwest::Client,
    day: NaiveDate,
    exercise_id: i64,
    set_number: i32,
) -> color_eyre::Result<()> {
    let url = format!(
        "{BACKEND_BASE_URL}{DAYS_PATH}/{day}/exercises/{exercise_id}/sets/{set_number}",
        day = day.format("%Y-%m-%d"),
    );
    client
        .delete(url)
        .send()
        .await
        .wrap_err("request failed")?
        .error_for_status()
        .wrap_err("backend error")?;
    Ok(())
}
