# machine_locks_server

## Configuration

Environment variables can be used to configre properties of this server.
Alternatively, a `.env` file can be used. A `.env.example` file at the root of
this repository provides example values that can be used in a deployment.

Available variables:

- `DATABASE_URL`: A SQLite URL pointing to the desired database location.
  Upon starting the server, this file will be created if missing.
- `DEFAULT_TIMOUT`: _(optional)_: Default timeout for locks in seconds,
  defaulting to 7200s (2 hours).
- `SERVER_POLL_PERIOD`: _(optional)_: The time in seconds the server holds
  open the HTTP long polling connection, defaulting to 30 seconds.

## Deployment

TODO:

```
nix run au-ts/machine-locks#server
```

Proxy port 3000.
