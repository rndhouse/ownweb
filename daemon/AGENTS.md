# Daemon Agent Instructions

## Scope

- These rules apply to tasks that change files under `daemon/`.
- These rules are additive with the repository root `AGENTS.md`.

## Validation Gates

- Run `cargo fmt` before commit when Rust files change.
- Run `cargo check` before commit for daemon changes.
- Run `cargo test` when behavior changes or tests exist for the affected code.
- For REST contract changes, smoke-test the changed endpoint with `curl` when practical.

## Runtime Policy

- Keep classification and filtering policy in the daemon.
- Route daemon output through structured `tracing` logs on stdout.
- Keep captured-content logging opt-in through `OWNWEB_LOG_CAPTURED_CONTENT`.
- Bind to `127.0.0.1:17891` by default.
- Use `OWNWEB_BIND_ADDR` for alternate local test ports.

## Rust Policy

- Prefer explicit request and response structs for HTTP JSON payloads.
- Keep serde field names compatible with the extension contract.
- Add focused doc comments where a type or function represents a boundary contract.
- Avoid expensive or network-backed checks unless the task explicitly requires them.
