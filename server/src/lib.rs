mod db;
mod error;
mod handlers;

use std::net::SocketAddr;

use axum::{
    Router,
    http::{HeaderValue, Method, header},
    routing::{get, patch, post, put},
};
use config::Config;
use serde::Deserialize;
use tokio::net::TcpListener;
use tower_http::{compression::CompressionLayer, cors::CorsLayer, trace::TraceLayer};
use tracing::info;

pub use crate::db::init_database;
pub use crate::error::{AppError, AppResult};
pub use ekman_core::models;

#[derive(Debug, Deserialize)]
struct ServerConfig {
    port: u16,
    cors_origins: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct DatabaseConfig {
    path: String,
}

#[derive(Debug, Deserialize)]
struct AppConfig {
    server: ServerConfig,
    database: DatabaseConfig,
}

#[derive(Clone)]
pub struct AppState {
    pub db: turso::Database,
}

pub fn build_router(state: AppState, cors_layer: CorsLayer) -> Router {
    Router::new()
        .route("/api/plans/daily", get(handlers::get_daily_plans))
        .route("/api/activity/days", get(handlers::get_activity_days))
        .route("/api/auth/register", post(handlers::register))
        .route("/api/auth/login", post(handlers::login))
        .route("/api/auth/logout", post(handlers::logout))
        .route("/api/auth/me", get(handlers::me))
        .route("/api/auth/totp/setup", get(handlers::totp_setup))
        .route("/api/auth/totp/enable", post(handlers::totp_enable))
        .route(
            "/api/exercises/{id}/graph",
            get(handlers::get_exercise_graph),
        )
        .route("/api/sessions", post(handlers::create_session))
        .route("/api/sets", put(handlers::upsert_set))
        .route(
            "/api/sets/{id}",
            patch(handlers::update_set).delete(handlers::delete_set),
        )
        .route(
            "/api/exercises",
            get(handlers::list_exercises).post(handlers::create_exercise),
        )
        .route(
            "/api/exercises/{id}",
            patch(handlers::update_exercise).delete(handlers::archive_exercise),
        )
        .with_state(state)
        .layer(CompressionLayer::new())
        .layer(cors_layer)
        .layer(TraceLayer::new_for_http())
}

fn load_config() -> AppResult<AppConfig> {
    Config::builder()
        .add_source(
            config::Environment::with_prefix("EKMAN")
                .separator("__")
                .list_separator(",")
                .try_parsing(true)
                .with_list_parse_key("server.cors_origins"),
        )
        .build()
        .map_err(|err| AppError::Internal(format!("failed to load config: {err}")))?
        .try_deserialize()
        .map_err(|err| AppError::Internal(format!("failed to parse config: {err}")))
}

fn build_cors_layer(origins: &[String]) -> AppResult<CorsLayer> {
    let allowed_origins: Vec<HeaderValue> = origins
        .iter()
        .map(|origin| {
            origin
                .parse()
                .map_err(|err| AppError::Internal(format!("invalid CORS origin '{origin}': {err}")))
        })
        .collect::<Result<_, _>>()?;

    Ok(CorsLayer::new()
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
        ])
        .allow_origin(allowed_origins)
        .allow_credentials(true)
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION, header::ACCEPT]))
}

pub async fn run() -> AppResult<()> {
    tracing_subscriber::fmt::init();

    let app_config = load_config()?;
    let cors_layer = build_cors_layer(&app_config.server.cors_origins)?;

    let db = init_database(&app_config.database.path).await?;

    let app = build_router(AppState { db }, cors_layer);

    let addr: SocketAddr = ([0, 0, 0, 0], app_config.server.port).into();
    let listener = TcpListener::bind(addr).await?;
    info!(
        "listening on {addr}, database: {}",
        app_config.database.path
    );
    axum::serve(listener, app.into_make_service()).await?;
    Ok(())
}
