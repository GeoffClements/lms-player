# lms-player Workspace

This repository contains a Rust workspace for the `lms-player` project. It is
organized as a multi-crate workspace with the following members:

- `lms-proto` — a small Rust crate implementing core Slim Protocol client
  primitives for Logitech Media Server (LMS).
- `vibe` — a command-line music player built on top of `lms-proto` that connects
  to LMS servers and plays audio through local audio backends.

## Workspace structure

- `Cargo.toml` — workspace manifest that defines the member crates.
- `lms-proto/` — protocol library crate for LMS discovery, HELO handshakes,
  framed messaging, and status reporting.
- `vibe/` — binary crate for the Vibe music player and runtime daemon.

## Building

Build the entire workspace with:

```bash
cargo build --workspace
```

Build only the player binary:

```bash
cargo build -p Vibe_Player
```

Build only the protocol crate:

```bash
cargo build -p lms-proto
```

## Testing

Run all workspace tests:

```bash
cargo test --workspace
```

Run tests for a single package:

```bash
cargo test -p lms-proto
```

## Running Vibe

To run the `vibe` command from the workspace:

```bash
cargo run -p Vibe_Player -- --help
```

The `vibe` crate supports multiple audio backend features. By default it builds
with `pulse`. Use feature flags to select other systems:

```bash
cargo run -p Vibe_Player --features pipewire -- --help
```

## Documentation

- `lms-proto` documentation: `lms-proto/README.md`
- `vibe` documentation: `vibe/README.md`

## License

This workspace uses the MIT license. See the individual package manifests for
package-specific metadata.
