//! HTTP API client with background task.

use chrono::NaiveDate;
use ekman_core::{
    Activity, ActivityQuery, DaySets, Graph, GraphQuery, LoginInput, RegisterInput, Session,
    SetInput, Template, User, WorkoutSet,
};
use reqwest::{
    Client, Url,
    cookie::{CookieStore, Jar},
};
use std::{fs, sync::Arc};
use tokio::sync::mpsc;

const BASE_URL: &str = "http://localhost:3000";

/// Requests to the background task.
#[derive(Debug)]
pub enum Request {
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
    LoadPlans,
    LoadGraph(i64),
    LoadActivity(ActivityQuery),
    LoadSets {
        day: NaiveDate,
        exercise_id: i64,
    },
    SaveSet {
        day: NaiveDate,
        exercise_id: i64,
        set_number: i32,
        input: SetInput,
    },
    DeleteSet {
        day: NaiveDate,
        exercise_id: i64,
        set_number: i32,
    },
}

/// Responses from the background task.
#[derive(Debug)]
pub enum Response {
    LoggedIn(Result<Session, String>),
    Registered(Result<Session, String>),
    SessionChecked(Result<User, String>),
    Plans(Result<Vec<Template>, String>),
    Graph(i64, Result<Graph, String>),
    Activity(Result<Activity, String>),
    SetsLoaded {
        exercise_id: i64,
        day: NaiveDate,
        result: Result<DaySets, String>,
    },
    SetSaved {
        exercise_id: i64,
        day: NaiveDate,
        set_number: i32,
        result: Result<WorkoutSet, String>,
    },
    SetDeleted {
        exercise_id: i64,
        day: NaiveDate,
        set_number: i32,
        result: Result<(), String>,
    },
}

pub struct ApiClient {
    pub tx: mpsc::Sender<Request>,
    pub rx: mpsc::Receiver<Response>,
    pub jar: Arc<Jar>,
    pub cookie_path: std::path::PathBuf,
}

impl ApiClient {
    pub fn new(cookie_path: std::path::PathBuf) -> color_eyre::Result<Self> {
        let jar = Arc::new(Jar::default());
        let url: Url = BASE_URL.parse()?;

        // Load existing cookie
        if cookie_path.exists()
            && let Ok(data) = fs::read_to_string(&cookie_path)
            && !data.trim().is_empty()
        {
            jar.add_cookie_str(data.trim(), &url);
        }

        let client = Client::builder()
            .cookie_provider(Arc::clone(&jar))
            .build()?;

        let (req_tx, req_rx) = mpsc::channel(16);
        let (resp_tx, resp_rx) = mpsc::channel(16);

        tokio::spawn(run_worker(client, req_rx, resp_tx));

        Ok(Self {
            tx: req_tx,
            rx: resp_rx,
            jar,
            cookie_path,
        })
    }

    pub fn send(&self, req: Request) {
        let _ = self.tx.try_send(req);
    }

    pub fn save_cookie(&self) {
        let url: Url = match BASE_URL.parse() {
            Ok(u) => u,
            Err(_) => return,
        };

        if let Some(header) = self.jar.cookies(&url)
            && let Ok(value) = header.to_str()
            && !value.is_empty()
        {
            let persisted = format!("{value}; Path=/");
            if let Some(parent) = self.cookie_path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let _ = fs::write(&self.cookie_path, persisted);
        }
    }
}

async fn run_worker(client: Client, mut rx: mpsc::Receiver<Request>, tx: mpsc::Sender<Response>) {
    while let Some(req) = rx.recv().await {
        let resp = handle_request(&client, req).await;
        if tx.send(resp).await.is_err() {
            break;
        }
    }
}

async fn handle_request(client: &Client, req: Request) -> Response {
    match req {
        Request::Login {
            username,
            password,
            totp,
        } => {
            let result = client
                .post(format!("{BASE_URL}/api/auth/login"))
                .json(&LoginInput {
                    username,
                    password,
                    totp: Some(totp),
                })
                .send()
                .await
                .and_then(|r| r.error_for_status());

            match result {
                Ok(r) => Response::LoggedIn(r.json().await.map_err(|e| e.to_string())),
                Err(e) => Response::LoggedIn(Err(e.to_string())),
            }
        }

        Request::Register {
            username,
            password,
            totp_secret,
            totp_code,
        } => {
            let result = client
                .post(format!("{BASE_URL}/api/auth/register"))
                .json(&RegisterInput {
                    username,
                    password,
                    totp_secret,
                    totp_code,
                })
                .send()
                .await
                .and_then(|r| r.error_for_status());

            match result {
                Ok(r) => Response::Registered(r.json().await.map_err(|e| e.to_string())),
                Err(e) => Response::Registered(Err(e.to_string())),
            }
        }

        Request::CheckSession => {
            let result = client
                .get(format!("{BASE_URL}/api/auth/me"))
                .send()
                .await
                .and_then(|r| r.error_for_status());

            match result {
                Ok(r) => Response::SessionChecked(r.json().await.map_err(|e| e.to_string())),
                Err(e) => Response::SessionChecked(Err(e.to_string())),
            }
        }

        Request::LoadPlans => {
            let result = client
                .get(format!("{BASE_URL}/api/plans/daily"))
                .send()
                .await
                .and_then(|r| r.error_for_status());

            match result {
                Ok(r) => Response::Plans(r.json().await.map_err(|e| e.to_string())),
                Err(e) => Response::Plans(Err(e.to_string())),
            }
        }

        Request::LoadGraph(id) => {
            let result = client
                .get(format!("{BASE_URL}/api/exercises/{id}/graph"))
                .query(&GraphQuery {
                    start: None,
                    end: None,
                    metric: None,
                })
                .send()
                .await
                .and_then(|r| r.error_for_status());

            match result {
                Ok(r) => Response::Graph(id, r.json().await.map_err(|e| e.to_string())),
                Err(e) => Response::Graph(id, Err(e.to_string())),
            }
        }

        Request::LoadActivity(query) => {
            let result = client
                .get(format!("{BASE_URL}/api/activity/days"))
                .query(&query)
                .send()
                .await
                .and_then(|r| r.error_for_status());

            match result {
                Ok(r) => Response::Activity(r.json().await.map_err(|e| e.to_string())),
                Err(e) => Response::Activity(Err(e.to_string())),
            }
        }

        Request::LoadSets { day, exercise_id } => {
            let result = client
                .get(format!(
                    "{BASE_URL}/api/days/{}/exercises/{exercise_id}/sets",
                    day.format("%Y-%m-%d")
                ))
                .send()
                .await
                .and_then(|r| r.error_for_status());

            match result {
                Ok(r) => Response::SetsLoaded {
                    exercise_id,
                    day,
                    result: r.json().await.map_err(|e| e.to_string()),
                },
                Err(e) => Response::SetsLoaded {
                    exercise_id,
                    day,
                    result: Err(e.to_string()),
                },
            }
        }

        Request::SaveSet {
            day,
            exercise_id,
            set_number,
            input,
        } => {
            let result = client
                .put(format!(
                    "{BASE_URL}/api/days/{}/exercises/{exercise_id}/sets/{set_number}",
                    day.format("%Y-%m-%d")
                ))
                .json(&input)
                .send()
                .await
                .and_then(|r| r.error_for_status());

            match result {
                Ok(r) => Response::SetSaved {
                    exercise_id,
                    day,
                    set_number,
                    result: r.json().await.map_err(|e| e.to_string()),
                },
                Err(e) => Response::SetSaved {
                    exercise_id,
                    day,
                    set_number,
                    result: Err(e.to_string()),
                },
            }
        }

        Request::DeleteSet {
            day,
            exercise_id,
            set_number,
        } => {
            let result = client
                .delete(format!(
                    "{BASE_URL}/api/days/{}/exercises/{exercise_id}/sets/{set_number}",
                    day.format("%Y-%m-%d")
                ))
                .send()
                .await
                .and_then(|r| r.error_for_status());

            Response::SetDeleted {
                exercise_id,
                day,
                set_number,
                result: result.map(|_| ()).map_err(|e| e.to_string()),
            }
        }
    }
}
