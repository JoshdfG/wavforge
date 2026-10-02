# WavForge
M4A / MP4 (AAC, ALAC) → WAV converter: Rust, axum, Postgres, symphonia.

    docker compose up -d postgres      # from the boilerplate
    cp .env.example .env               # set DATABASE_URL to match your Postgres creds
    cargo run                          # http://localhost:3000

    cargo test --lib                   # unit tests, no DB needed
    cargo test                         # + API tests (need DATABASE_URL, user must be able to CREATE DATABASE)

API: POST /api/convert (multipart field `file`) -> 202 {id}; GET /api/jobs/:id; GET /api/jobs/:id/download
Test fixtures: `tests/fixtures/make_fixtures.sh` (needs ffmpeg).
