# CLAUDE.md

This repo is the YRAL billing service. It is a Rust/Axum API backed by SQLite and deployed as a Docker container behind Caddy.

## Current Live Runtime

The live runtime is intentionally simple for now. Do not assume distributed SQLite or LiteFS is active.

- `billing.sarvesh.yral.com` is served through Caddy on `sarvesh-1` and `sarvesh-2`.
- `sarvesh-1` runs the only live `yral-billing` application container and owns the writable SQLite DB.
- `sarvesh-2` is an edge/proxy only for billing. Its Caddy forwards billing traffic to `sarvesh-1`.
- `sarvesh-3` runs Sentry at `sentry.sarvesh.yral.com`.
- The billing DB lives on `sarvesh-1` at `/home/yral-deploy/yral-billing/data/billing.db`.
- The public health endpoint is `https://billing.sarvesh.yral.com/health`.

The live database was manually merged from old production data plus the small amount of new Sarvesh data before cutover. Last verified counts were:

- `bot_chat_access`: `463`
- `purchase_tokens`: `270`
- `transactions`: `426`
- `apple_app_account_tokens`: `0`

## Current Caddy Shape

Effective billing block on `sarvesh-1`:

```caddy
billing.sarvesh.yral.com:443 {
    tls /etc/caddy/certs/fullchain.pem /etc/caddy/certs/privkey.pem
    reverse_proxy yral-billing:3000 {
        health_uri /health
        health_interval 30s
        health_timeout 10s
    }
}
```

Effective billing block on `sarvesh-2`:

```caddy
billing.sarvesh.yral.com:443 {
    tls /etc/caddy/certs/fullchain.pem /etc/caddy/certs/privkey.pem
    reverse_proxy https://88.99.58.111 {
        header_up Host billing.sarvesh.yral.com
        transport http {
            tls_server_name billing.sarvesh.yral.com
            tls_insecure_skip_verify
        }
    }
}
```

The `tls_insecure_skip_verify` is only for the internal `sarvesh-2 -> sarvesh-1` Caddy hop because sarvesh-1 presents a shared/internal cert setup that does not validate cleanly by IP.

## CI/CD Behavior

Workflow: `.github/workflows/docker-publish.yml`

- `pull_request` to `main`: runs tests and builds Docker, but does not push or deploy.
- `push` to `main`: builds and pushes `ghcr.io/dolr-ai/yral-billing:latest`, then deploys.
- `workflow_dispatch`: builds and pushes, but deploy only runs if the ref is `refs/heads/main`.

The deploy matrix has roles:

- `sarvesh-1` is `primary`. It receives `docker-compose.yml` and `litestream.yml`, pulls the latest billing image, runs the billing container, runs Litestream, and configures local Caddy to proxy to `yral-billing:3000`.
- `sarvesh-2` is `edge`. It removes any local billing/Litestream containers and configures Caddy to proxy `billing.sarvesh.yral.com` to `sarvesh-1`.

This preserves the current single-writer SQLite topology during `main` deploys.

Do not reintroduce LiteFS/Consul until the servers can communicate on the required Consul/LiteFS ports. Previous checks showed Consul port `8301` was blocked between the Sarvesh servers.

## Server Names

Known Sarvesh servers:

- `sarvesh-1`: `88.99.58.111`
- `sarvesh-2`: `136.243.153.19`
- `sarvesh-3`: `138.201.57.116`

The deploy user used by CI/runtime is `yral-deploy`.

## Secrets And Env

Runtime env used by billing:

- `DATABASE_URL`: SQLite path. In Docker this is usually `/data/billing.db`.
- `PORT`: defaults to `3000`.
- `APP_ENV`: usually `production`.
- `GOOGLE_SERVICE_ACCOUNT_JSON`: Google service account JSON for Google Play API access.
- `APPLE_ISSUER_ID`: App Store Connect issuer ID for Apple purchase verification.
- `APPLE_KEY_ID`: App Store Connect API key ID.
- `APPLE_BUNDLE_ID`: iOS app bundle ID used for App Store Server API calls.
- `APPLE_PRIVATE_KEY`: App Store Connect `.p8` private key contents.
- `APPLE_DEFAULT_ENVIRONMENT`: Apple API environment, normally `production`.
- `BACKEND_ADMIN_SECRET_KEY`: IC admin identity private key.
- `SENTRY_DSN`: Sentry DSN.
- `SENTRY_TRACES_SAMPLE_RATE`: optional, defaults to `1.0`.

Litestream env used by the committed Compose file:

- `LITESTREAM_ACCESS_KEY_ID`
- `LITESTREAM_SECRET_ACCESS_KEY`
- `LITESTREAM_S3_BUCKET`
- `LITESTREAM_S3_ENDPOINT`: set in Compose as `fsn1.your-objectstorage.com`.

Never print secret values in logs or assistant replies. The Google service account JSON contains embedded newlines and should not be stored naively in a shell `.env` file unless it is safely quoted/escaped.

## Local Backup Directory

`old-prod-db/` contains local SQLite backup and merge artifacts. It must not be committed. It is ignored by `.gitignore`.

## Code Map

Entry points:

- `src/main.rs`: tiny binary entry point that calls `yral_billing::run()`.
- `src/lib.rs`: app startup, Sentry init, DB pool creation, migrations, route registration, Swagger/OpenAPI, and server bind.

State:

- `AppState` in `src/lib.rs` owns:
  - optional Google Play auth client,
  - optional IC admin agent,
  - cached Google public keys,
  - SQLite connection pool.
- With `--features=local`, Google auth and IC agent are disabled for local tests.

Database:

- Diesel SQLite is used.
- Migrations are embedded with `diesel_migrations` and run on application startup.
- Schema is generated in `src/schema.rs`.

Tables:

- `purchase_tokens`: Google subscription purchase tokens and status.
- `bot_chat_access`: bot access grants for Google and Apple purchases.
- `transactions`: reward transactions, currently `bot_subscription_reward`.
- `apple_app_account_tokens`: stable StoreKit `appAccountToken` UUIDs mapped to YRAL user IDs.

Models:

- `src/model.rs` has Diesel structs and constructors for the four tables.

Types:

- `src/types.rs` has API request/response DTOs, Google Play response structs, Apple StoreKit response structs, and Diesel-backed enums.
- Important enums:
  - `PurchaseTokenStatus`: `pending`, `access_granted`, `expired`
  - `BotChatAccessStatus`: `consume_pending`, `active`, `canceled`, `expired`
  - `PurchaseSource`: `google`, `apple`
  - `TransactionType`: `bot_subscription_reward`

Auth:

- `src/auth.rs` handles:
  - Google service account auth for Google Play API calls.
  - Google public JWK fetching and Google JWT validation.
  - Ed25519 bearer-token middleware for protected credit routes.

Errors:

- `src/error.rs` maps app errors to HTTP status codes and JSON `ApiResponse` bodies.
- Server-side errors are captured to Sentry.

Routes:

- `GET /health`: health check.
- `GET /`: redirects to `/explore`.
- `GET /explore`: Swagger UI.
- `GET /api-doc/openapi.json`: OpenAPI JSON.
- `POST /google/verify`: Google subscription verification.
- `POST /google/rtdn-webhook`: Google real-time developer notifications.
- `POST /google/chat-access/grant`: Google one-time product chat access grant.
- `GET /google/chat-access/check`: checks active bot chat access.
- `POST /apple/app-account-token`: creates or returns stable Apple `appAccountToken` for a user.
- `POST /apple/chat-access/grant`: verifies Apple transaction and grants chat access.
- `POST /apple/server-notifications`: handles App Store server notifications.
- `GET /transactions`: user transaction history.
- `GET /transactions/balance`: reward balance.
- `POST /credits/deduct` and `POST /credits/increment`: protected by JWT middleware.

Route modules:

- `src/routes/purchase.rs`: Google subscription verification.
- `src/routes/rtdn.rs`: Google RTDN webhook handling.
- `src/routes/chat_access.rs`: Google chat-access grant/check flow.
- `src/routes/apple_chat_access.rs`: Apple app account token, transaction grant, and server notification flow.
- `src/routes/apple_billing_helpers.rs`: App Store Server API and JWS verification helpers.
- `src/routes/goole_play_billing_helpers.rs`: Google Play API helpers. The filename contains a typo: `goole`, not `google`.
- `src/routes/purchase_token_helpers.rs`: purchase token DB helpers.
- `src/routes/transactions.rs`: transaction list and balance.
- `src/routes/credits.rs`: credit mutation endpoints.
- `src/routes/utils.rs`: shared route helpers.

## Build And Test

Common local checks:

```bash
cargo fmt
cargo check
cargo test --features=local -- --no-capture --test-threads=1
```

Docker image:

```bash
docker build -t ghcr.io/dolr-ai/yral-billing:latest .
```

The Dockerfile uses a multi-stage build:

- Rust builder image compiles the app and copies migrations/static assets.
- Debian runtime image contains only runtime dependencies, the compiled binary, migrations, and `entrypoint.sh`.
- Runtime runs as the `app` user.

## Operational Cautions

- SQLite is stateful. Do not run multiple writable app containers against separate DB files for the same production endpoint unless the product accepts divergent data.
- Do not delete or overwrite `/home/yral-deploy/yral-billing/data/billing.db` without a fresh SQLite backup.
- Before changing Caddy, back up `/home/yral-deploy/yral-proxy/Caddyfile`.
- Validate Caddy config before restart/reload:

```bash
docker exec yral-proxy caddy validate --config /etc/caddy/Caddyfile
```

- Verify health after deploy:

```bash
curl -kfsS --resolve billing.sarvesh.yral.com:443:88.99.58.111 https://billing.sarvesh.yral.com/health
curl -kfsS --resolve billing.sarvesh.yral.com:443:136.243.153.19 https://billing.sarvesh.yral.com/health
curl -kfsS https://billing.sarvesh.yral.com/health
```
