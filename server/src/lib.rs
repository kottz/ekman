mod auth;
mod db;
mod error;
mod routes;

use axum::{
    Router,
    http::{HeaderValue, Method, header},
};
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tower_http::{compression::CompressionLayer, cors::CorsLayer, trace::TraceLayer};
use tracing::info;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

pub use error::{Error, Result};

#[derive(Clone)]
pub struct State {
    pub db: turso::Database,
}

pub async fn run() -> Result<()> {
    init_tracing();

    let port: u16 = std::env::var("EKMAN__SERVER__PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3000);

    let db_path = std::env::var("EKMAN__DATABASE__PATH").unwrap_or_else(|_| "ekman.db".into());

    let origins: Vec<HeaderValue> = std::env::var("EKMAN__SERVER__CORS_ORIGINS")
        .unwrap_or_else(|_| "http://localhost:5173".into())
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();

    let db = db::init(&db_path).await?;
    let state = State { db };

    let cors = CorsLayer::new()
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
        ])
        .allow_origin(origins)
        .allow_credentials(true)
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION, header::ACCEPT]);

    let app = Router::new()
        .merge(routes::api())
        .with_state(state)
        .layer(CompressionLayer::new())
        .layer(cors)
        .layer(TraceLayer::new_for_http());

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = TcpListener::bind(addr).await?;
    info!("listening on {addr}, database: {db_path}");
    axum::serve(listener, app).await?;
    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("warn,ekman_server=info"));

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_target(false))
        .with(filter)
        .try_init()
        .ok();
}
