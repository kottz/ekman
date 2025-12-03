mod db;
mod error;
mod handlers;

use std::net::SocketAddr;

use axum::{
    Router,
    http::{HeaderValue, Method, header},
    routing::{get, post, put},
};
use config::Config;
use serde::Deserialize;
use tokio::net::TcpListener;
use tower_http::{
    compression::CompressionLayer,
    cors::CorsLayer,
    trace::{DefaultMakeSpan, DefaultOnFailure, DefaultOnRequest, DefaultOnResponse, TraceLayer},
};
use tracing::{Level, info};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

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
        .route(
            "/api/days/{date}/exercises/{exercise_id}/sets",
            get(handlers::get_sets_for_day_exercise),
        )
        .route(
            "/api/days/{date}/exercises/{exercise_id}/sets/{set_number}",
            put(handlers::upsert_set_for_day).delete(handlers::delete_set_for_day),
        )
        .route(
            "/api/exercises",
            get(handlers::list_exercises).post(handlers::create_exercise),
        )
        .route(
            "/api/exercises/{id}",
            get(handlers::get_exercise).patch(handlers::update_exercise),
        )
        .route(
            "/api/exercises/{id}/archive",
            post(handlers::archive_exercise),
        )
        .with_state(state)
        .layer(CompressionLayer::new())
        .layer(cors_layer)
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().level(Level::DEBUG))
                .on_request(DefaultOnRequest::new().level(Level::DEBUG))
                .on_response(DefaultOnResponse::new().level(Level::DEBUG))
                .on_failure(DefaultOnFailure::new().level(Level::ERROR)),
        )
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

fn init_tracing() {
    let env_filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new("warn,server=info,tower_http=warn"))
        .unwrap_or_else(|_| EnvFilter::new("info"));

    let subscriber = tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_target(false))
        .with(env_filter);

    let _ = subscriber.try_init();
}

pub async fn run() -> AppResult<()> {
    init_tracing();

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
