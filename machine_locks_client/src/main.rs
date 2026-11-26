// SPDX-FileCopyrightText: 2026 UNSW
//
// SPDX-License-Identifier: BSD-2-Clause
mod auth;

use std::{collections::HashMap, env, path::PathBuf, process::exit, thread::sleep, time::Duration};

use anyhow::{Context, Result, anyhow, bail};
use chrono::NaiveDateTime;
use clap::{Parser, Subcommand};
use common::{
    Job, Machine,
    PollStatus::{self, JobReady},
};
use log::{error, info, warn};
use openidconnect::reqwest::{
    Url,
    blocking::{Client, RequestBuilder},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tabled::{
    Table,
    settings::{Color, Style, object::Rows},
};

use crate::auth::auth_commands;

#[derive(Parser)]
#[command(version, about)]
#[command(propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Interact directly with locks
    Sem {
        #[command(subcommand)]
        command: SemCommands,
    },
    /// Manage machines
    Machines {
        #[command(subcommand)]
        command: MachineCommands,
    },
    /// Manage authentication
    Auth {
        #[command(subcommand)]
        command: AuthCommands,
    },
}

#[derive(Subcommand)]
enum AuthCommands {
    Status,
    Login,
    Logout,
}

#[derive(Subcommand)]
enum SemCommands {
    /// Get information about a particular machine
    Info {
        /// The machine name
        machine: String,
        #[arg(long)]
        /// Output in machine readable JSON
        json: bool,
        #[arg(long, default_missing_value = "5", require_equals = true, num_args = 0..=1)]
        /// Whether to show as a live updating queue (ignored in JSON mode), specify a value for poll period in seconds
        poll: Option<u64>,
    },
    /// Acquire the lock for the specified machine
    Acquire {
        /// The machine name
        machine: String,
        #[arg(long)]
        owner: String,
        #[arg(long)]
        /// Optional job name
        job_name: Option<String>,
        #[arg(long)]
        /// Priority of the job, larger numbers are higher priority. Defaults
        /// to 0.
        priority: Option<i64>,
        #[arg(long)]
        /// Specify a timeout after which the lock will be released in seconds, the default
        /// is set by the server.
        timeout: Option<i64>,
        #[arg(long)]
        /// Disable the timeout; the lock will not be released until that is explicitly done.
        no_timeout: bool,
        #[arg(long)]
        /// Number of retries to re-acquire the lock, defaults to infinite
        num_retries: Option<u32>,
    },
    /// Release the lock for the specified machine
    Release {
        /// The machine name
        machine: String,
        #[arg(long)]
        owner: String,
        #[arg(short, long)]
        /// Release the lock, even if you do not currently own it
        force: bool,
    },
    /// Cancel a job before it executes
    Cancel { id: i64 },
}

#[derive(Subcommand)]
enum MachineCommands {
    /// List all machines
    List {
        #[arg(long)]
        json: bool,
    },
    /// List all locked machines
    ListLocked {
        #[arg(long)]
        json: bool,
    },
    /// Import machines from a TSV file, skipping already existing machines
    ImportTsv { file: PathBuf },
    /// Add a new machine
    Add,
    /// Remove a machine
    Remove { machine: String },
    Set {
        machine: String,
        key: String,
        value: String,
    },
}

#[derive(Debug, Clone)]
pub struct ApiClient {
    pub client: Client,
    pub base_url: Url,
}

impl ApiClient {
    pub fn new() -> Result<Self> {
        Ok(ApiClient {
            // TODO: configurable timeouts by env vars
            client: Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(60))
                .build()?,
            base_url: Url::parse(
                &env::var("MACHINE_LOCKS_BASE_URL")
                    .unwrap_or_else(|_| "https://trustworthy.systems/mq/".to_owned()),
            )?,
        })
    }

    pub fn get(&self, endpoint: &str) -> Result<RequestBuilder> {
        Ok(self.client.get(self.base_url.join(endpoint)?))
    }

    pub fn post(&self, endpoint: &str) -> Result<RequestBuilder> {
        Ok(self.client.post(self.base_url.join(endpoint)?))
    }

    pub fn delete(&self, endpoint: &str) -> Result<RequestBuilder> {
        Ok(self.client.delete(self.base_url.join(endpoint)?))
    }

    pub fn get_machine_by_name(&self, machine: &str) -> Result<Machine> {
        self.get(&format!("machines/by-name/{}", machine))?
            .send()?
            .error_for_status()?
            .json()
            .map_err(Into::into)
    }

    /// Gets active machine jobs sorted by queue order, including
    /// the current job running on the machine (this is first)
    pub fn get_machine_jobs(&self, machine_id: i64) -> Result<Vec<Job>> {
        self.get(&format!("machines/{}/jobs", machine_id))?
            .send()?
            .error_for_status()?
            .json()
            .map_err(Into::into)
    }

    pub fn acquire(
        &self,
        machine_id: i64,
        owner: &str,
        job_name: Option<&str>,
        priority: Option<i64>,
        no_timeout: bool,
        timeout: Option<i64>,
        num_retries: Option<u32>,
    ) -> Result<Job> {
        let mut payload = json!({
            "machine_id": machine_id,
            "owner": owner,
            "job_name": job_name,
            "priority": priority.unwrap_or_default()
        });

        match timeout {
            _ if no_timeout => {
                payload["timeout"] = json!(null);
            }
            Some(timeout) => {
                payload["timeout"] = json!(timeout);
            }
            None => {}
        }

        let job_id = self
            .post("jobs")?
            .json(&payload)
            .send()?
            .error_for_status()?
            .json::<Value>()?["id"]
            .as_number()
            .ok_or(anyhow!("Expected job ID to be a number"))?
            .as_i64()
            .ok_or(anyhow!("Expected job ID to fit in i64"))?;

        let self_clone = self.clone();
        if let Err(e) = ctrlc::set_handler(move || {
            if let Err(e) = self_clone.cancel(job_id) {
                error!("Failed to cancel job {:?}", e);
            }
            exit(1);
        }) {
            error!("Could not set up ctrl+c handler. {:?}", e);
        };

        let result = 'outer: {
            let iterator: Box<dyn Iterator<Item = u32>> = num_retries
                .map(|n| Box::new((0..n).into_iter()) as Box<dyn Iterator<Item = u32>>)
                .unwrap_or(Box::new(std::iter::repeat(0)));

            for _ in iterator {
                // This uses HTTP long polling, so the server will hold open the connection
                let status: PollStatus = self
                    .get(&format!("jobs/{}/poll", job_id))?
                    .send()?
                    .error_for_status()?
                    .json()?;

                info!("{:?}", status);

                match status {
                    JobReady(job) => break 'outer Ok(job),
                    PollStatus::JobTimedOut(job) => {
                        break 'outer Err(anyhow!("Job timed out {:?}", job));
                    }
                    PollStatus::Waiting => {}
                }
            }
            break 'outer Err(anyhow!("Failed to acquire lock within specified retries"));
        };

        if let Err(_) = result {
            self.cancel(job_id)
                .context("Failed to cancel job of failed acquisition")?;
        };

        result
    }

    pub fn release(&self, machine_id: i64, owner: &str, force: bool) -> Result<()> {
        let jobs = self.get_machine_jobs(machine_id)?;

        match jobs.first() {
            Some(active_job) if active_job.acquired_lock_at.is_some() => {
                if force || active_job.owner == owner {
                    // This is not a race: the specific job ID is specified and releasing is idempotent
                    self.post(&format!("jobs/{}/release", active_job.id))?
                        .send()?
                        .error_for_status()?;
                } else {
                    bail!("Will not release job you do not own, use --force to force release.")
                }
            }
            _ => bail!("No active job to release"),
        }

        Ok(())
    }

    pub fn get_machines(&self) -> Result<Vec<Machine>> {
        self.get("machines")?
            .send()?
            .error_for_status()?
            .json()
            .map_err(Into::into)
    }

    pub fn get_locked_machines(&self) -> Result<Vec<Machine>> {
        self.get("machines/locked")?
            .send()?
            .error_for_status()?
            .json()
            .map_err(Into::into)
    }

    pub fn create_machine(&self, machine: Machine) -> Result<()> {
        self.post("machines")?
            .json(&machine)
            .send()?
            .error_for_status()
            .map(|_| ())
            .map_err(Into::into)
    }

    pub fn remove_machine(&self, machine_id: i64) -> Result<()> {
        self.delete(&format!("machines/{}", machine_id))?
            .send()?
            .error_for_status()
            .map(|_| ())
            .map_err(Into::into)
    }

    fn cancel(&self, id: i64) -> Result<()> {
        self.delete(&format!("jobs/{}", id))?
            .send()?
            .error_for_status()
            .map(|_| ())
            .map_err(Into::into)
    }
}

pub fn dump_json<T>(value: &T) -> Result<(), serde_json::Error>
where
    T: ?Sized + Serialize,
{
    serde_json::to_writer(std::io::stdout().lock(), &value)?;
    println!();
    Ok(())
}

pub fn machines_table(machines: &Vec<Machine>) {
    let mut table = Table::new(machines);
    println!("{}", table.with(Style::modern()))
}

fn machine_comannd(client: &ApiClient, command: MachineCommands) -> Result<()> {
    match command {
        MachineCommands::Set {
            machine: _,
            key: _,
            value: _,
        } => todo!(),

        MachineCommands::Add => todo!(),

        MachineCommands::ListLocked { json } => {
            let machines = client.get_locked_machines()?;
            if json {
                dump_json(&machines)?;
            } else {
                machines_table(&machines);
            }
        }

        MachineCommands::List { json } => {
            let machines = client.get_machines()?;
            if json {
                dump_json(&machines)?;
            } else {
                machines_table(&machines);
            }
        }

        MachineCommands::ImportTsv { file } => {
            info!("file {:?}", file);
            let machines = client
                .get_machines()
                .with_context(|| "Failed to get machines")?;

            let machines: HashMap<&str, Machine> = machines
                .iter()
                .map(|machine| (machine.name.as_str(), machine.clone()))
                .collect();

            let mut csv_reader = csv::ReaderBuilder::new()
                .delimiter(b'\t')
                .comment(Some(b'#'))
                .from_reader(std::fs::File::open(file)?);

            for result in csv_reader.deserialize::<MachineRow>() {
                match result {
                    Ok(machine) if !machines.contains_key(machine.name.as_str()) => {
                        if let Err(e) = client.create_machine(machine.into()) {
                            error!("Failed to create machine: {:?}", e);
                        }
                    }
                    Ok(machine) => {
                        warn!("Already found machine with name {}", machine.name);
                    }
                    Err(e) => {
                        error!("Failed to decode row, {:?}", e);
                    }
                }
            }
        }

        MachineCommands::Remove { machine } => {
            let machine = client.get_machine_by_name(&machine)?;
            client.remove_machine(machine.id)?;
        }
    }
    Ok(())
}

fn sem_commands(client: &ApiClient, command: SemCommands) -> Result<()> {
    match command {
        SemCommands::Info {
            machine,
            json,
            poll,
        } => {
            let machine = client.get_machine_by_name(&machine)?;
            if json {
                let jobs = client.get_machine_jobs(machine.id)?;
                dump_json(&jobs)?;
            } else {
                loop {
                    let jobs = client.get_machine_jobs(machine.id)?;
                    jobs_table(&machine.name, jobs);
                    if let Some(poll_period) = poll {
                        sleep(Duration::from_secs(poll_period));
                        println!("\x1B[2J\x1B[1;1H ");
                    } else {
                        break;
                    }
                }
            }
        }

        SemCommands::Acquire {
            machine,
            owner,
            job_name,
            priority,
            no_timeout,
            timeout,
            num_retries,
        } => {
            let machine = client.get_machine_by_name(&machine)?;
            let job = client.acquire(
                machine.id,
                &owner,
                job_name.as_deref(),
                priority,
                no_timeout,
                timeout,
                num_retries,
            )?;
            dump_json(&job)?;
        }

        SemCommands::Release {
            machine,
            owner,
            force,
        } => {
            let machine = client.get_machine_by_name(&machine)?;
            client.release(machine.id, &owner, force)?;
        }
        SemCommands::Cancel { id } => {
            client.cancel(id)?;
        }
    }

    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let client = ApiClient::new()?;
    env_logger::init();

    match cli.command {
        Commands::Machines { command } => machine_comannd(&client, command)?,
        Commands::Sem { command } => sem_commands(&client, command)?,
        Commands::Auth { command } => auth_commands(command)?,
    }

    Ok(())
}

fn jobs_table(machine: &str, jobs: Vec<Job>) {
    println!("Jobs for {}, in queue order (current job first):", machine);
    let mut table = Table::new(&jobs);
    table.with(Style::modern());

    if let Some(job) = jobs.first()
        && job.acquired_lock_at.is_some()
    {
        table.modify(Rows::one(1), Color::FG_BLUE);
    }

    println!("{}", table);
}

#[derive(Debug, Clone, Deserialize)]
/// Table row in TSV import
pub struct MachineRow {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub created_at: NaiveDateTime,
    pub name: String,
    pub poweroff_name: Option<String>,
    pub arch: Option<String>,
    pub sel4_plat: Option<String>,
    pub isa: Option<String>,
    pub soc: Option<String>,
    pub cpu: Option<String>,
    pub cores: Option<String>,
    pub ram: Option<String>,
    pub max_clk: Option<String>,
    pub num_files: Option<i64>,
    pub linux_num_files: Option<i64>,
    pub pxe_linux: Option<String>,
    pub dtb: Option<String>,
}

fn parse_bool(s: Option<&str>) -> bool {
    s.map(|s| {
        let s = s.to_lowercase();
        let s = s.trim();
        s == "true" || s == "yes"
    })
    .unwrap_or_default()
}

impl From<MachineRow> for Machine {
    fn from(
        MachineRow {
            id,
            created_at,
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
            dtb,
        }: MachineRow,
    ) -> Self {
        Machine {
            id,
            created_at,
            available: true,
            name,
            poweroff_name,
            arch,
            sel4_plat,
            isa,
            soc,
            cpu,
            cores,
            ram: ram.and_then(|ram| {
                let ram = ram.trim().to_lowercase();
                if let Some(ram) = ram.strip_suffix("tb") {
                    return ram.parse().ok().map(|r: i64| r * 1024 * 1024);
                }
                if let Some(ram) = ram.strip_suffix("gb") {
                    return ram.parse().ok().map(|r: i64| r * 1024);
                }

                if let Some(ram) = ram.strip_suffix("mb") {
                    return ram.parse().ok();
                }

                None
            }),
            max_clk: max_clk.and_then(|max_clk| {
                let max_clk = max_clk.trim().to_lowercase();
                if let Some(max_clk) = max_clk.strip_suffix("thz") {
                    return max_clk.parse().ok().map(|r: f64| r * 1000.);
                }
                if let Some(max_clk) = max_clk.strip_suffix("ghz") {
                    return max_clk.parse().ok();
                }

                if let Some(max_clk) = max_clk.strip_suffix("mhz") {
                    return max_clk.parse().ok().map(|r: f64| r / 1000.);
                }

                None
            }),
            num_files,
            linux_num_files,
            pxe_linux: parse_bool(pxe_linux.as_deref()),
            dtb: parse_bool(dtb.as_deref()),
        }
    }
}
