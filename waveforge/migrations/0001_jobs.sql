CREATE TABLE IF NOT EXISTS jobs (
    id            UUID PRIMARY KEY,
    status        TEXT NOT NULL DEFAULT 'queued', -- queued | processing | done | failed
    original_name TEXT NOT NULL,
    error         TEXT,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
