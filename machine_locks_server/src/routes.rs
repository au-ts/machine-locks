// SPDX-FileCopyrightText: 2026 UNSW
//
// SPDX-License-Identifier: BSD-2-Clause

//!
//! Provided routes:
//! - Get pools
//! - [/machines]    Get machines (w/ lock/unlocked status)
//! - Get system job list
//! - Post job to pool
//! - Post job to system
//! - Poll job
//! - Release job
//! - Cancel job
//! - Forcibly cancel all jobs for machine
//!
//! - Create machine
//! - Edit machine
//! - Delete machine
//! - Create pool
//! - Edit pool
//! - Delete pool

use axum::{
    Json, Router,
    extract::{Path, State},
    response::IntoResponse,
    routing::{delete, get},
};
use sqlx::{query, query_as};

use crate::{AppResult, AppState, Machine, MachineInsert};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/machines", get(get_machines).post(create_machine))
        .route("/machines/{machine_id}", delete(delete_machine))
}

async fn get_machines(State(state): State<AppState>) -> AppResult<impl IntoResponse> {
    Ok(Json(
        query_as!(
            Machine,
            "
        SELECT * FROM machines
    "
        )
        .fetch_all(&state.db)
        .await?,
    ))
}

async fn create_machine(
    State(state): State<AppState>,
    Json(machine): Json<MachineInsert>,
) -> AppResult<impl IntoResponse> {
    query!(
        "INSERT INTO machines (
            name,
            poweroff_name,
            arch,
            sel4_plat,
            isa,
            soc,
            cpu,
            cores,
            ram,
            max_clk,
            num_files,
            linux_num_files,
            pxe_linux,
            dtb
        ) VALUES
        (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?);",
        machine.name,
        machine.poweroff_name,
        machine.arch,
        machine.sel4_plat,
        machine.isa,
        machine.soc,
        machine.cpu,
        machine.cores,
        machine.ram,
        machine.max_clk,
        machine.num_files,
        machine.linux_num_files,
        machine.pxe_linux,
        machine.dtb
    )
    .execute(&state.db)
    .await?;
    Ok(())
}

async fn delete_machine(
    State(state): State<AppState>,
    Path((id,)): Path<(i64,)>,
) -> AppResult<impl IntoResponse> {
    query!("DELETE FROM machines WHERE id = ?", id)
        .execute(&state.db)
        .await?;
    Ok(())
}
