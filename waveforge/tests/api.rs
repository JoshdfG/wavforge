//! Integration tests. Need Postgres: DATABASE_URL must point at a server where
//! the user may create databases (sqlx::test makes a fresh DB per test).
use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tower::ServiceExt;
use wavforge::{app, AppState};

const MP3: &[u8] = include_bytes!("fixtures/sample.mp3");
const M4A: &[u8] = include_bytes!("fixtures/sample.m4a");
const MP4: &[u8] = include_bytes!("fixtures/sample.mp4");

fn test_app(pool: PgPool) -> (Router, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    (app(AppState::new(pool, dir.path().to_path_buf())), dir)
}

fn multipart(filename: &str, data: &[u8]) -> Request<Body> {
    let b = "XBOUNDARYX";
    let mut body = format!(
        "--{b}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(data);
    body.extend_from_slice(format!("\r\n--{b}--\r\n").as_bytes());
    Request::post("/api/convert")
        .header("content-type", format!("multipart/form-data; boundary={b}"))
        .body(Body::from(body))
        .unwrap()
}

async fn json(resp: axum::response::Response) -> serde_json::Value {
    serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

async fn wait_for_final_status(app: &Router, id: &str) -> serde_json::Value {
    for _ in 0..100 {
        let r = app
            .clone()
            .oneshot(
                Request::get(format!("/api/jobs/{id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let j = json(r).await;
        if j["status"] == "done" || j["status"] == "failed" {
            return j;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("job never finished");
}

async fn convert_ok(app: &Router, name: &str, data: &[u8]) -> Vec<u8> {
    let r = app.clone().oneshot(multipart(name, data)).await.unwrap();
    assert_eq!(r.status(), StatusCode::ACCEPTED);
    let id = json(r).await["id"].as_str().unwrap().to_string();
    let j = wait_for_final_status(app, &id).await;
    assert_eq!(j["status"], "done", "conversion failed: {}", j["error"]);

    let r = app
        .clone()
        .oneshot(
            Request::get(format!("/api/jobs/{id}/download"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(r.headers()["content-type"], "audio/wav");
    r.into_body().collect().await.unwrap().to_bytes().to_vec()
}

#[sqlx::test(migrations = "./migrations")]
async fn m4a_converts_to_valid_wav(pool: PgPool) {
    let (app, _dir) = test_app(pool);
    let wav = convert_ok(&app, "sample.m4a", M4A).await;
    assert_eq!(&wav[0..4], b"RIFF");
    assert_eq!(&wav[8..12], b"WAVE");
    let r = hound::WavReader::new(std::io::Cursor::new(wav)).unwrap();
    assert_eq!(r.spec().sample_rate, 44_100);
    assert_eq!(r.spec().channels, 1);
    // ~1s of audio; AAC adds priming/padding so allow slack
    assert!(
        (r.len() as i64 - 44_100).abs() < 4_096,
        "got {} samples",
        r.len()
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn mp4_video_audio_track_converts(pool: PgPool) {
    let (app, _dir) = test_app(pool);
    let wav = convert_ok(&app, "clip.mp4", MP4).await;
    assert_eq!(&wav[0..4], b"RIFF");
}

#[sqlx::test(migrations = "./migrations")]
async fn garbage_upload_ends_in_failed(pool: PgPool) {
    let (app, _dir) = test_app(pool);
    let r = app
        .clone()
        .oneshot(multipart("x.m4a", &[7u8; 2048]))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::ACCEPTED);
    let id = json(r).await["id"].as_str().unwrap().to_string();
    let j = wait_for_final_status(&app, &id).await;
    assert_eq!(j["status"], "failed");
    assert!(j["error"].is_string());

    let r = app
        .oneshot(
            Request::get(format!("/api/jobs/{id}/download"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::CONFLICT);
}

#[sqlx::test(migrations = "./migrations")]
async fn empty_upload_is_rejected(pool: PgPool) {
    let (app, _dir) = test_app(pool);
    let r = app.oneshot(multipart("empty.m4a", &[])).await.unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrations = "./migrations")]
async fn unknown_and_malformed_job_ids(pool: PgPool) {
    let (app, _dir) = test_app(pool);
    let r = app
        .clone()
        .oneshot(
            Request::get("/api/jobs/00000000-0000-0000-0000-000000000000")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    let r = app
        .oneshot(
            Request::get("/api/jobs/not-a-uuid")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrations = "./migrations")]
async fn cleanup_removes_expired_jobs_and_files(pool: PgPool) {
    let (app, dir) = test_app(pool.clone());
    let wav = convert_ok(&app, "sample.m4a", M4A).await;
    assert!(!wav.is_empty());
    sqlx::query("UPDATE jobs SET created_at = now() - interval '2 hours'")
        .execute(&pool)
        .await
        .unwrap();

    let st = AppState::new(pool.clone(), dir.path().to_path_buf());
    wavforge::cleanup(&st, 30).await;

    let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM jobs")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn mp3_converts_to_wav(pool: PgPool) {
    let (app, _dir) = test_app(pool);
    let wav = convert_ok(&app, "song.mp3", MP3).await;
    assert_eq!(&wav[0..4], b"RIFF");
}
