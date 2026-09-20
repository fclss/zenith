# sing-box as the core, run as a subprocess

We build the client around sing-box rather than xray, mihomo, or an in-process engine. sing-box wins on first-class TUN support (our flagship mode), clean JSON config, and active maintenance. We run it as a managed child process instead of linking it into our process: the core is Go, so in-process would mean a CGo binding — a build and safety burden — and a subprocess gives us a clean isolation boundary and lets the core keep running after the TUI exits.

Considered: xray (weaker TUN story), mihomo (native Clash YAML, but Clash-shaped config), and CGo in-process bindings (rejected for build complexity and crash-surface). Consequence: everything we need from the core must come over its network API or the filesystem; we never call into it directly.
