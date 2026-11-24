# machine-locks-server

`cargo sqlx prepare --workspace` should be run after any changes to the queries
to have `sqlx` generate query metadata to use in building the project.

## Configuration

Environment variables can be used to configure properties of this server.
Alternatively, a `.env` file can be used. A `.env.example` file at the root of
this repository provides example values that can be used in a deployment.

Available variables:

- `DATABASE_URL`: A SQLite URL pointing to the desired database location. Upon
  starting the server, this file will be created if missing.
- `DEFAULT_TIMEOUT` _(optional)_: Default timeout for locks in seconds,
  defaulting to 7200s (2 hours).
- `SERVER_POLL_PERIOD` _(optional)_: The time in seconds the server holds open
  the HTTP long polling connection, defaulting to 30 seconds.
- `RUST_LOG` _(optional)_: Log verbosity, one of:
  - error (default)
  - warn
  - info
  - debug
  - trace
  - off

    See [env_logger](https://docs.rs/env_logger/latest/env_logger/)
    documentation for more configuration options.

## Deployment

Run a server ad-hoc with:

```bash
RUST_LOG=info nix run github:au-ts/machine-locks#server
```

Proxy port 3000.
