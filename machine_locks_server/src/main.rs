// SPDX-FileCopyrightText: 2026 UNSW
//
// SPDX-License-Identifier: BSD-2-Clause
mod auth;
mod routes;

use std::{
    collections::{HashMap, HashSet},
    str::FromStr,
    sync::{Arc, Mutex},
};

use anyhow::Result;
use axum::{http::StatusCode, middleware::from_fn_with_state, response::IntoResponse};
use common::Job;
use log::{error, info};
use openidconnect::{
    ClientId, IssuerUrl, JsonWebKeySetUrl,
    core::{CoreJsonWebKey, CoreJsonWebKeySet},
    reqwest::redirect::Policy,
};
use sqlx::{SqlitePool, migrate, query, query_as, sqlite::SqliteConnectOptions};
use tokio::{net::TcpListener, sync::broadcast};

use crate::routes::router;

#[derive(Debug, Clone)]
struct AppState {
    db: SqlitePool,
    /// Map from machine ID -> channel, where a broadcast indicates a change in the queue
    /// Broadcast is sent whenever a lock is released
    machine_channels: Arc<Mutex<HashMap<i64, broadcast::Sender<()>>>>,
    keys: Vec<CoreJsonWebKey>,
    client_ids: HashSet<ClientId>,
    issuer_urls: HashSet<IssuerUrl>,
}

impl AppState {
    fn get_or_create_sender(&self, machine_id: i64) -> broadcast::Sender<()> {
        let mut channels = self.machine_channels.lock().unwrap();
        let channel = channels.get(&machine_id);
        match channel {
            Some(sender) => sender.clone(),
            None => {
                let (sender, _) = broadcast::channel(16);
                channels.insert(machine_id, sender.clone());
                sender
            }
        }
    }

    async fn query_current_job(&self, machine_id: i64) -> AppResult<Option<Job>> {
        query_as!(
            Job,
            "SELECT * from jobs
            WHERE released_lock_at IS NULL AND machine_id = ? AND acquired_lock_at IS NOT NULL
            LIMIT 1",
            machine_id
        )
        .fetch_optional(&self.db)
        .await
        .map_err(Into::into)
    }

    /// Releases any expired lock. This is idempotent: it is safe to call it
    /// arbitrarily many times.
    async fn release_expired_lock(&self, machine_id: i64) -> AppResult<()> {
        query!(
            "UPDATE jobs SET
                released_lock_at = CURRENT_TIMESTAMP
                WHERE released_lock_at IS NULL
                AND machine_id = ?
                AND acquired_lock_at IS NOT NULL
                AND CURRENT_TIMESTAMP > datetime(acquired_lock_at, timeout || ' seconds')",
            &machine_id
        )
        .execute(&self.db)
        .await
        .map(|_| ())
        .map_err(Into::into)
    }

    /// Atomically advances the queue
    async fn advance_queue(&self, machine_id: i64) -> AppResult<()> {
        query!(
            "UPDATE jobs SET
            acquired_lock_at = CURRENT_TIMESTAMP
            WHERE id = ( -- next row to acquire
                SELECT id from jobs
                WHERE acquired_lock_at IS NULL AND machine_id = ?1
                ORDER BY priority DESC, created_at ASC
                LIMIT 1
            ) AND NOT EXISTS ( -- no job currently holds lock
                SELECT 1 FROM jobs
                WHERE released_lock_at IS NULL AND machine_id = ?1
                AND acquired_lock_at IS NOT NULL
            )",
            &machine_id
        )
        .execute(&self.db)
        .await
        .map(|_| ())
        .map_err(Into::into)
    }

    async fn validate_machine_available(&self, machine_id: i64) -> AppResult<()> {
        query!(
            "SELECT available FROM machines WHERE id = ? AND available = TRUE",
            machine_id
        )
        .fetch_one(&self.db)
        .await
        .map(|_| ())
        .map_err(|_| AppError(StatusCode::NOT_FOUND))
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv(); // Load additional env vars from a .env file
    env_logger::init();

    // Initialise db
    let options =
        SqliteConnectOptions::from_str(&dotenvy::var("DATABASE_URL")?)?.create_if_missing(true);
    let pool = SqlitePool::connect_with(options).await?;
    migrate!().run(&pool).await?;

    let client = openidconnect::reqwest::ClientBuilder::new()
        .redirect(Policy::none())
        .build()?;

    // Load keys
    info!("Loading JWKS keys");
    let mut keys = vec![];

    for url in dotenvy::var("JWKS_URLS")?.split(",") {
        let jwks = CoreJsonWebKeySet::fetch_async(&JsonWebKeySetUrl::new(url.to_owned())?, &client)
            .await?;
        keys.extend(jwks.keys().iter().cloned());
    }

    let state = AppState {
        db: pool,
        machine_channels: Arc::new(Mutex::new(HashMap::new())),
        keys,
        client_ids: dotenvy::var("CLIENT_IDS")?
            .split(",")
            .map(|id| ClientId::new(id.to_owned()))
            .collect(),
        issuer_urls: dotenvy::var("ISSUER_URLS")?
            .split(",")
            .flat_map(|url| IssuerUrl::new(url.to_owned()).into_iter())
            .collect(),
    };

    let app = router()
        .route_layer(from_fn_with_state(state.clone(), auth::layer))
        .with_state(state);

    let listener = TcpListener::bind("0.0.0.0:3000").await?;
    info!("Serving at http://localhost:3000");
    axum::serve(listener, app).await?;
    Ok(())
}

struct AppError(pub StatusCode);
type AppResult<T> = Result<T, AppError>;

impl<E> From<E> for AppError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        error!("error response: {}", err.into());
        Self(StatusCode::INTERNAL_SERVER_ERROR)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        self.0.into_response()
    }
}
