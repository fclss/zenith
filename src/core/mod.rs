pub mod fake;
pub mod singbox;

use std::fmt;
use std::sync::mpsc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::settings::SubscriptionEntry;

/// How the tunnel captures traffic. Two modes only (TUN, proxy).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Tun,
    Proxy,
}

impl Serialize for Mode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Mode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Mode, D::Error> {
        let s = String::deserialize(deserializer)?;
        Mode::from_str(&s).ok_or_else(|| serde::de::Error::custom(format!("invalid mode: {}", s)))
    }
}

impl Mode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Mode::Tun => "tun",
            Mode::Proxy => "proxy",
        }
    }

    pub fn from_str(s: &str) -> Option<Mode> {
        match s {
            "tun" => Some(Mode::Tun),
            "proxy" => Some(Mode::Proxy),
            _ => None,
        }
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Mode {
    type Err = ();
    fn from_str(s: &str) -> Result<Mode, ()> {
        Mode::from_str(s).ok_or(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TunnelState {
    Down,
    Up(Mode),
}

impl TunnelState {
    pub fn is_up(&self) -> bool {
        matches!(self, TunnelState::Up(_))
    }

    #[cfg(test)]
    pub fn mode(&self) -> Option<Mode> {
        match self {
            TunnelState::Up(mode) => Some(*mode),
            TunnelState::Down => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub name: String,
    pub delay: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RulePolicy {
    Proxied,
    Direct,
}

impl RulePolicy {
    pub fn as_str(&self) -> &'static str {
        match self {
            RulePolicy::Proxied => "proxied",
            RulePolicy::Direct => "direct",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub name: String,
    pub policy: RulePolicy,
}

/// Keep `active` if it still names a node in `nodes`; otherwise fall back to
/// the first node (or `None` for an empty list). Shared by both core
/// implementations so an out-of-date active node never survives an update.
pub fn reconcile_active(nodes: &[Node], active: &Option<String>) -> Option<String> {
    match active {
        Some(name) if nodes.iter().any(|n| &n.name == name) => Some(name.clone()),
        _ => nodes.first().map(|n| n.name.clone()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrafficSample {
    pub up: u64,
    pub down: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreError(pub String);

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CoreError {}

impl From<String> for CoreError {
    fn from(s: String) -> Self {
        CoreError(s)
    }
}

impl From<&str> for CoreError {
    fn from(s: &str) -> Self {
        CoreError(s.to_string())
    }
}

/// The single seam of the application (spec "Testing Decisions"). Everything the
/// client does with the world outside the TUI flows through this trait: the
/// sing-box process, the Clash-compatible API, subscription update, and
/// detach/re-attach. A fake implementation emulates the whole domain as state;
/// the real implementation drives sing-box.
///
/// Implementations use interior mutability so the TUI and background worker
/// threads can share an `Arc<dyn Core>`.
pub trait Core: Send + Sync {
    /// Point the core at a subscription: its file becomes the source of nodes
    /// and rules. Does not start the tunnel.
    fn set_subscription(&self, entry: &SubscriptionEntry) -> Result<(), CoreError>;

    /// Re-fetch the subscription and replace its node list. Local-only
    /// subscriptions (no update URL) are a no-op that reports success.
    fn update(&self) -> Result<(), CoreError>;

    /// The current node list, in order, with any measured delays.
    fn nodes(&self) -> Vec<Node>;

    /// The selector's active node, if the tunnel has one.
    fn active_node(&self) -> Option<String>;

    /// Make `name` the active node in the selector. Fails if unknown.
    fn switch_node(&self, name: &str) -> Result<(), CoreError>;

    /// Measure delay for every node and record it on the node list.
    fn test_delays(&self) -> Result<(), CoreError>;

    /// Start the tunnel in the given mode.
    fn connect(&self, mode: Mode) -> Result<(), CoreError>;

    /// Stop the tunnel.
    fn disconnect(&self) -> Result<(), CoreError>;

    /// Whether the tunnel is up, and in which mode.
    fn tunnel_state(&self) -> TunnelState;

    /// The routing rules carried by the current subscription.
    fn rules(&self) -> Vec<Rule>;

    /// Re-read the subscription file and apply changes to the live core
    /// without dropping the tunnel.
    fn reload(&self) -> Result<(), CoreError>;

    /// Open `path` in the user's editor (`$EDITOR`, falling back to `vi`).
    fn edit_file(&self, path: &std::path::Path) -> Result<(), CoreError>;

    /// Whether a Nerd Font is installed, used to decide the header logo. A
    /// best-effort probe — a terminal cannot be asked which font it renders.
    fn nerd_font_available(&self) -> bool;

    /// A stream of traffic samples, available after connect. Consumed by the
    /// traffic graph.
    fn traffic_rx(&self) -> Option<mpsc::Receiver<TrafficSample>>;

    /// Leave the core and tunnel running after the TUI exits.
    fn detach(&self) -> Result<(), CoreError>;

    /// Probe for a live tunnel left by a previous run; if found, adopt it.
    fn reattach(&self) -> Result<bool, CoreError>;

    /// Advertise the proxy-mode port as the system proxy (US19). Remembers
    /// the previous values so `clear_system_proxy` can restore them. A no-op
    /// when system proxy support is unavailable (non-GNOME, no gsettings).
    fn apply_system_proxy(&self, backend: crate::settings::ProxyBackend) -> Result<(), CoreError>;

    /// Restore the system-proxy values captured by `apply_system_proxy`.
    fn clear_system_proxy(&self) -> Result<(), CoreError>;

    /// The last N lines of the core's stderr log, ANSI stripped.
    fn log_content(&self) -> String;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_roundtrips_through_string() {
        assert_eq!(Mode::from_str("tun"), Some(Mode::Tun));
        assert_eq!(Mode::from_str("proxy"), Some(Mode::Proxy));
        assert_eq!(Mode::from_str("bogus"), None);
        assert_eq!(Mode::Tun.to_string(), "tun");
    }

    #[test]
    fn tunnel_state_reports_mode() {
        assert_eq!(TunnelState::Down.mode(), None);
        assert!(!TunnelState::Down.is_up());
        assert_eq!(TunnelState::Up(Mode::Proxy).mode(), Some(Mode::Proxy));
        assert!(TunnelState::Up(Mode::Proxy).is_up());
    }
}
