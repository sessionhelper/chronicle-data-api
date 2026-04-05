# ovp-data-api

> Org-wide conventions (Rust style, git workflow, shared-secret auth, cross-service architecture) live in `/home/alex/sessionhelper-hub/CLAUDE.md`. Read that first for anything cross-cutting.

Internal storage abstraction for the OVP system. The only service that touches Postgres and S3 — all others are HTTP clients.

## What this does

- Axum HTTP server, binds `127.0.0.1:8001` (localhost only)
- Owns Postgres via `sqlx` (runtime-checked queries, migrations in `migrations/`)
- Owns S3 (Hetzner Object Storage) — audio chunks, metadata files
- Issues service session tokens, validates Bearer tokens on every protected route

No business logic. Every endpoint validates the session token via middleware, calls a db/storage function, returns JSON.

## Project layout

```
src/
  main.rs          — startup, router, session reaper task
  config.rs        — env config
  error.rs         — AppError with IntoResponse
  auth/            — session token issue/validate, middleware
  db/              — Postgres CRUD (sessions, users, participants, segments, flags, audit)
  storage/         — S3 operations
  routes/          — Axum handlers
migrations/        — 001_initial, 002_transcripts, 003_service_sessions
```

## Repo-specific conventions

- Use `sqlx::query` / `sqlx::query_as` (runtime-checked, not compile-time macros — migrations aren't always applied when tools run).
- All routes return JSON. Errors return `{ "error": "..." }` with the right status.
- Session tokens stored as SHA-256 hash in `service_sessions` for lookup.
- Structured logging via `tracing`.

## Env vars

| Var | Required | Default |
|---|---|---|
| `DATABASE_URL` | yes | — |
| `S3_ENDPOINT` | yes | — |
| `S3_ACCESS_KEY` / `S3_SECRET_KEY` | yes | — |
| `S3_BUCKET` | no | `ttrpg-dataset-raw` |
| `SHARED_SECRET` | yes | — |
| `BIND_ADDR` | no | `127.0.0.1:8001` |

## Build / run

```bash
cargo build --release
cargo check
cargo test
```
