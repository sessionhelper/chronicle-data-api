# OVP Data API

Internal storage abstraction for the OVP (Open Voice Pipeline) system. This is the only service that touches Postgres and S3. All other services (bot, pipeline, portal) are API clients.

## What this does

- Axum HTTP server binding to `127.0.0.1:8001` (localhost only)
- Owns Postgres via sqlx — runs migrations, handles all CRUD
- Owns S3 (Hetzner Object Storage) — audio chunk storage, metadata files
- Shared-secret auth + service sessions with heartbeat

## Architecture

No business logic. Pure CRUD + storage abstraction. Every endpoint validates the service session token via middleware, calls the appropriate db/storage function, and returns JSON.

## Auth model

1. All services share a secret (`SHARED_SECRET` env var)
2. Services POST /internal/auth with the shared secret to get a session token
3. Session token is used as Bearer token for all subsequent requests
4. Services send heartbeats every 30 seconds; sessions inactive >90s are reaped

## Project structure

```
src/main.rs          — Startup, router, background tasks
src/config.rs        — Config from env vars
src/error.rs         — AppError type with IntoResponse
src/auth/            — Admission token rotation, session management, middleware
src/db/              — Postgres CRUD (sessions, users, participants, segments, flags, audit)
src/storage/         — S3 operations (audio chunks, metadata files)
src/routes/          — Axum route handlers
migrations/          — SQL migrations (001-003)
```

## Conventions

- Use `sqlx::query` / `sqlx::query_as` (runtime checked, not compile-time)
- All routes return JSON; errors return `{ "error": "..." }` with appropriate status
- Structured logging via `tracing`
- Result + ? everywhere, no unwrap in non-test code
- Session token stored as hash in DB for lookup

## Required env vars

- `DATABASE_URL` — Postgres connection string
- `S3_ENDPOINT` — Object storage endpoint
- `S3_ACCESS_KEY` / `S3_SECRET_KEY` — S3 credentials
- `S3_BUCKET` — Bucket name (default: ttrpg-dataset-raw)
- `SHARED_SECRET` — Shared secret for service authentication (required)
- `BIND_ADDR` — Listen address (default: 127.0.0.1:8001)
