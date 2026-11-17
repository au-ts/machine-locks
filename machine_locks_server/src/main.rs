// SPDX-FileCopyrightText: 2026 UNSW
//
// SPDX-License-Identifier: BSD-2-Clause
mod routes;

use std::str::FromStr;

use anyhow::Result;
use axum::{http::StatusCode, response::IntoResponse};
use chrono::NaiveDateTime;
use log::{error, info};
use serde::{Deserialize, Serialize};
use sqlx::{SqlitePool, migrate, sqlite::SqliteConnectOptions};
use tokio::net::TcpListener;

use crate::routes::router;

#[derive(Debug, Clone)]
pub struct AppState {
    pub db: SqlitePool,
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

    let app = router().with_state(AppState { db: pool });

    let listener = TcpListener::bind("0.0.0.0:3000").await?;
    info!("Serving at http://localhost:3000");
    axum::serve(listener, app).await?;
    Ok(())
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Machine {
    id: i64,
    created_at: NaiveDateTime,
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
    pxe_linux: bool,
    dtb: bool,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MachineInsert {
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
    pxe_linux: bool,
    dtb: bool,
}

pub struct AppError(pub StatusCode);
pub type AppResult<T> = Result<T, AppError>;

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
