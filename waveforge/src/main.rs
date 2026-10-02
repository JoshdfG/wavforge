use axum::http::{HeaderValue, Method};
use std::{path::PathBuf, time::Duration};
use tower_http::cors::{Any, CorsLayer};
use wavforge::{app, cleanup, AppState};

#[tokio::main]
async fn main() {
    dotenv::dotenv().ok();
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(10)
        .connect(&url)
        .await
        .expect("cannot connect to database");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("migrations failed");

    let dir = PathBuf::from(std::env::var("STORAGE_DIR").unwrap_or_else(|_| "storage".into()));
    tokio::fs::create_dir_all(&dir)
        .await
        .expect("cannot create storage dir");
    let state = AppState::new(pool, dir);

    let bg = state.clone();
    tokio::spawn(async move {
        loop {
            cleanup(&bg, 30).await;
            tokio::time::sleep(Duration::from_secs(300)).await;
        }
    });

    let addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| {
        format!(
            "0.0.0.0:{}",
            std::env::var("PORT").unwrap_or_else(|_| "3000".into())
        )
    });
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    println!("WavForge listening on http://{addr}");
    let origins: Vec<HeaderValue> = std::env::var("ALLOWED_ORIGINS")
        .unwrap_or_else(|_| "http://localhost:3001".into())
        .split(',')
        .filter_map(|o| o.trim().parse().ok())
        .collect();
    let cors = CorsLayer::new()
        .allow_origin(origins)
        .allow_methods([Method::GET, Method::POST])
        .allow_headers(Any);

    axum::serve(listener, app(state).layer(cors)).await.unwrap();
}
