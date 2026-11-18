// SPDX-FileCopyrightText: 2026 UNSW
//
// SPDX-License-Identifier: BSD-2-Clause
mod routes;

use std::{
    collections::HashMap,
    str::FromStr,
    sync::{Arc, Mutex},
};

use anyhow::Result;
use axum::{http::StatusCode, response::IntoResponse};
use chrono::NaiveDateTime;
use log::{error, info};
use serde::{Deserialize, Serialize};
use sqlx::{SqlitePool, migrate, query, query_as, sqlite::SqliteConnectOptions};
use tokio::{net::TcpListener, sync::broadcast};

use crate::routes::router;

#[derive(Debug, Clone)]
struct AppState {
    db: SqlitePool,
    /// Map from machine ID -> channel, where a broadcast indicates a change in the queue
    /// Broadcast is sent whenever a lock is released
    machine_channels: Arc<Mutex<HashMap<i64, broadcast::Sender<()>>>>,
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

    async fn query_next_job(&self, machine_id: i64) -> AppResult<Option<Job>> {
        query_as!(
            Job,
            "SELECT * from jobs
            WHERE acquired_lock_at IS NULL AND machine_id = ?
            ORDER BY priority DESC, created_at ASC
            LIMIT 1",
            machine_id
        )
        .fetch_optional(&self.db)
        .await
        .map_err(Into::into)
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
    dotenvy::dotenv()?; // Load additional env vars from a .env file
    env_logger::init();

    // Initialise db
    let options =
        SqliteConnectOptions::from_str(&dotenvy::var("DATABASE_URL")?)?.create_if_missing(true);
    let pool = SqlitePool::connect_with(options).await?;
    migrate!().run(&pool).await?;

    let app = router().with_state(AppState {
        db: pool,
        machine_channels: Arc::new(Mutex::new(HashMap::new())),
    });

    let listener = TcpListener::bind("0.0.0.0:3000").await?;
    info!("Serving at http://localhost:3000");
    axum::serve(listener, app).await?;
    Ok(())
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct Machine {
    #[serde(default)]
    id: i64,
    #[serde(default)]
    created_at: NaiveDateTime,
    #[serde(default)]
    available: bool,
    name: String,
    poweroff_name: Option<String>,
    arch: Option<String>,
    sel4_plat: Option<String>,
    isa: Option<String>,
    soc: Option<String>,
    cpu: Option<String>,
    cores: Option<String>,
    ram: Option<i64>,
    max_clk: Option<f64>,
    num_files: Option<i64>,
    linux_num_files: Option<i64>,
    #[serde(default)]
    pxe_linux: bool,
    #[serde(default)]
    dtb: bool,
}

fn default_timeout() -> Option<i64> {
    Some(
        dotenvy::var("DEFAULT_TIMEOUT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(7200),
    )
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
struct Job {
    #[serde(default)]
    id: i64,
    #[serde(default)]
    created_at: NaiveDateTime,
    acquired_lock_at: Option<NaiveDateTime>,
    released_lock_at: Option<NaiveDateTime>,
    #[serde(default = "default_timeout")]
    timeout: Option<i64>,
    #[serde(default)]
    priority: i64,
    job_name: Option<String>,
    owner: String,
    machine_id: i64,
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
