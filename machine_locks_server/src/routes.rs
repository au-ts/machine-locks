// SPDX-FileCopyrightText: 2026 UNSW
//
// SPDX-License-Identifier: BSD-2-Clause

//!
//! Provided routes (not all implemented):
//! - Get pools
//! - [GET /machines]               Get machines
//! - [GET /machines/<id>/job]      Get jobs for a machine
//! - [GET /jobs]                   Get system job list
//! - Post job to pool
//! - Post job to system
//! - [GET /jobs/<id>/poll]         Poll job
//! - [POST /jobs/<id>/release]     Release job
//! - [DELETE /jobs/<id>]           Cancel job
//! - Forcibly cancel all jobs for machine
//!
//! - [POST /machines]              Create machine
//! - Edit machine
//! - [DELETE /machines/<id>]       Delete machine
//! - Create pool
//! - Edit pool
//! - Delete pool

use std::time::Duration;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
};
use serde::Serialize;
use serde_json::json;
use sqlx::{query, query_as};
use tokio::time::timeout;

use crate::{AppError, AppResult, AppState, Job, Machine};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/machines", get(get_machines).post(create_machine))
        .route("/machines/{machine_id}", delete(delete_machine))
        .route("/machines/{machine_id}/jobs", get(get_machine_jobs))
        .route("/jobs", get(get_jobs).post(create_job))
        .route("/jobs/{job_id}", delete(cancel_job))
        .route("/jobs/{job_id}/poll", get(poll_job))
        .route("/jobs/{job_id}/release", post(release_job))
}

async fn get_machine_jobs(
    State(state): State<AppState>,
    Path((id,)): Path<(i64,)>,
) -> AppResult<impl IntoResponse> {
    state.release_expired_lock(id).await?; // Idempotent
    Ok(Json(
        query_as!(
            Job,
            "
        SELECT * FROM jobs
        WHERE machine_id = ? AND released_lock_at IS NULL
    ",
            id
        )
        .fetch_all(&state.db)
        .await?,
    ))
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
    Json(machine): Json<Machine>,
) -> AppResult<impl IntoResponse> {
    let machine = query!(
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
        (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        RETURNING id",
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
    .fetch_one(&state.db)
    .await?;

    Ok(Json(json!({
        "id": machine.id
    })))
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

async fn get_jobs(State(state): State<AppState>) -> AppResult<impl IntoResponse> {
    Ok(Json(
        query_as!(Job, "SELECT * FROM jobs")
            .fetch_all(&state.db)
            .await?,
    ))
}

async fn create_job(
    State(state): State<AppState>,
    Json(job): Json<Job>,
) -> AppResult<impl IntoResponse> {
    state
        .validate_machine_available(job.machine_id)
        .await
        .map_err(|_| AppError(StatusCode::NOT_FOUND))?;

    let job = query!(
        "INSERT INTO jobs (timeout, priority, job_name, owner, machine_id) VALUES
        (?, ?, ?, ?, ?)
        RETURNING id",
        job.timeout,
        job.priority,
        job.job_name,
        job.owner,
        job.machine_id
    )
    .fetch_one(&state.db)
    .await?;

    Ok(Json(json!({
        "id": job.id
    })))
}

async fn cancel_job(
    State(state): State<AppState>,
    Path((id,)): Path<(i64,)>,
) -> AppResult<impl IntoResponse> {
    query!("DELETE FROM jobs WHERE id = ?", id)
        .execute(&state.db)
        .await?;
    Ok(())
}

async fn release_job(
    State(state): State<AppState>,
    Path((id,)): Path<(i64,)>,
) -> AppResult<impl IntoResponse> {
    let job = query_as!(
        Job,
        "UPDATE jobs SET
        released_lock_at = CURRENT_TIMESTAMP
        WHERE id = ?
        RETURNING *",
        id
    )
    .fetch_one(&state.db)
    .await?;

    let sender = state.get_or_create_sender(job.machine_id);
    let _ = sender.send(()); // Sending may fail if there are no listeners on the channel, that is OK
    Ok(Json(job))
}

#[derive(Debug, Serialize, PartialEq, Eq)]
enum PollStatus {
    JobReady(Job),
    Waiting,
}

async fn poll_job(
    State(state): State<AppState>,
    Path((id,)): Path<(i64,)>,
) -> AppResult<Json<PollStatus>> {
    let timeout_result = timeout(Duration::from_secs(30), async {
        let job = query_as!(Job, "SELECT * FROM jobs WHERE id = ?", id)
            .fetch_one(&state.db)
            .await
            .map_err(|_| AppError(StatusCode::NOT_FOUND))?;

        // Cancel job immediately if machines was marked as unavailable
        if state
            .validate_machine_available(job.machine_id)
            .await
            .is_err()
        {
            query!("DELETE FROM jobs WHERE id = ?", id)
                .execute(&state.db)
                .await?;
            return Err(AppError(StatusCode::NOT_FOUND));
        }

        let mut receiver = state.get_or_create_sender(job.machine_id).subscribe();

        loop {
            state.release_expired_lock(job.machine_id).await?; // Idempotent
            let next_job = state.query_next_job(job.machine_id).await?;
            let current_job = state.query_current_job(job.machine_id).await?;

            match (current_job, next_job) {
                (Some(job), _) if job.id == id => {
                    // Already acquired
                    return Ok(Json(PollStatus::JobReady(job)));
                }
                (None, Some(job)) if job.id == id => {
                    // Next job in line, acquire the machine
                    let job = query_as!(
                        Job,
                        "UPDATE jobs SET
                            acquired_lock_at = CURRENT_TIMESTAMP
                            WHERE id = ? RETURNING *",
                        id
                    )
                    .fetch_one(&state.db)
                    .await?; // This operation is idempotent

                    return Ok(Json(PollStatus::JobReady(job)));
                }
                _ => {} // Waiting
            }

            receiver.recv().await?;
        }
    })
    .await;

    match timeout_result {
        Err(_) => Ok(Json(PollStatus::Waiting)), // The operation timed out
        Ok(inner) => inner,                      // Pass through the operation of the timeout
    }
}
