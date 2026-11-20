//! Common items for `machine_locks_client` and `machine_locks_server`

use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use tabled::Tabled;
use tabled::derive::display;

#[derive(Debug, Clone, Deserialize, Serialize, Tabled)]
pub struct Machine {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub created_at: NaiveDateTime,
    #[serde(default)]
    pub available: bool,
    pub name: String,
    #[tabled(display("display::option", "-"))]
    pub poweroff_name: Option<String>,
    #[tabled(display("display::option", "-"))]
    pub arch: Option<String>,
    #[tabled(display("display::option", "-"))]
    pub sel4_plat: Option<String>,
    #[tabled(display("display::option", "-"))]
    pub isa: Option<String>,
    #[tabled(display("display::option", "-"))]
    pub soc: Option<String>,
    #[tabled(display("display::option", "-"))]
    pub cpu: Option<String>,
    #[tabled(display("display::option", "-"))]
    pub cores: Option<String>,
    #[tabled(display("display::option", "-"))]
    pub ram: Option<i64>,
    #[tabled(display("display::option", "-"))]
    pub max_clk: Option<f64>,
    #[tabled(display("display::option", "-"))]
    pub num_files: Option<i64>,
    #[tabled(display("display::option", "-"))]
    pub linux_num_files: Option<i64>,
    #[serde(default)]
    pub pxe_linux: bool,
    #[serde(default)]
    pub dtb: bool,
}

pub fn default_timeout() -> Option<i64> {
    Some(
        dotenvy::var("DEFAULT_TIMEOUT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(7200),
    )
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Tabled)]
pub struct Job {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub created_at: NaiveDateTime,
    #[tabled(display("display::option", "-"))]
    pub acquired_lock_at: Option<NaiveDateTime>,
    #[tabled(display("display::option", "-"))]
    pub released_lock_at: Option<NaiveDateTime>,
    #[tabled(display("display::option", "-"))]
    #[serde(default = "default_timeout")]
    pub timeout: Option<i64>,
    #[serde(default)]
    pub priority: i64,
    #[tabled(display("display::option", "-"))]
    pub job_name: Option<String>,
    pub owner: String,
    pub machine_id: i64,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum PollStatus {
    JobReady(Job),
    Waiting,
}
