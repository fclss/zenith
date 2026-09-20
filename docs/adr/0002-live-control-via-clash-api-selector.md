# Live control via the Clash-compatible API and an injected selector

We spawn the core once with a config that has all nodes as outbounds and a selector group named `proxy`, then control it at runtime through sing-box's Clash-compatible HTTP API — switching the active node is one call, never a process restart. The TUI injects the selector itself so live switching is never hostage to what a subscription's provider named its groups.

Considered: regenerate-and-restart per switch (rejected: every switch would flap the TUN interface and drop the tunnel) and sing-box's native JSON-RPC API (less well-trodden). Consequence: the selector is a fixed contract — the TUI guarantees a group named `proxy` exists and fills it with the subscription's outbounds; the subscription's rules reference that name.
