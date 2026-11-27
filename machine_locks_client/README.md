# machine-locks-client

## Installation

### With Nix

Create a new shell with the `machine-lock-client` in your PATH with:

```bash
nix shell github:au-ts/machine-locks#client
```

### From pre-built binary

### Building from source

Make sure you have Rust and Cargo installed. If you don't, install them via [rustup](https://rustup.rs).

Clone this repo:

```bash
git clone https://github.com/au-ts/machine-locks.git
```

Then run:

```bash
cargo build --release -p machine-locks-client
```

And copy the resulting `target/release/machine-locks-client` binary to somewhere in your PATH.

## Configuration

Environment variables can be used to configure properties of this server.
Available variables:

- `MACHINE_LOCKS_BASE_URL` _(optional)_: Base URL to make lock API requests to,
  must include a trailing slash.
- `MACHINE_LOCKS_ISSUER_URL` _(optional)_: OIDC issuer URL for authentication.
- `MACHINE_LOCKS_CLIENT_ID` _(optional)_: OIDC client ID for authentication.
- `RUST_LOG` _(optional)_: Log verbosity, one of:
  - error (default)
  - warn
  - info
  - debug
  - trace
  - off

    See [env_logger](https://docs.rs/env_logger/latest/env_logger/)
    documentation for more configuration options.
