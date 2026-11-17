-- SPDX-FileCopyrightText: 2026 UNSW
--
-- SPDX-License-Identifier: BSD-2-Clause

PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS machines (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP NOT NULL,
    available BOOLEAN NOT NULL,  -- used to mark a board as down for maintenance
    name TEXT NOT NULL,
    -- ipbench boards do not have any of the below columns
    poweroff_name TEXT,
    arch TEXT,
    sel4_plat TEXT,
    isa TEXT,
    soc TEXT,
    cpu TEXT,
    cores TEXT, -- text because there can be notation like "4+2" or "4 (8)"
    ram INTEGER, -- in MB
    max_clk REAL, -- in GHz
    num_files INTEGER,
    linux_num_files INTEGER,
    pxe_linux BOOLEAN NOT NULL DEFAULT FALSE CHECK (pxe_linux in (FALSE, TRUE)),
    dtb BOOLEAN NOT NULL DEFAULT FALSE CHECK (pxe_linux in (FALSE, TRUE))
);

-- Needed because machines <-> pools is a many to many relation
CREATE TABLE IF NOT EXISTS pool_memberships (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP NOT NULL,
    machine_id INTEGER NOT NULL REFERENCES machines (id) ON DELETE CASCADE,
    pool_id INTEGER NOT NULL REFERENCES pools (id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS pools (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP NOT NULL,
    available BOOLEAN NOT NULL,  -- used to mark a pool as down for maintenance
    name TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS jobs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP NOT NULL,
    acquired_lock_at DATETIME, -- null until lock acquired
    released_lock_at DATETIME, -- null until lock released
    timeout INTEGER, -- TODO: configurable default timeout
    priority INTEGER NOT NULL DEFAULT 0, -- task priority, larger is higher priority
    job_name TEXT, -- equivalent of MQ's `key` field.
    owner TEXT NOT NULL, -- POSIX user who owns the lock
    machine_id INTEGER NOT NULL REFERENCES machines (id) ON DELETE CASCADE
);
