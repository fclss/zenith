<h1 align="center">
  <img src="docs/assets/zenith.png" alt="" width="32" height="32" /> zenith
</h1>

<div align="center">

<table>
  <tr>
    <th>CI &amp; Quality</th>
    <th>Project Info</th>
  </tr>
  <tr>
    <td>
      <a href="https://github.com/fclss/zenith/actions/workflows/ci.yml">
        <img src="https://github.com/fclss/zenith/actions/workflows/ci.yml/badge.svg" alt="CI" />
      </a><br />
      <a href="https://github.com/fclss/zenith/actions/workflows/release.yml">
        <img src="https://github.com/fclss/zenith/actions/workflows/release.yml/badge.svg" alt="Release" />
      </a>
    </td>
    <td>
      <a href="https://github.com/fclss/zenith/releases/latest">
        <img src="https://img.shields.io/github/v/release/fclss/zenith?color=3775A9" alt="Latest release" />
      </a><br />
      <a href="https://github.com/fclss/zenith/blob/main/LICENSE">
        <img src="https://img.shields.io/github/license/fclss/zenith?color=2E7D32" alt="License" />
      </a><br />
      <img src="https://img.shields.io/badge/rust-1.75%2B-orange?logo=rust" alt="Rust" />
    </td>
  </tr>
</table>

</div>

`zenith` is a Linux terminal UI that routes this machine's traffic through a remote
proxy server. It is a control surface around [sing-box](https://sing-box.sagernet.org/):
zenith owns the subscription, node selection, and lifecycle, while sing-box does the
actual tunnelling.

## Features

- **Two capture modes**
  - **TUN mode** — captures traffic at the network layer through a virtual interface (needs `sudo`).
  - **Proxy mode** — runs a local HTTP/SOCKS port and advertises it to apps (no elevation).
- **Subscriptions** — add, edit, delete, and update them from inside the TUI; the
  client only ever *reads* the subscription JSON, never rewrites it.
- **Node list with latency** — ping every node in parallel and switch the active node live
  through the Clash API.
- **Live traffic** — current/peak/total throughput, with counters you can reset.
- **Detach & re-attach** — quit the TUI and the tunnel keeps running; the next launch
  re-attaches to the live core.
- **Themes from files** — drop a `themes/<name>.yaml` into your config dir.
- **Optional Nerd Font logo** — auto-detected, or forced on/off.
- **System-proxy integration** — writes GNOME `gsettings`, a sourceable `proxy.env`,
  or both, so browsers *and* CLI apps like Telegram pick it up.

## Requirements

- Linux (uses `ip`, `resolvectl`, and optionally `gsettings`).
- [sing-box](https://sing-box.sagernet.org/) on `PATH` (v1.11+; developed against 1.13).
- Rust 1.75+ to build from source.
- For TUN mode: passwordless `sudo` (`sudo -n`) so the core can be spawned elevated.

## Install

### From source

```sh
git clone https://github.com/fclss/zenith
cd zenith
cargo build --release
install -m755 target/release/zenith ~/.local/bin/zenith
```

### Prebuilt binaries

Download a tarball for your architecture from the
[releases page](https://github.com/fclss/zenith/releases), unpack it, and put the
`zenith` binary on your `PATH`.

### Try it without sing-box

```sh
cargo run -- --fake
```

The fake core exercises the whole UI without spawning anything.

## Usage

```sh
zenith              # start the TUI
zenith --fake       # run against the in-process fake core
zenith --proxy-env  # print shell exports (for `eval "$(zenith --proxy-env)"`)
```

### Keys

| Key | Action |
| --- | --- |
| `↑` / `k`, `↓` / `j` | move selection · scroll logs |
| `↵` | connect / disconnect |
| `space` | switch to the selected node |
| `c` / `d` | connect / disconnect |
| `t` | toggle TUN / proxy mode |
| `tab` / `shift+tab` | next / previous subscription |
| `p` | ping all nodes |
| `u` | update the active subscription |
| `z` | reset traffic counters |
| `r` | routing rules |
| `l` | logs |
| `e` | edit the subscription file in `$EDITOR` |
| `s` | subscriptions |
| `?` / `h` | help |
| `q` / `esc` | quit (detaches the tunnel) |

## Configuration

Settings live at `~/.config/zenith/settings.yaml` (or `$XDG_CONFIG_HOME/zenith`):

```yaml
subscriptions:
  - name: work
    update_url: https://example.com/sub
    file: ~/.config/zenith/subscriptions/work.json
    auto_update: true
    default_node: null

default_subscription: work
default_mode: tun            # tun | proxy
system_proxy: true           # advertise the proxy-mode port to apps
proxy_backend: gnome         # gnome | env | both | none
theme: default               # themes/<name>.yaml under the config dir
nerd_fonts: auto             # auto | on | off
delay_refresh_seconds: 300   # auto ping-test interval; 0 disables
```

### Proxy backends

| Value | Effect |
| --- | --- |
| `gnome` | writes GNOME `gsettings` (Firefox, GNOME apps) |
| `env` | writes `~/.config/zenith/proxy.env` to `source` (Telegram, curl, CLI tools) |
| `both` | does both |
| `none` | leaves the system alone (TUN-only setups) |

Many desktop apps read environment variables rather than GNOME settings. With
`env`/`both`, source the file before launching them:

```sh
source ~/.config/zenith/proxy.env
```

### Themes

Create `~/.config/zenith/themes/<name>.yaml` and point `theme:` at it. Every field
is optional and falls back to the built-in default:

```yaml
accent: "#b2a6df"
text: "#ffffff"
dim: darkgray
selected_fg: black
selected_bg: gray
up: "#c8e3ab"
down: "#b2a6df"
ok: "#c8e3ab"
warn: "#b2a6df"
error: "#7d3138"
```

## Architecture

zenith never implements its own tunnel. It manages `sing-box` as a child process and
drives it over its Clash-compatible HTTP API. The whole outside world is behind one
`Core` trait, with a fake implementation for the UI and a real one that spawns
sing-box — this is what makes the TUI testable without a live tunnel.

Design decisions are recorded as ADRs under [`docs/adr/`](docs/adr/).

## Development

```sh
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

Licensed under the [MIT License](LICENSE).
