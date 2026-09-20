# Elevate the core per mode; never the whole app

The TUI always runs as the normal user. Proxy mode spawns the core as that user; TUN mode spawns it elevated (sudo). We accept a process bounce on mode switch. The rationale is attack surface: the TUI parses subscription payloads and renders a full-screen UI — the last components that should run as root — while sing-box is the only thing that actually needs CAP_NET_ADMIN.

Considered: running the whole app as root (rejected: config/state bifurcate into `/root`, and any bug in subscription parsing or rendering becomes a root compromise), setcap on the core binary (rejected: must be re-applied after every core update), and pkexec/polkit (rejected in favour of sudo, which is passwordless on the target system). Consequence: generated config is written by the user and read by a root-spawned core — fine, since root reads anything — and mode switches restart the core process.
