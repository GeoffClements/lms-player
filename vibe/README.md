![Static Badge](https://img.shields.io/badge/Written%20in-Rust-blue?style=flat&logo=rust)
![Crates.io Version](https://img.shields.io/crates/v/Vibe_Player)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
![push workflow](https://github.com/GeoffClements/Vibe/actions/workflows/feature_build.yml/badge.svg)
![Static Badge](https://img.shields.io/badge/Feel%20the-Vibe-red?style=plastic)


# Vibe

## About
Vibe is a music player that uses the [SLIM TCP protocol][`slimtcp`] to 
connect to a [Lyrion Music Server][`lms`], formally known as a
Logitech Media Server.

Vibe is intended to be run as a user daemon and is designed to be simple and
as unobtrusive as possible. It's possible to start Vibe at login
using a systemd service file and this can be generated automatically.

It's possible to use auto-discovery to find the Lyrion Music Server
or, in cases where this is not possible, like when using a `tailscale` VPN,
then a server can be specified either by its domain name or its IP address.

The output device can be selected, in this case it's helpful to list 
the devices first and use the appropriate device name from the list.

Desktop notifications can be displayed when a new track starts,
although this needs to be enabled at compile time. Please note that
the Lyrion Music Server does not send metadata for mp3 streams,
and so no notifications will be produced when playing an mp3.

## Running
To list the run-time options:
```
vibe -h
```

To see all audio output devices on your machine:
```
vibe -l
```

## Automatic starting at login
Vibe can create a systemd user service file for you using
```
vibe --create-service
```

If you want either a Lyrion server, 
or an output device, or both to be specified in the service
file then just use the `--server` and `--device` options
, e.g.

```
vibe --create-service --server my.lyrion.server --device my.output.device
 
```

This will create a systemd service file under
`${HOME}/.config/systemd/user`. Once created,
tell systemd of the new service with

```bash
systemctl --user daemon-reload
```

Start the service with

```bash
systemctl --user start vibe.service
```

You can make it so that vibe will start whenever you login with

```bash
systemctl --user enable vibe.service
```

## Building

Vibe can be built with the ability to connect to different audio
systems using compile-time features. These features are:

- `pulse` for Pulseaudio (the default)
- `pipewire` for Pipewire
- `rodio` for ALSA

Note that if `pulse` is selected then it is still possible to 
play audio on Pipewire systems because Pipewire implements the
Pulseaudio API as well as its own.

It is possible to compile for one, two or all three of the audio
systems by selecting the appropriate features at compile time. If
more than one of these features are selected then a run-time
command switch ("--system" or "-a") can be used to select
which audio system should be used.

At least one of these features **must** be selected, noting that
`pulse` is normally selected by default. This default can be switched off
by using `--no-default-features` when building.

Each audio system has its own build-time dependencies and the 
appropriate packages must be on the development system.

### Pulse
- Pulseaudio development files

### Pipewire
- Pipewire development files
- SPA development files
- clang development files

### Rodio
- ALSA development files

## Other Build-time Features

### Notify
To enable new track notifications on the desktop use the `notify`
feature. If this is enabled, you will be able to suppress the
notifications at run time with the `--quiet` command option.

Build-time dependencies are:

- DBus development files
- `pkg-config`

## Run-time Dependencies
Vibe has zero run-time dependencies, all the stream
demultiplexing and decoding is done natively thanks to 
[Symphonia][`symphonia`], a big "thank-you" to the Symphonia devs for their
amazing work!

## A Note About The Name
I named this project `Vibe` a few years ago, before AI was a mainstream thing.
I thought it sounded cool and was short enough to be a command line instruction.
Since then "vibe coding" has become a thing where, I believe, so called "vibe coders"
instruct an AI to code for them. I would like to state now that I do not use
vibe coding in this project, all code is human-generated and as such, I take
responsibility for all mistakes in, and short-comings of the code.

## Support me
Vibe runs on coffee!
[![ko-fi](https://ko-fi.com/img/githubbutton_sm.svg)](https://ko-fi.com/V7V719WYF6)

[`slimtcp`]: https://lyrion.org/reference/slimproto-protocol/
[`lms`]: https://lyrion.org/
[`squeezelite`]: https://github.com/ralph-irving/squeezelite
[`symphonia`]: https://crates.io/crates/symphonia
[`rust`]: https://www.rust-lang.org/
[`rodio`]: https://crates.io/crates/rodio
[`cpal`]: https://crates.io/crates/cpal