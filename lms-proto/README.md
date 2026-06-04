# lms-proto

A minimal Rust crate for building clients that interact with Logitech Media
Server (LMS) over the Slim Protocol.

This crate provides the core protocol primitives required to:

- discover LMS instances on the local network
- perform the HELO handshake with a server
- send and receive framed Slim Protocol messages
- manage client status updates and capabilities

## Features

- `Hello` builder for initial LMS connection setup
- `discover()` helper for LAN server discovery
- `LmsRecv` and `LmsSend` framed transport helpers
- `ClientMessage` and `ServerMessage` enums for protocol messages
- `StatusData` helpers for LMS status reporting
- `SlimBuffer` for buffered audio reads with status tracking

## Usage

Add `lms-proto` to your `Cargo.toml`:

```toml
[dependencies]
lms-proto = "0.1"
```

Example client connection:

```no_run
use lms_proto::{Hello, SLIM_PORT};

let hello = Hello::new()
    .device_id(1)
    .revision(0)
    .language(['e', 'n'])
    .capabilities(vec![lms_proto::Capability::Pcm]);

let (mut rx, mut tx) = hello.connect(("127.0.0.1", SLIM_PORT)).unwrap();

// Now use `tx.send(...)` and `rx.recv()` to communicate with the server.
```

Discovery example:

```no_run
use std::time::Duration;
use lms_proto::discover;

let server = discover(Some(Duration::from_secs(2))).unwrap();
println!("Discovery result: {:?}", server);
```

## Documentation

For full API details, see the generated docs on [docs.rs](https://docs.rs/lms-proto).

[![MIT licensed][mit-badge]][mit-url]
[![Crate](https://img.shields.io/crates/v/lms-proto.svg)](https://crates.io/crates/lms-proto)
[![GitHub last commit](https://img.shields.io/github/last-commit/GeoffClements/lms-player.svg)][github]
[![Build Status][actions-badge]][actions-url]

[mit-badge]: https://img.shields.io/badge/license-MIT-blue.svg
[mit-url]: https://github.com/GeoffClements/slim-client-protocol-rs/blob/master/LICENSE.txt
[github]: https://github.com/GeoffClements/lms-player
[actions-badge]: https://github.com/GeoffClements/lms-player/actions/workflows/feature_build.yml/badge.svg
[actions-url]: https://github.com/GeoffClements/lms-player/actions?query=workflow%3Arust+branch%3Amaster
