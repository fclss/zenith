# Quit detaches; no daemon

Quitting the TUI leaves the core and tunnel running (detach). On the next launch the client re-attaches by connecting to the live core's API and reconstructing state (active node, mode) from it. We ship no background daemon — the orphaned sing-box process *is* the persistence, and a small state file records where to find it.

Considered: a `zenithd` daemon owning the core (rejected: a second binary, supervision, and autostart units for a benefit we get for free) and quit-kills-the-tunnel (rejected: closing a terminal should not silently drop a tunnel the user is relying on). Consequence: the client's persisted state is thin; the live core is the source of truth for what's connected, so re-attach reads from the core, not from client-side snapshots.
