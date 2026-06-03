# lms-proto

A crate for building clients to connect to a [Lyrion Music Server][`lms`]

The [Slim Protocol][slimtcpwiki] is a TCP protocol for streaming audio files
to a [slim device][slimdevices].

This crate simplifies writing of a client for this protocol by providing a
library that sends and receives messages to a slim server.

[slimtcpwiki]: https://lyrion.org/reference/slimproto-protocol/
[slimdevices]: https://en.wikipedia.org/wiki/Slim_Devices
[`lms`]: https://lyrion.org/

## License

This project is licensed under the [MIT license].

[MIT license]: https://github.com/GeoffClements/slim-client-protocol-rs/blob/master/LICENSE.txt

[![MIT licensed][mit-badge]][mit-url]
[![Crate](https://img.shields.io/crates/v/lms-proto.svg)](https://crates.io/crates/lms-proto)
[![GitHub last commit](https://img.shields.io/github/last-commit/GeoffClements/lms-player.svg)][github]
[![Build Status][actions-badge]][actions-url]

[mit-badge]: https://img.shields.io/badge/license-MIT-blue.svg
[mit-url]: https://github.com/slimproto/blob/master/LICENSE
[github]: https://github.com/GeoffCLements/lms-player
[actions-badge]: https://github.com/GeoffClements/lms-player/actions/workflows/feature_build.yml/badge.svg
[actions-url]: https://github.com/GeoffClements/lms-player/actions?query=workflow%3Arust+branch%3Amaster
