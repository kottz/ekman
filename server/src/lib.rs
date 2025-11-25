mod db;
mod error;
mod handlers;
mod models;

use std::env;
use std::net::SocketAddr;

use axum::{
    Router,
    routing::{get, patch, post},
};
use tokio::net::TcpListener;
use tower_http::{compression::CompressionLayer, cors::CorsLayer, trace::TraceLayer};
use tracing::info;

pub use crate::db::{ensure_default_user, init_database};
pub use crate::error::{AppError, AppResult};
pub use crate::models::*;

#[derive(Clone)]
pub struct AppState {
    pub db: turso::Database,
    pub default_user_id: i64,
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/api/plans/daily", get(handlers::get_daily_plans))
        .route(
            "/api/exercises/{id}/graph",
            get(handlers::get_exercise_graph),
        )
        .route("/api/sessions", post(handlers::create_session))
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
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
}

pub async fn run() -> AppResult<()> {
    tracing_subscriber::fmt::init();

    let database_path = env::var("DATABASE_PATH").unwrap_or_else(|_| "gym.db".to_string());
    let username = env::var("APP_USERNAME").unwrap_or_else(|_| "demo".to_string());
    let port: u16 = env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3000);

    let db = init_database(&database_path).await?;
    let user_id = ensure_default_user(&db, &username).await?;

    let app = build_router(AppState {
        db,
        default_user_id: user_id,
    });

    let addr: SocketAddr = ([0, 0, 0, 0], port).into();
    let listener = TcpListener::bind(addr).await?;
    info!("listening on {addr}, database: {database_path}, user: {username}");
    axum::serve(listener, app.into_make_service()).await?;
    Ok(())
}
