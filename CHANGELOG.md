# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- TUI shell with connect/disconnect, mode switching, and a live header.
- Two capture modes: TUN (elevated) and proxy (local mixed port).
- Subscription registry with in-app add/edit/delete and update.
- Node list with parallel latency probing and live node switching.
- Traffic counters (current / peak / total) with reset.
- Detach on quit and re-attach on launch.
- Themes loaded from `themes/<name>.yaml`.
- Optional Nerd Font logo with auto-detection.
- System-proxy integration via GNOME `gsettings` and/or a sourceable
  `proxy.env` (`proxy_backend: gnome | env | both | none`).
- `zenith --proxy-env` to print the proxy exports.
- `zenith --fake` to run the UI against the in-process fake core.

[Unreleased]: https://github.com/fclss/zenith/commits/main
