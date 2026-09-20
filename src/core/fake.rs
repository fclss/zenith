use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use crate::core::{Core, CoreError, Mode, Node, Rule, RulePolicy, TrafficSample, TunnelState};
use crate::settings::SubscriptionEntry;
use crate::subscription;

/// The fake core's world: all the state that would live in a real sing-box
/// process. An `Arc<Mutex<FakeWorld>>` is shared between a "relaunched" fake
/// core so re-attach is testable without a real process.
#[derive(Debug)]
pub struct FakeWorld {
    pub nodes: Vec<Node>,
    pub active: Option<String>,
    pub tunnel: TunnelState,
    pub rules: Vec<Rule>,
    pub traffic_tx: Option<mpsc::Sender<TrafficSample>>,
    pub detached: bool,
}

impl Default for FakeWorld {
    fn default() -> Self {
        FakeWorld {
            nodes: vec![
                Node {
                    name: "node-a".to_string(),
                    delay: None,
                },
                Node {
                    name: "node-b".to_string(),
                    delay: None,
                },
                Node {
                    name: "node-c".to_string(),
                    delay: None,
                },
            ],
            active: None,
            tunnel: TunnelState::Down,
            rules: vec![
                Rule {
                    name: "adblock".to_string(),
                    policy: RulePolicy::Direct,
                },
                Rule {
                    name: "streaming".to_string(),
                    policy: RulePolicy::Proxied,
                },
            ],
            traffic_tx: None,
            detached: false,
        }
    }
}

pub struct FakeCore {
    world: Arc<Mutex<FakeWorld>>,
    sub: Mutex<Option<SubscriptionEntry>>,
    traffic_rx: Mutex<Option<mpsc::Receiver<TrafficSample>>>,
    update_count: AtomicU64,
    delay_test_count: AtomicU64,
}

impl FakeCore {
    pub fn new(world: Arc<Mutex<FakeWorld>>) -> Self {
        FakeCore {
            world,
            sub: Mutex::new(None),
            traffic_rx: Mutex::new(None),
            update_count: AtomicU64::new(0),
            delay_test_count: AtomicU64::new(0),
        }
    }

    /// Number of times `test_delays` has run — lets tests prove the UI does not
    /// re-run the whole ping pass on its own thread.
    #[cfg(test)]
    pub fn delay_test_count(&self) -> u64 {
        self.delay_test_count.load(Ordering::SeqCst)
    }

    /// A fake core with a fresh world.
    pub fn fresh() -> Self {
        Self::new(Arc::new(Mutex::new(FakeWorld::default())))
    }

    #[cfg(test)]
    pub fn world(&self) -> Arc<Mutex<FakeWorld>> {
        Arc::clone(&self.world)
    }

    fn canned_nodes(&self, name: &str, count: u64) -> Vec<Node> {
        (0..3)
            .map(|i| Node {
                name: format!("{name}-node-{i}-v{count}"),
                delay: None,
            })
            .collect()
    }
}

impl Core for FakeCore {
    fn set_subscription(&self, entry: &SubscriptionEntry) -> Result<(), CoreError> {
        let parsed = if entry.file.exists() {
            subscription::parse_file(&entry.file)?
        } else {
            let nodes = self.canned_nodes(&entry.name, 0);
            let rules = self.world.lock().unwrap().rules.clone();
            subscription::ParsedSubscription { nodes, rules }
        };
        let mut world = self.world.lock().unwrap();
        world.nodes = parsed.nodes;
        world.rules = parsed.rules;
        world.active = entry
            .default_node
            .clone()
            .filter(|name| world.nodes.iter().any(|n| &n.name == name));
        if world.active.is_none() {
            world.active = world.nodes.first().map(|n| n.name.clone());
        }
        *self.sub.lock().unwrap() = Some(entry.clone());
        Ok(())
    }

    fn update(&self) -> Result<(), CoreError> {
        let entry = self.sub.lock().unwrap().clone();
        let Some(entry) = entry else {
            return Err(CoreError("no subscription loaded".to_string()));
        };
        if entry.update_url.is_none() {
            return Ok(());
        }
        let count = self.update_count.fetch_add(1, Ordering::SeqCst);
        let mut world = self.world.lock().unwrap();
        world.nodes = self.canned_nodes(&entry.name, count + 1);
        if let Some(active) = &world.active {
            if !world.nodes.iter().any(|n| &n.name == active) {
                world.active = world.nodes.first().map(|n| n.name.clone());
            }
        }
        Ok(())
    }

    fn nodes(&self) -> Vec<Node> {
        self.world.lock().unwrap().nodes.clone()
    }

    fn active_node(&self) -> Option<String> {
        self.world.lock().unwrap().active.clone()
    }

    fn switch_node(&self, name: &str) -> Result<(), CoreError> {
        let mut world = self.world.lock().unwrap();
        if !world.nodes.iter().any(|n| n.name == name) {
            return Err(CoreError(format!("unknown node: {}", name)));
        }
        world.active = Some(name.to_string());
        Ok(())
    }

    fn test_delays(&self) -> Result<(), CoreError> {
        self.delay_test_count.fetch_add(1, Ordering::SeqCst);
        let mut world = self.world.lock().unwrap();
        for (i, node) in world.nodes.iter_mut().enumerate() {
            node.delay = Some(40 + (i as u64) * 15);
        }
        Ok(())
    }

    fn connect(&self, mode: Mode) -> Result<(), CoreError> {
        let mut world = self.world.lock().unwrap();
        if world.tunnel.is_up() {
            return Err(CoreError("tunnel already up".to_string()));
        }
        world.tunnel = TunnelState::Up(mode);
        if world.active.is_none() {
            world.active = world.nodes.first().map(|n| n.name.clone());
        }
        let (tx, rx) = mpsc::channel();
        world.traffic_tx = Some(tx);
        *self.traffic_rx.lock().unwrap() = Some(rx);
        let world = Arc::clone(&self.world);
        std::thread::spawn(move || {
            let mut n = 0u64;
            loop {
                std::thread::sleep(Duration::from_millis(50));
                let live = {
                    let w = world.lock().unwrap();
                    w.tunnel.is_up() && w.traffic_tx.is_some()
                };
                if !live {
                    break;
                }
                n += 1024;
                if let Some(tx) = world.lock().unwrap().traffic_tx.as_ref() {
                    let _ = tx.send(TrafficSample { up: n, down: n * 3 });
                }
            }
        });
        Ok(())
    }

    fn disconnect(&self) -> Result<(), CoreError> {
        let mut world = self.world.lock().unwrap();
        world.tunnel = TunnelState::Down;
        world.traffic_tx = None;
        world.detached = false;
        Ok(())
    }

    fn tunnel_state(&self) -> TunnelState {
        self.world.lock().unwrap().tunnel
    }

    fn rules(&self) -> Vec<Rule> {
        self.world.lock().unwrap().rules.clone()
    }

    fn edit_file(&self, _path: &std::path::Path) -> Result<(), CoreError> {
        Ok(())
    }

    fn nerd_font_available(&self) -> bool {
        false
    }

    fn reload(&self) -> Result<(), CoreError> {
        let entry = self.sub.lock().unwrap().clone();
        let Some(entry) = entry else {
            return Err(CoreError("no subscription loaded".to_string()));
        };
        let parsed = subscription::parse_file(&entry.file)?;
        let mut world = self.world.lock().unwrap();
        world.nodes = parsed.nodes;
        world.rules = parsed.rules;
        if let Some(active) = &world.active {
            if !world.nodes.iter().any(|n| &n.name == active) {
                world.active = world.nodes.first().map(|n| n.name.clone());
            }
        }
        Ok(())
    }

    fn traffic_rx(&self) -> Option<mpsc::Receiver<TrafficSample>> {
        self.traffic_rx.lock().unwrap().take()
    }

    fn detach(&self) -> Result<(), CoreError> {
        let mut world = self.world.lock().unwrap();
        world.detached = true;
        Ok(())
    }

    fn reattach(&self) -> Result<bool, CoreError> {
        let world = self.world.lock().unwrap();
        Ok(world.tunnel.is_up())
    }

    fn apply_system_proxy(&self, _backend: crate::settings::ProxyBackend) -> Result<(), CoreError> {
        Ok(())
    }

    fn clear_system_proxy(&self) -> Result<(), CoreError> {
        Ok(())
    }

    fn log_content(&self) -> String {
        String::from("[fake] no logs")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_then_disconnect_flips_tunnel_state() {
        let core = FakeCore::fresh();
        assert_eq!(core.tunnel_state(), TunnelState::Down);
        core.connect(Mode::Tun).unwrap();
        assert_eq!(core.tunnel_state(), TunnelState::Up(Mode::Tun));
        core.disconnect().unwrap();
        assert_eq!(core.tunnel_state(), TunnelState::Down);
    }

    #[test]
    fn connecting_twice_is_an_error() {
        let core = FakeCore::fresh();
        core.connect(Mode::Proxy).unwrap();
        assert!(core.connect(Mode::Proxy).is_err());
    }

    #[test]
    fn connect_picks_active_node_when_none_selected() {
        let core = FakeCore::fresh();
        core.connect(Mode::Tun).unwrap();
        assert_eq!(core.active_node().unwrap(), "node-a");
    }

    #[test]
    fn switch_node_changes_selector_active() {
        let core = FakeCore::fresh();
        core.switch_node("node-b").unwrap();
        assert_eq!(core.active_node().unwrap(), "node-b");
        assert!(core.switch_node("nope").is_err());
    }

    #[test]
    fn test_delays_records_deterministic_values() {
        let core = FakeCore::fresh();
        core.test_delays().unwrap();
        let nodes = core.nodes();
        assert_eq!(nodes[0].delay, Some(40));
        assert_eq!(nodes[1].delay, Some(55));
        assert_eq!(nodes[2].delay, Some(70));
    }

    #[test]
    fn update_replaces_nodes() {
        let core = FakeCore::fresh();
        let entry = SubscriptionEntry {
            name: "work".into(),
            update_url: Some("https://example.com/sub".into()),
            file: "/nonexistent/work.json".into(),
            auto_update: false,
            default_node: None,
        };
        core.set_subscription(&entry).unwrap();
        core.update().unwrap();
        let names: Vec<String> = core.nodes().iter().map(|n| n.name.clone()).collect();
        assert!(names[0].starts_with("work-node-0-v1"));
    }

    #[test]
    fn local_only_subscription_update_is_noop() {
        let core = FakeCore::fresh();
        let entry = SubscriptionEntry {
            name: "local".into(),
            update_url: None,
            file: "/nonexistent/local.json".into(),
            auto_update: false,
            default_node: None,
        };
        core.set_subscription(&entry).unwrap();
        core.update().unwrap();
        assert_eq!(core.nodes()[0].name, "local-node-0-v0");
    }

    #[test]
    fn traffic_stream_flows_while_connected_then_ends() {
        let core = FakeCore::fresh();
        core.connect(Mode::Tun).unwrap();
        let rx = core.traffic_rx().unwrap();
        let first = rx.recv_timeout(Duration::from_millis(500)).unwrap();
        assert!(first.up > 0);
        core.disconnect().unwrap();
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
    }

    #[test]
    fn reattach_sees_live_tunnel_across_cores_sharing_world() {
        let world = Arc::new(Mutex::new(FakeWorld::default()));
        let first = FakeCore::new(Arc::clone(&world));
        first.connect(Mode::Proxy).unwrap();
        first.detach().unwrap();
        let second = FakeCore::new(Arc::clone(&world));
        assert!(second.reattach().unwrap());
        assert_eq!(second.tunnel_state(), TunnelState::Up(Mode::Proxy));
    }

    #[test]
    fn reattach_reports_false_when_tunnel_down() {
        let core = FakeCore::fresh();
        assert!(!core.reattach().unwrap());
    }

    #[test]
    fn set_subscription_honors_default_node() {
        let core = FakeCore::fresh();
        let entry = SubscriptionEntry {
            name: "work".into(),
            update_url: None,
            file: "/nonexistent/work.json".into(),
            auto_update: false,
            default_node: Some("work-node-1-v0".into()),
        };
        core.set_subscription(&entry).unwrap();
        assert_eq!(core.active_node().unwrap(), "work-node-1-v0");
    }
}
