pub mod converter;

use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Multipart, Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde_json::json;
use sqlx::PgPool;
use std::{path::PathBuf, sync::Arc};
use tokio::sync::Semaphore;
use tokio_util::io::ReaderStream;
use tower_http::services::ServeDir;
use uuid::Uuid;

pub const MAX_UPLOAD_BYTES: usize = 50 * 1024 * 1024;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub dir: PathBuf,
    pub workers: Arc<Semaphore>,
}

impl AppState {
    pub fn new(pool: PgPool, dir: PathBuf) -> Self {
        Self { pool, dir, workers: Arc::new(Semaphore::new(2)) }
    }
}

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/api/convert", post(create_job))
        .route("/api/jobs/:id", get(job_status))
        .route("/api/jobs/:id/download", get(download))
        .layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES))
        .fallback_service(ServeDir::new("static"))
        .with_state(state)
}

fn err(code: StatusCode, msg: &str) -> Response {
    (code, Json(json!({ "error": msg }))).into_response()
}

/// Keep only a safe stem for the download filename (no paths, no extension).
pub fn sanitize(name: &str) -> String {
    let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name);
    let s: String = stem
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(64)
        .collect();
    if s.is_empty() { "audio".into() } else { s }
}

async fn create_job(State(st): State<AppState>, mut mp: Multipart) -> Response {
    let mut upload: Option<(String, Vec<u8>)> = None;
    while let Ok(Some(field)) = mp.next_field().await {
        if field.name() == Some("file") {
            let name = field.file_name().unwrap_or("audio").to_string();
            match field.bytes().await {
                Ok(b) => upload = Some((name, b.to_vec())),
                Err(_) => return err(StatusCode::PAYLOAD_TOO_LARGE, "upload too large or interrupted"),
            }
            break;
        }
    }
    let Some((name, bytes)) = upload.filter(|(_, b)| !b.is_empty()) else {
        return err(StatusCode::BAD_REQUEST, "missing or empty 'file' field");
    };

    let id = Uuid::new_v4(); // server-chosen; the uploaded filename never touches the disk
    let input = st.dir.join(format!("{id}.in"));
    if tokio::fs::write(&input, &bytes).await.is_err() {
        return err(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
    }
    let inserted = sqlx::query("INSERT INTO jobs (id, original_name) VALUES ($1, $2)")
        .bind(id)
        .bind(sanitize(&name))
        .execute(&st.pool)
        .await;
    if inserted.is_err() {
        let _ = tokio::fs::remove_file(&input).await;
        return err(StatusCode::INTERNAL_SERVER_ERROR, "database error");
    }
    tokio::spawn(run_job(st, id, input));
    (StatusCode::ACCEPTED, Json(json!({ "id": id }))).into_response()
}

async fn run_job(st: AppState, id: Uuid, input: PathBuf) {
    let _permit = st.workers.acquire().await; // cap concurrent conversions
    let _ = sqlx::query("UPDATE jobs SET status = 'processing' WHERE id = $1")
        .bind(id)
        .execute(&st.pool)
        .await;

    let output = st.dir.join(format!("{id}.wav"));
    let (i, o) = (input.clone(), output.clone());
    let outcome = match tokio::task::spawn_blocking(move || converter::convert(&i, &o)).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) => Err(e.to_string()),
        Err(e) => Err(e.to_string()),
    };
    let _ = tokio::fs::remove_file(&input).await;

    match outcome {
        Ok(()) => {
            let _ = sqlx::query("UPDATE jobs SET status = 'done' WHERE id = $1")
                .bind(id)
                .execute(&st.pool)
                .await;
        }
        Err(msg) => {
            let _ = tokio::fs::remove_file(&output).await;
            let _ = sqlx::query("UPDATE jobs SET status = 'failed', error = $2 WHERE id = $1")
                .bind(id)
                .bind(msg)
                .execute(&st.pool)
                .await;
        }
    }
}

async fn job_status(State(st): State<AppState>, Path(id): Path<Uuid>) -> Response {
    let row = sqlx::query_as::<_, (String, Option<String>)>("SELECT status, error FROM jobs WHERE id = $1")
        .bind(id)
        .fetch_optional(&st.pool)
        .await;
    match row {
        Ok(Some((status, error))) => Json(json!({ "id": id, "status": status, "error": error })).into_response(),
        Ok(None) => err(StatusCode::NOT_FOUND, "job not found"),
        Err(_) => err(StatusCode::INTERNAL_SERVER_ERROR, "database error"),
    }
}

async fn download(State(st): State<AppState>, Path(id): Path<Uuid>) -> Response {
    let row = sqlx::query_as::<_, (String, String)>("SELECT status, original_name FROM jobs WHERE id = $1")
        .bind(id)
        .fetch_optional(&st.pool)
        .await;
    match row {
        Ok(Some((status, name))) if status == "done" => {
            match tokio::fs::File::open(st.dir.join(format!("{id}.wav"))).await {
                Ok(f) => Response::builder()
                    .header(header::CONTENT_TYPE, "audio/wav")
                    .header(header::CONTENT_DISPOSITION, format!("attachment; filename=\"{name}.wav\""))
                    .body(Body::from_stream(ReaderStream::new(f)))
                    .unwrap(),
                Err(_) => err(StatusCode::GONE, "file expired"),
            }
        }
        Ok(Some(_)) => err(StatusCode::CONFLICT, "job not finished"),
        Ok(None) => err(StatusCode::NOT_FOUND, "job not found"),
        Err(_) => err(StatusCode::INTERNAL_SERVER_ERROR, "database error"),
    }
}

/// Delete jobs (and their files) older than `max_age_minutes`.
pub async fn cleanup(st: &AppState, max_age_minutes: i32) {
    let rows = sqlx::query_as::<_, (Uuid,)>(
        "DELETE FROM jobs WHERE created_at < now() - make_interval(mins => $1) RETURNING id",
    )
    .bind(max_age_minutes)
    .fetch_all(&st.pool)
    .await;
    if let Ok(rows) = rows {
        for (id,) in rows {
            let _ = tokio::fs::remove_file(st.dir.join(format!("{id}.wav"))).await;
            let _ = tokio::fs::remove_file(st.dir.join(format!("{id}.in"))).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::sanitize;

    #[test]
    fn sanitize_strips_paths_and_extension() {
        assert_eq!(sanitize("../../etc/passwd.m4a"), "etcpasswd");
        assert_eq!(sanitize("my song (live).mp4"), "mysonglive");
        assert_eq!(sanitize("///.m4a"), "audio");
        assert_eq!(sanitize(&"a".repeat(200)).len(), 64);
    }
}
