use std::fs::File;
use std::io::{BufRead, BufReader};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Remote address used to steer the system resolver into the TUN so sing-box's
/// `hijack-dns` action can intercept it (ADR-0009). It is a routing target, not
/// necessarily the real upstream — sing-box answers the query itself. It must
/// not be an address excluded from the TUN routes.
const TUN_DNS_SENTINEL: &str = "1.1.1.1";

/// Routing table reserved for zenith's TUN routes, kept away from nekobox's
/// 2022 so the two can coexist (ADR-0010).
pub const TUN_ROUTE_TABLE: u32 = 1024;

/// First ip-rule priority reserved for zenith's TUN routes (ADR-0010).
pub const TUN_RULE_INDEX: u32 = 8000;

/// After TUN connect, point systemd-resolved at a remote DNS IP routed through the TUN.
///
/// We cannot use the TUN gateway (172.28.0.1) because gvisor delivers those packets
/// locally — they never reach the route system where `hijack-dns` intercepts port-53
/// traffic and feeds it to the DNS engine.  By using a remote IP the DNS packets are
/// forwarded through the TUN → route system → hijack-dns → DNS engine, which
/// resolves them via the configured upstream (fakeip / proxy).
fn configure_dns_for_tun() {
    let tun_name = {
        let Ok(output) = Command::new("ip")
            .arg("-o")
            .arg("link")
            .arg("show")
            .output()
        else {
            return;
        };
        let stdout = String::from_utf8_lossy(&output.stdout);
        stdout.lines().find_map(|l| {
            let name = l.split(": ").nth(1)?;
            if name == "zenith" {
                Some(name.to_string())
            } else {
                None
            }
        })
    };
    let Some(name) = tun_name else {
        return;
    };
    let _ = Command::new("sudo")
        .args(["-n", "-E", "resolvectl", "dns", &name, TUN_DNS_SENTINEL])
        .output();
    let _ = Command::new("sudo")
        .args(["-n", "-E", "resolvectl", "domain", &name, "~."])
        .output();
    let _ = Command::new("sudo")
        .args(["-n", "-E", "resolvectl", "default-route", &name, "yes"])
        .output();
}

/// Restore default DNS after TUN disconnect.
fn restore_dns_for_tun() {
    let _ = Command::new("sudo")
        .args(["-n", "-E", "resolvectl", "revert", "zenith"])
        .output();
}

/// Remove TUN routing state left behind if a previous core died without
/// cleaning up. The kernel destroys a TUN device when its owner exits, but the
/// ip rules and routes it installed persist. We only ever touch zenith's
/// reserved table and rule indices (ADR-0010): another client's `tun_*`
/// interface normally means that client is *live*, since TUN devices die with
/// their owner, so we leave them alone.
fn cleanup_stale_tun_state() {
    let _ = Command::new("sudo")
        .args([
            "-n",
            "ip",
            "route",
            "flush",
            "table",
            &TUN_ROUTE_TABLE.to_string(),
        ])
        .output();
    for pref in [TUN_RULE_INDEX, TUN_RULE_INDEX + 1] {
        let _ = Command::new("sudo")
            .args(["-n", "ip", "rule", "del", "pref", &pref.to_string()])
            .output();
    }
    let _ = Command::new("sudo")
        .args(["-n", "ip", "link", "del", "zenith"])
        .output();
}

use serde_json::json;
use ureq::{Agent, AgentBuilder};

use crate::core::{
    reconcile_active, Core, CoreError, Mode, Node, Rule, TrafficSample, TunnelState,
};
use crate::settings::{
    expand_home, load_state, save_state, state_path, CorePointer, ProxyBackend, SubscriptionEntry,
};
use crate::subscription;

/// Port the proxy-mode mixed inbound listens on.
pub const MIXED_PORT: u16 = 1080;

const DELAY_TEST_URL: &str = "https://www.gstatic.com/generate_204";
const DELAY_TEST_TIMEOUT_MS: u64 = 5000;

/// Name of the log that receives a spawned core's stderr, so failed startups
/// can explain themselves.
const SINGBOX_ERR_LOG: &str = "sing-box.err";

/// Deprecated sing-box config features that real subscriptions still ship;
/// sing-box keeps them behind env vars until 1.14 removes them.
const DEPRECATION_VARS: &[&str] = &[
    "ENABLE_DEPRECATED_LEGACY_DNS_FAKEIP_OPTIONS",
    "ENABLE_DEPRECATED_LEGACY_DNS_SERVERS",
    "ENABLE_DEPRECATED_OUTBOUND_DNS_RULE_ITEM",
    "ENABLE_DEPRECATED_MISSING_DOMAIN_RESOLVER",
];

/// Where the sourceable proxy environment file lives, next to the generated
/// config. `clear_system_proxy` removes it.
fn env_file_path(config_path: &std::path::Path) -> PathBuf {
    config_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("proxy.env")
}

/// The `export ...` lines pointing proxy-aware tools at the mixed port. Printed
/// by `zenith --proxy-env` and written to `proxy.env` for `source`-ing.
pub fn proxy_env_script() -> String {
    let http = format!("http://127.0.0.1:{}", MIXED_PORT);
    let socks = format!("socks5://127.0.0.1:{}", MIXED_PORT);
    format!(
        "# Generated by zenith while proxy mode is connected. Source this file.\n\
         export http_proxy=\"{http}\"\n\
         export https_proxy=\"{http}\"\n\
         export all_proxy=\"{socks}\"\n\
         export no_proxy=\"localhost,127.0.0.1,::1\"\n\
         export ALL_PROXY=\"{socks}\"\n\
         export NO_PROXY=\"localhost,127.0.0.1,::1\"\n"
    )
}

/// Resolve the sing-box binary on PATH so `sudo` can be handed an absolute
/// path (sudo resets PATH and would otherwise report it as missing).
fn find_singbox() -> Option<PathBuf> {
    for dir in std::env::split_paths(&std::env::var_os("PATH")?) {
        let candidate = dir.join("sing-box");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Remove ANSI color/reset codes from a log line.
fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                while let Some(&c) = chars.peek() {
                    chars.next();
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(ch);
    }
    out
}

/// The system-proxy values captured by `apply_system_proxy` so they can be
/// restored on disconnect (US19).
#[derive(Clone, Debug)]
struct SavedProxy {
    mode: Option<String>,
    http_host: Option<String>,
    http_port: Option<String>,
    https_host: Option<String>,
    https_port: Option<String>,
    socks_host: Option<String>,
    socks_port: Option<String>,
}

impl SavedProxy {
    fn capture() -> SavedProxy {
        SavedProxy {
            mode: gsettings_get("mode"),
            http_host: gsettings_get("http host"),
            http_port: gsettings_get("http port"),
            https_host: gsettings_get("https host"),
            https_port: gsettings_get("https port"),
            socks_host: gsettings_get("socks host"),
            socks_port: gsettings_get("socks port"),
        }
    }

    fn restore(&self) {
        if let Some(mode) = &self.mode {
            gsettings_set("mode", mode);
        }
        if let Some(v) = &self.http_host {
            gsettings_set("http host", v);
        }
        if let Some(v) = &self.http_port {
            gsettings_set("http port", v);
        }
        if let Some(v) = &self.https_host {
            gsettings_set("https host", v);
        }
        if let Some(v) = &self.https_port {
            gsettings_set("https port", v);
        }
        if let Some(v) = &self.socks_host {
            gsettings_set("socks host", v);
        }
        if let Some(v) = &self.socks_port {
            gsettings_set("socks port", v);
        }
    }
}

fn gsettings_get(schema_key: &str) -> Option<String> {
    let (schema, key) = match schema_key.split_once(' ') {
        Some((s, k)) => (format!("org.gnome.system.proxy.{}", s), k.to_string()),
        None => ("org.gnome.system.proxy".to_string(), schema_key.to_string()),
    };
    let out = Command::new("gsettings")
        .args(["get", &schema, &key])
        .output()
        .ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        None
    }
}

fn gsettings_set(schema_key: &str, value: &str) {
    let (schema, key) = match schema_key.split_once(' ') {
        Some((s, k)) => (format!("org.gnome.system.proxy.{}", s), k.to_string()),
        None => ("org.gnome.system.proxy".to_string(), schema_key.to_string()),
    };
    let _ = Command::new("gsettings")
        .args(["set", &schema, &key, value])
        .status();
}

/// Whether the current GNOME proxy points at zenith's mixed port — e.g. a
/// setting left behind by a previous run that died before restoring it. Only
/// this exact signature is treated as ours, so a user's own proxy is untouched.
fn proxy_points_at_us() -> bool {
    let mode = gsettings_get("mode").unwrap_or_default();
    if !mode.contains("manual") {
        return false;
    }
    let host = gsettings_get("http host").unwrap_or_default();
    let port = gsettings_get("http port").unwrap_or_default();
    let host = host.trim().trim_matches('\'');
    (host == "127.0.0.1" || host == "localhost") && port.trim() == MIXED_PORT.to_string()
}

struct LiveState {
    entry: Option<SubscriptionEntry>,
    nodes: Vec<Node>,
    active: Option<String>,
    tunnel: TunnelState,
    pointer: Option<CorePointer>,
    config_path: PathBuf,
    child: Option<Child>,
    rules: Vec<Rule>,
    traffic_rx: Option<mpsc::Receiver<TrafficSample>>,
    generation: u64,
    saved_proxy: Option<SavedProxy>,
}

/// The real `Core`: spawns and drives the sing-box subprocess over its
/// Clash-compatible HTTP API (ADR-0001, ADR-0002).
pub struct SingBoxCore {
    inner: Arc<Mutex<LiveState>>,
    agent: Agent,
}

impl SingBoxCore {
    pub fn new(config_dir: PathBuf) -> Self {
        let agent = AgentBuilder::new()
            .user_agent("singbox")
            .timeout_connect(Duration::from_secs(5))
            .timeout_read(Duration::from_secs(5))
            .build();
        SingBoxCore {
            inner: Arc::new(Mutex::new(LiveState {
                entry: None,
                nodes: Vec::new(),
                active: None,
                tunnel: TunnelState::Down,
                pointer: None,
                config_path: config_dir.join("generated.json"),
                child: None,
                rules: Vec::new(),
                traffic_rx: None,
                generation: 0,
                saved_proxy: None,
            })),
            agent,
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, LiveState> {
        self.inner.lock().unwrap()
    }

    fn load_subscription_into(&self) -> Result<(), CoreError> {
        let entry = self.state().entry.clone();
        let Some(entry) = entry else {
            return Err(CoreError("no subscription loaded".to_string()));
        };
        let parsed = subscription::parse_file(&entry.file)?;
        let mut state = self.state();
        state.nodes = parsed.nodes;
        state.rules = parsed.rules;
        state.active = reconcile_active(&state.nodes, &state.active);
        Ok(())
    }

    /// Build and write the runnable config the core launches: the subscription
    /// as-is plus the injected control surface (ADR-0002).
    fn write_generated_config(
        &self,
        file: &std::path::Path,
        mode: Mode,
        port: u16,
        secret: &str,
    ) -> Result<(), CoreError> {
        let config_value = std::fs::read_to_string(file)
            .map_err(|e| CoreError(format!("read {}: {}", file.display(), e)))?;
        let base: serde_json::Value = serde_json::from_str(&config_value)
            .map_err(|e| CoreError(format!("invalid subscription JSON: {}", e)))?;
        let injected = subscription::inject_control(&base, mode, port, secret)?;
        let json = serde_json::to_string_pretty(&injected).map_err(|e| e.to_string())?;
        std::fs::write(&self.state().config_path, json).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn regenerate(&self) -> Result<(), CoreError> {
        let (file, mode, port, secret) = {
            let state = self.state();
            let entry = state
                .entry
                .as_ref()
                .ok_or_else(|| CoreError("no subscription".to_string()))?;
            let pointer = state
                .pointer
                .as_ref()
                .ok_or_else(|| CoreError("core not running".to_string()))?;
            (
                entry.file.clone(),
                pointer.mode,
                pointer.port,
                pointer.secret.clone(),
            )
        };
        self.write_generated_config(&file, mode, port, &secret)
    }

    /// After the subscription file changes, regenerate the config and tell a
    /// live core to load it — without dropping the tunnel (US15).
    fn apply_config_to_live(&self) -> Result<(), CoreError> {
        let (pointer, config_path) = {
            let state = self.state();
            if !state.tunnel.is_up() {
                return Ok(());
            }
            let pointer = state
                .pointer
                .clone()
                .ok_or_else(|| CoreError("core not running".to_string()))?;
            (pointer, state.config_path.clone())
        };
        self.regenerate()?;
        self.put_json(&pointer, "/configs", json!({ "path": config_path }))?;
        Ok(())
    }

    fn api_base(&self, pointer: &CorePointer) -> String {
        format!("http://127.0.0.1:{}", pointer.port)
    }

    fn auth_header(secret: &str) -> String {
        format!("Bearer {}", secret)
    }

    fn get(&self, pointer: &CorePointer, path: &str) -> Result<ureq::Response, CoreError> {
        let url = format!("{}{}", self.api_base(pointer), path);
        self.agent
            .get(&url)
            .set("Authorization", &Self::auth_header(&pointer.secret))
            .call()
            .map_err(|e| CoreError(format!("{}: {}", url, e)))
    }

    fn put_json(
        &self,
        pointer: &CorePointer,
        path: &str,
        body: serde_json::Value,
    ) -> Result<(), CoreError> {
        let url = format!("{}{}", self.api_base(pointer), path);
        self.agent
            .put(&url)
            .set("Authorization", &Self::auth_header(&pointer.secret))
            .send_json(body)
            .map(|_| ())
            .map_err(|e| CoreError(format!("{}: {}", url, e)))
    }

    fn probe_alive(&self, pointer: &CorePointer, tries: usize) -> bool {
        for _ in 0..tries {
            if self.get(pointer, "/version").is_ok() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        false
    }

    fn spawn_core(&self, mode: Mode) -> Result<Child, CoreError> {
        let config_path = self.state().config_path.clone();
        let bin =
            find_singbox().ok_or_else(|| CoreError("sing-box not found on PATH".to_string()))?;
        let devnull = File::open("/dev/null").map_err(|e| CoreError(e.to_string()))?;
        let needs_sudo = unsafe { libc::geteuid() } != 0 && mode == Mode::Tun;

        if needs_sudo {
            let check = std::process::Command::new("sudo")
                .arg("-n")
                .arg("true")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            match check {
                Ok(s) if s.success() => {}
                _ => {
                    return Err(CoreError(
                        "sudo credentials not cached — run `sudo -v` first".to_string(),
                    ));
                }
            }
        }

        let mut cmd = if needs_sudo {
            let mut c = Command::new("sudo");
            c.arg("-n").arg("-E").arg(&bin);
            c
        } else {
            Command::new(&bin)
        };
        cmd.arg("run").arg("-c").arg(&config_path);
        for var in DEPRECATION_VARS {
            cmd.env(var, "true");
        }
        // Detach from our terminal (new session) so the core survives the TUI
        // quitting (ADR-0003). stderr goes to a log so a failed startup can
        // explain itself instead of being invisible.
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        let stderr_log = config_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join(SINGBOX_ERR_LOG);
        let stderr = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&stderr_log)
            .map_err(|e| CoreError(format!("open {}: {}", stderr_log.display(), e)))?;
        cmd.stdin(Stdio::from(
            devnull.try_clone().map_err(|e| CoreError(e.to_string()))?,
        ))
        .stdout(Stdio::from(
            devnull.try_clone().map_err(|e| CoreError(e.to_string()))?,
        ))
        .stderr(Stdio::from(stderr));
        cmd.spawn()
            .map_err(|e| CoreError(format!("failed to spawn sing-box: {}", e)))
    }

    /// The tail of the last core run's stderr, ANSI stripped, for error
    /// messages — sing-box prints a FATAL line when it refuses a config.
    fn last_stderr(&self) -> String {
        let path = self
            .state()
            .config_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join(SINGBOX_ERR_LOG);
        let Ok(log) = std::fs::read_to_string(path) else {
            return String::new();
        };
        let lines: Vec<String> = log
            .lines()
            .filter_map(|l| {
                let clean = strip_ansi(l).trim().to_string();
                if clean.is_empty() {
                    None
                } else {
                    Some(clean)
                }
            })
            .collect();
        lines
            .iter()
            .rev()
            .take(4)
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(" | ")
    }

    fn free_port(&self) -> u16 {
        if let Ok(listener) = std::net::TcpListener::bind(("127.0.0.1", 0)) {
            if let Ok(addr) = listener.local_addr() {
                return addr.port();
            }
        }
        9090
    }

    fn new_secret(&self) -> String {
        let mut bytes = vec![0x5e; 16];
        if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
            use std::io::Read;
            let mut buf = [0u8; 16];
            if f.read_exact(&mut buf).is_ok() {
                bytes = buf.to_vec();
            }
        }
        bytes.iter().map(|b| format!("{:02x}", b)).collect()
    }

    fn persist_pointer(&self) -> Result<(), CoreError> {
        let state = self.state();
        let Some(pointer) = state.pointer.clone() else {
            return Ok(());
        };
        let dir = state
            .config_path
            .parent()
            .unwrap_or(std::path::Path::new("."));
        let state_path = state_path(dir);
        let mut file_state = load_state(&state_path).unwrap_or_default();
        file_state.core = Some(pointer);
        save_state(&state_path, &file_state).map_err(CoreError)
    }

    fn start_traffic_thread(&self) -> Result<(), CoreError> {
        let mut state = self.state();
        let pointer = state
            .pointer
            .clone()
            .ok_or_else(|| CoreError("core not running".to_string()))?;
        let (tx, rx) = mpsc::channel();
        state.traffic_rx = Some(rx);
        let generation = state.generation;
        let inner = Arc::clone(&self.inner);
        let agent = self.agent.clone();
        std::thread::spawn(move || {
            let base = format!("http://127.0.0.1:{}", pointer.port);
            let url = format!("{}/traffic", base);
            let req = agent
                .get(&url)
                .set("Authorization", &SingBoxCore::auth_header(&pointer.secret));
            let Ok(resp) = req.call() else {
                return;
            };
            let mut reader = BufReader::new(resp.into_reader());
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) => break,
                    Err(_) => break,
                    Ok(_) => {}
                }
                let g = inner.lock().unwrap().generation;
                if g != generation {
                    break;
                }
                let value: Option<serde_json::Value> = serde_json::from_str(line.trim()).ok();
                let Some(value) = value else {
                    continue;
                };
                // sing-box's /traffic already reports per-second deltas
                // (`Up: uploadTotalNew - uploadTotal`), i.e. a rate; do not
                // difference them again.
                let sample = TrafficSample {
                    up: value.get("up").and_then(|v| v.as_u64()).unwrap_or(0),
                    down: value.get("down").and_then(|v| v.as_u64()).unwrap_or(0),
                };
                let _ = tx.send(sample);
            }
            let mut state = inner.lock().unwrap();
            if state.tunnel.is_up() && state.generation == generation {
                state.tunnel = TunnelState::Down;
            }
        });
        Ok(())
    }
}

impl Core for SingBoxCore {
    fn set_subscription(&self, entry: &SubscriptionEntry) -> Result<(), CoreError> {
        let mut state = self.state();
        if state.tunnel.is_up() {
            return Err(CoreError(
                "disconnect before switching subscription".to_string(),
            ));
        }
        let mut entry = entry.clone();
        entry.file = expand_home(&entry.file);
        state.entry = Some(entry.clone());
        drop(state);
        self.load_subscription_into()?;
        if let Some(default) = entry.default_node {
            let mut state = self.state();
            if state.nodes.iter().any(|n| n.name == default) {
                state.active = Some(default);
            }
        }
        Ok(())
    }

    fn update(&self) -> Result<(), CoreError> {
        let entry = self.state().entry.clone();
        let Some(entry) = entry else {
            return Err(CoreError("no subscription loaded".to_string()));
        };
        let Some(url) = entry.update_url.clone() else {
            return Ok(());
        };
        let body = self
            .agent
            .get(&url)
            .call()
            .map_err(|e| CoreError(format!("update fetch failed: {}", e)))?
            .into_string()
            .map_err(|e| CoreError(format!("update read failed: {}", e)))?;
        // Validate before touching the file so a bad payload never clobbers the
        // previous node list (07). Write via a sibling temp file + rename so
        // the subscription file is always complete.
        subscription::parse_str(&body)?;
        let tmp = entry
            .file
            .with_extension(format!("json.tmp{}", std::process::id()));
        std::fs::write(&tmp, &body)
            .map_err(|e| CoreError(format!("write {}: {}", tmp.display(), e)))?;
        std::fs::rename(&tmp, &entry.file)
            .map_err(|e| CoreError(format!("replace {}: {}", entry.file.display(), e)))?;
        self.load_subscription_into()?;
        self.apply_config_to_live()?;
        Ok(())
    }

    fn nodes(&self) -> Vec<Node> {
        self.state().nodes.clone()
    }

    fn active_node(&self) -> Option<String> {
        self.state().active.clone()
    }

    fn switch_node(&self, name: &str) -> Result<(), CoreError> {
        {
            let state = self.state();
            if !state.nodes.iter().any(|n| n.name == name) {
                return Err(CoreError(format!("unknown node: {}", name)));
            }
        }
        let pointer = self.state().pointer.clone();
        if let Some(pointer) = pointer {
            self.put_json(&pointer, "/proxies/proxy", json!({ "name": name }))?;
        }
        self.state().active = Some(name.to_string());
        Ok(())
    }

    fn test_delays(&self) -> Result<(), CoreError> {
        let pointer = self
            .state()
            .pointer
            .clone()
            .ok_or_else(|| CoreError("connect first to test delays".to_string()))?;
        let nodes = self.state().nodes.clone();
        if nodes.is_empty() {
            return Ok(());
        }

        // Probe every node at once: a dead node costs the full timeout, so a
        // serial sweep of N dead nodes would take N × timeout. Each worker only
        // reads shared state via `get`; results are merged afterwards.
        let results: Vec<(String, Option<u64>)> = std::thread::scope(|scope| {
            let handles: Vec<_> = nodes
                .iter()
                .map(|node| {
                    let name = node.name.clone();
                    let pointer = &pointer;
                    scope.spawn(move || {
                        let url = format!(
                            "/proxies/{}/delay?url={}&timeout={}",
                            name, DELAY_TEST_URL, DELAY_TEST_TIMEOUT_MS
                        );
                        let delay = match self.get(pointer, &url) {
                            Ok(resp) => resp
                                .into_json::<serde_json::Value>()
                                .ok()
                                .and_then(|v| v.get("delay").and_then(|d| d.as_u64())),
                            Err(_) => None,
                        };
                        (name, delay)
                    })
                })
                .collect();
            handles
                .into_iter()
                .filter_map(|handle| handle.join().ok())
                .collect()
        });

        for (name, delay) in results {
            if let Some(node) = self.state().nodes.iter_mut().find(|n| n.name == name) {
                node.delay = delay;
            }
        }
        Ok(())
    }

    fn connect(&self, mode: Mode) -> Result<(), CoreError> {
        {
            let state = self.state();
            if state.tunnel.is_up() {
                return Err(CoreError("tunnel already up".to_string()));
            }
        }
        let entry = self
            .state()
            .entry
            .clone()
            .ok_or_else(|| CoreError("no subscription loaded".to_string()))?;
        self.state().generation += 1;
        if mode == Mode::Tun {
            cleanup_stale_tun_state();
        }
        let port = self.free_port();
        let secret = self.new_secret();
        self.write_generated_config(&entry.file, mode, port, &secret)?;
        let child = self.spawn_core(mode)?;
        let pid = child.id();
        let pointer = CorePointer {
            pid: Some(pid),
            port,
            secret,
            mode,
        };
        if !self.probe_alive(&pointer, 15) {
            let _ = unsafe { libc::kill(-(pid as i32), libc::SIGTERM) };
            let reason = self.last_stderr();
            let msg = if reason.is_empty() {
                "sing-box did not come up".to_string()
            } else {
                format!("sing-box did not come up: {}", reason)
            };
            return Err(CoreError(msg));
        }
        let mut state = self.state();
        state.child = Some(child);
        state.pointer = Some(pointer.clone());
        state.tunnel = TunnelState::Up(mode);
        state.active = reconcile_active(&state.nodes, &state.active);
        let active_node = state.active.clone();
        drop(state);
        if let Some(ref name) = active_node {
            self.switch_node(name)?;
        }
        self.start_traffic_thread()?;
        self.persist_pointer()?;
        if mode == Mode::Tun {
            configure_dns_for_tun();
        }
        Ok(())
    }

    fn disconnect(&self) -> Result<(), CoreError> {
        let mut state = self.state();
        state.generation += 1;
        let was_tun = matches!(state.tunnel, TunnelState::Up(Mode::Tun));
        state.tunnel = TunnelState::Down;
        if let Some(pointer) = state.pointer.take() {
            if let Some(pid) = pointer.pid {
                let _ = unsafe { libc::kill(-(pid as i32), libc::SIGTERM) };
            }
        }
        if let Some(mut child) = state.child.take() {
            let _ = child.wait();
        }
        let dir = state
            .config_path
            .parent()
            .unwrap_or(std::path::Path::new("."));
        let state_path = state_path(dir);
        if let Ok(mut file_state) = load_state(&state_path) {
            file_state.core = None;
            let _ = save_state(&state_path, &file_state);
        }
        drop(state);
        if was_tun {
            restore_dns_for_tun();
        }
        Ok(())
    }

    fn tunnel_state(&self) -> TunnelState {
        self.state().tunnel
    }

    fn rules(&self) -> Vec<Rule> {
        self.state().rules.clone()
    }

    fn reload(&self) -> Result<(), CoreError> {
        self.load_subscription_into()?;
        self.apply_config_to_live()
    }

    fn edit_file(&self, path: &std::path::Path) -> Result<(), CoreError> {
        let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
        let status = std::process::Command::new(&editor)
            .arg(path)
            .status()
            .map_err(|e| CoreError(format!("could not start {}: {}", editor, e)))?;
        if status.success() {
            Ok(())
        } else {
            Err(CoreError(format!("{} exited with {}", editor, status)))
        }
    }

    fn nerd_font_available(&self) -> bool {
        static CACHE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *CACHE.get_or_init(|| {
            let Ok(output) = std::process::Command::new("fc-list")
                .arg(":family")
                .output()
            else {
                return false;
            };
            let families = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
            families.contains("nerd font") || families.contains("nerdfont")
        })
    }

    fn traffic_rx(&self) -> Option<mpsc::Receiver<TrafficSample>> {
        self.state().traffic_rx.take()
    }

    fn detach(&self) -> Result<(), CoreError> {
        self.persist_pointer()
    }

    fn reattach(&self) -> Result<bool, CoreError> {
        let state = self.state();
        let dir = state
            .config_path
            .parent()
            .unwrap_or(std::path::Path::new("."));
        let state_path = state_path(dir);
        drop(state);
        let Some(pointer) = load_state(&state_path).unwrap_or_default().core else {
            return Ok(false);
        };
        if !self.probe_alive(&pointer, 5) {
            // No live core on the recorded endpoint: start fresh and clear the
            // stale pointer (06).
            if let Ok(mut file_state) = load_state(&state_path) {
                file_state.core = None;
                let _ = save_state(&state_path, &file_state);
            }
            let mut state = self.state();
            state.pointer = None;
            state.tunnel = TunnelState::Down;
            return Ok(false);
        }
        let mut state = self.state();
        state.generation += 1;
        state.pointer = Some(pointer.clone());
        state.tunnel = TunnelState::Up(pointer.mode);
        drop(state);
        self.load_subscription_into()?;
        let active = self
            .get(&pointer, "/proxies/proxy")
            .ok()
            .and_then(|resp| resp.into_json::<serde_json::Value>().ok())
            .and_then(|value| {
                value
                    .get("now")
                    .and_then(|n| n.as_str())
                    .map(|s| s.to_string())
            });
        if active.is_some() {
            self.state().active = active;
        }
        self.start_traffic_thread()?;
        Ok(true)
    }

    fn apply_system_proxy(&self, backend: ProxyBackend) -> Result<(), CoreError> {
        let mut state = self.state();
        if backend.touches_gnome() {
            if state.saved_proxy.is_none() {
                state.saved_proxy = Some(SavedProxy::capture());
            }
            gsettings_set("mode", "'manual'");
            gsettings_set("http host", "'127.0.0.1'");
            gsettings_set("http port", &MIXED_PORT.to_string());
            gsettings_set("https host", "'127.0.0.1'");
            gsettings_set("https port", &MIXED_PORT.to_string());
            gsettings_set("socks host", "'127.0.0.1'");
            gsettings_set("socks port", &MIXED_PORT.to_string());
        }
        if backend.touches_env() {
            let path = env_file_path(&state.config_path);
            let _ = std::fs::write(path, proxy_env_script());
        }
        Ok(())
    }

    fn clear_system_proxy(&self) -> Result<(), CoreError> {
        let mut state = self.state();
        if let Some(saved) = state.saved_proxy.take() {
            saved.restore();
        } else if proxy_points_at_us() {
            // No snapshot to restore (a previous run was killed, or this is a
            // fresh TUN connect): drop the stale setting only if it is ours.
            gsettings_set("mode", "'none'");
        }
        let _ = std::fs::remove_file(env_file_path(&state.config_path));
        Ok(())
    }

    fn log_content(&self) -> String {
        let path = self
            .state()
            .config_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join(SINGBOX_ERR_LOG);
        let Ok(log) = std::fs::read_to_string(path) else {
            return String::new();
        };
        let lines: Vec<String> = log
            .lines()
            .filter_map(|l| {
                let clean = strip_ansi(l).trim().to_string();
                if clean.is_empty() {
                    None
                } else {
                    Some(clean)
                }
            })
            .collect();
        let tail = if lines.len() > 100 {
            &lines[lines.len() - 100..]
        } else {
            &lines
        };
        tail.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core() -> SingBoxCore {
        let dir = std::env::temp_dir().join(format!("zenith-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        SingBoxCore::new(dir)
    }

    #[test]
    fn proxy_env_script_points_at_the_mixed_port() {
        let script = proxy_env_script();
        assert!(script.contains(&format!("http://127.0.0.1:{}", MIXED_PORT)));
        assert!(script.contains(&format!("socks5://127.0.0.1:{}", MIXED_PORT)));
        assert!(script.contains("all_proxy"));
        assert!(script.contains("no_proxy"));
    }

    #[test]
    fn secret_is_hex_string() {
        let c = core();
        let s = c.new_secret();
        assert_eq!(s.len(), 32);
        assert!(s.chars().all(|ch| ch.is_ascii_hexdigit()));
    }

    #[test]
    fn free_port_is_nonzero_and_bindable() {
        let c = core();
        let port = c.free_port();
        assert_ne!(port, 0);
        let listener = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
        drop(listener);
    }

    #[test]
    fn proxy_inbound_uses_fixed_mixed_port() {
        let sub: serde_json::Value =
            serde_json::from_str(r#"{"outbounds":[{"type":"vless","tag":"a"}]}"#).unwrap();
        let injected = subscription::inject_control(&sub, Mode::Proxy, 9090, "s").unwrap();
        let mixed = injected["inbounds"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["type"] == "mixed")
            .unwrap();
        assert_eq!(mixed["listen_port"], MIXED_PORT);
    }

    #[test]
    fn update_without_url_is_noop() {
        let c = core();
        let dir = std::env::temp_dir().join(format!("zenith-singbox-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("sub.json");
        std::fs::write(&file, r#"{"outbounds":[{"type":"vless","tag":"a"}]}"#).unwrap();
        let entry = SubscriptionEntry {
            name: "local".into(),
            update_url: None,
            file,
            auto_update: false,
            default_node: None,
        };
        c.set_subscription(&entry).unwrap();
        c.update().unwrap();
    }

    #[test]
    fn set_subscription_requires_existing_file() {
        let c = core();
        let entry = SubscriptionEntry {
            name: "x".into(),
            update_url: None,
            file: "/definitely/not/here.json".into(),
            auto_update: false,
            default_node: None,
        };
        assert!(c.set_subscription(&entry).is_err());
    }

    #[test]
    fn switch_unknown_node_fails_before_any_api_call() {
        let c = core();
        assert!(c.switch_node("ghost").is_err());
    }

    #[test]
    fn switches_node_against_live_core() {
        if std::process::Command::new("sing-box")
            .arg("version")
            .output()
            .is_err()
        {
            eprintln!("skipping: sing-box not installed");
            return;
        }
        // A live zenith (or other core) may already hold the mixed port; skip
        // rather than fail on a bind error.
        if std::net::TcpListener::bind(("127.0.0.1", MIXED_PORT)).is_err() {
            eprintln!("skipping: port {} already in use", MIXED_PORT);
            return;
        }
        let c = core();
        let dir = std::env::temp_dir().join(format!("zenith-live-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("sub.json");
        std::fs::write(
            &file,
            r#"{"log":{"level":"error"},
               "outbounds":[
                 {"type":"direct","tag":"direct"},
                 {"type":"vless","tag":"jp-1","server":"1.2.3.4","server_port":443,
                  "uuid":"00000000-0000-0000-0000-000000000000"}
               ],
               "route":{"final":"direct"}}"#,
        )
        .unwrap();
        let entry = SubscriptionEntry {
            name: "live".into(),
            update_url: None,
            file: file.clone(),
            auto_update: false,
            default_node: None,
        };
        c.set_subscription(&entry).unwrap();
        eprintln!("[t] before connect");
        c.connect(Mode::Proxy).unwrap();
        eprintln!("[t] connected, switching node");
        c.switch_node("jp-1").unwrap();
        eprintln!("[t] switched");
        assert_eq!(c.active_node().as_deref(), Some("jp-1"));
        c.disconnect().unwrap();
        eprintln!("[t] disconnected");
    }
}
