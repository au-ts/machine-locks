// SPDX-FileCopyrightText: 2026 UNSW
//
// SPDX-License-Identifier: BSD-2-Clause

use std::{env, path::PathBuf, thread::sleep, time::Duration};

use anyhow::{Result, anyhow, bail};
use clap::{Parser, Subcommand};
use common::{
    Job, Machine,
    PollStatus::{self, JobReady},
};
use log::info;
use reqwest::{
    Url,
    blocking::{Client, RequestBuilder},
};
use serde::Serialize;
use serde_json::{Value, json};
use tabled::{
    Table,
    settings::{Color, Style, object::Rows},
};

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
}

#[derive(Subcommand)]
enum SemCommands {
    /// Print all currently locked systems
    Dumpall,
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
}

#[derive(Subcommand)]
enum MachineCommands {
    /// List all machines
    List,
    /// Import machines from a TSV file
    ImportTsv {
        file: PathBuf,
        #[arg(long)]
        /// Whether to overwrite existing machines that are also in the TSV file
        overwrite: bool,
    },
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

    fn acquire(
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

            if let JobReady(job) = status {
                return Ok(job);
            }
        }

        bail!("Failed to acquire lock within specified retries");
    }

    fn release(&self, machine_id: i64, owner: &str, force: bool) -> Result<()> {
        let jobs = self.get_machine_jobs(machine_id)?;

        match jobs.first() {
            Some(active_job) if active_job.acquired_lock_at.is_some() => {
                if force || active_job.owner == owner {
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
}

pub fn dump_json<T>(value: &T) -> Result<(), serde_json::Error>
where
    T: ?Sized + Serialize,
{
    serde_json::to_writer(std::io::stdout().lock(), &value)?;
    println!();
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let client = ApiClient::new()?;

    // TODO: human readable format
    match cli.command {
        Commands::Sem {
            command:
                SemCommands::Info {
                    machine,
                    json,
                    poll,
                },
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
        Commands::Sem {
            command:
                SemCommands::Acquire {
                    machine,
                    owner,
                    job_name,
                    priority,
                    no_timeout,
                    timeout,
                    num_retries,
                },
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
        Commands::Sem {
            command:
                SemCommands::Release {
                    machine,
                    owner,
                    force,
                },
        } => {
            let machine = client.get_machine_by_name(&machine)?;
            client.release(machine.id, &owner, force)?;
        }
        _ => todo!(),
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
