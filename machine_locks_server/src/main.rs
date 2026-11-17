// SPDX-FileCopyrightText: 2026 UNSW
//
// SPDX-License-Identifier: BSD-2-Clause

use std::str::FromStr;

use anyhow::Result;
use axum::{Router, routing::get};
use log::info;
use sqlx::{SqlitePool, migrate, sqlite::SqliteConnectOptions};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv()?; // Load additional env vars from a .env file
    env_logger::init();
    let app = Router::new().route("/", get(|| async { "Hello, World!" }));

    // Initialise db
    let options =
        SqliteConnectOptions::from_str(&dotenvy::var("DATABASE_URL")?)?.create_if_missing(true);
    let pool = SqlitePool::connect_with(options).await?;
    migrate!().run(&pool).await?;

    let listener = TcpListener::bind("0.0.0.0:3000").await?;
    info!("Serving at http://localhost:3000");
    axum::serve(listener, app).await?;
    Ok(())
}
