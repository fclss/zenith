# Zenith

A Linux TUI client that routes the user's traffic through a remote proxy server. Built on an existing proxy engine ("the core") rather than implementing its own tunnelling. The product is colloquially called a "VPN client", but the mechanism is proxying, not a site-to-site VPN.

## Language

**VPN**:
Colloquial product name. Means "route this machine's traffic through a remote server". The actual mechanism is a proxy client, not a site-to-site/remote-access VPN.
_Avoid_: using "VPN" in code, issue titles, or docs to mean the tunnelling mechanism itself.

**Core**:
The proxy engine that does the actual tunnelling. A fixed, externally-provided binary (sing-box) managed as a child process by this client. This project builds a control surface around it, never its own tunnel.
_Avoid_: engine, proxy server (that's the remote host), VPN backend.

**Node**:
A single remote proxy server this client can connect through, described by its connection params (protocol, address, credentials). Nodes come from subscriptions and are the unit the user selects.
_Avoid_: server, endpoint, profile, proxy (in this context).

**Subscription**:
A named entry in the settings file pointing at a local subscription file (a sing-box JSON config carrying node outbounds, routing rules, rule-sets, and DNS). It may have an update URL (absent for purely local subscriptions) and an auto-update flag. The only operations on a subscription are update and manual editing by the user in their own editor — the client never edits it.
_Avoid_: source, feed, collection.

**Selector**:
The outbound group, injected by the client and named `proxy`, that holds the current node list — exactly one node active ("now"). Routing rules in the subscription reference this group. The client switches the active node by telling the core to change the selector.
_Avoid_: group, node list, rotation.

**Rule**:
A predicate on captured traffic (domain, IP, geoip, port) that decides where it goes: through the tunnel or direct. Rules are carried by the subscription and owned by the user's editor, never by the client.
_Avoid_: filter, policy, entry.

**Rule-set**:
A named, addressable collection of rule data (geosite/geoip sets, ad-block lists) that rules reference by name.
_Avoid_: list, blocklist, category.

**Direct**:
The destination for traffic that does not go through the tunnel — straight out the local network. The counterpart of "proxied".
_Avoid_: bypass, route-through-network.

**Tunnel**:
The live routing state created when the core is connected: this machine's captured traffic flowing through the active node. "Connected" and "disconnected" refer to the tunnel being on or off.
_Avoid_: connection, session, pipe.

**Mode**:
Whether traffic is captured at the network layer or handed to apps through a local proxy port. Two modes, set per tunnel:
**TUN mode**: traffic is captured at the network layer via a virtual interface (requires elevation).
**Proxy mode**: the core listens on a local HTTP/SOCKS port that apps are pointed at (no elevation).

**Connect**:
The operation that makes the tunnel live: start the core and route traffic through the node the selector points at.
_Avoid_: start, turn on, go.

**Disconnect**:
The operation that ends the tunnel: stop the core and remove its routing.
_Avoid_: stop, turn off.

**Update**:
The operation that re-fetches the subscription URL and replaces the local subscription file. The only subscription operation besides manual editing.
_Avoid_: refresh, sync, pull.

**Detach**:
Quitting the client while leaving the core and tunnel running.
_Avoid_: background, minimize, close.

**Re-attach**:
Launching the client and resuming control of a still-running core and tunnel left by a previous detach.
_Avoid_: resume, reconnect (that's for a dropped tunnel), rejoin.

**Delay**:
A node's measured round-trip latency, obtained by probing the node on demand. Shown per node in the node list and used by the user to pick a node.
_Avoid_: ping, latency (keep as the raw measurement only), response time.

**Traffic**:
Throughput through the tunnel, shown as a live tx/rx graph in the header.
_Avoid_: bandwidth, net.
