use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::core::Mode;

/// How the proxy-mode port is advertised to other apps. GNOME writes
/// `gsettings`; env writes a sourceable file (`proxy.env`) for tools that read
/// `http_proxy`/`all_proxy` (Telegram, curl, most CLI tools). `Both` does each;
/// `None` leaves the system alone (TUN-only setups).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProxyBackend {
    #[default]
    Gnome,
    Env,
    Both,
    None,
}

impl ProxyBackend {
    pub fn touches_gnome(&self) -> bool {
        matches!(self, ProxyBackend::Gnome | ProxyBackend::Both)
    }

    pub fn touches_env(&self) -> bool {
        matches!(self, ProxyBackend::Env | ProxyBackend::Both)
    }
}

impl Serialize for ProxyBackend {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(match self {
            ProxyBackend::Gnome => "gnome",
            ProxyBackend::Env => "env",
            ProxyBackend::Both => "both",
            ProxyBackend::None => "none",
        })
    }
}

impl<'de> Deserialize<'de> for ProxyBackend {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        match serde_yaml::Value::deserialize(d)? {
            serde_yaml::Value::Null => Ok(ProxyBackend::default()),
            serde_yaml::Value::String(s) => match s.to_ascii_lowercase().as_str() {
                "gnome" | "gsettings" => Ok(ProxyBackend::Gnome),
                "env" | "environment" => Ok(ProxyBackend::Env),
                "both" | "all" => Ok(ProxyBackend::Both),
                "none" | "off" => Ok(ProxyBackend::None),
                other => Err(D::Error::custom(format!(
                    "proxy_backend must be gnome, env, both, or none (got {other})"
                ))),
            },
            _ => Err(D::Error::custom(
                "proxy_backend must be gnome, env, both, or none",
            )),
        }
    }
}

/// Whether to draw the Nerd Font logo in the header. `Auto` probes the system
/// for installed patched fonts; a terminal cannot be asked what it renders with,
/// so this is a best-effort heuristic and `On`/`Off` override it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NerdFonts {
    #[default]
    Auto,
    On,
    Off,
}

impl Serialize for NerdFonts {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(match self {
            NerdFonts::Auto => "auto",
            NerdFonts::On => "on",
            NerdFonts::Off => "off",
        })
    }
}

impl<'de> Deserialize<'de> for NerdFonts {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        match serde_yaml::Value::deserialize(d)? {
            serde_yaml::Value::Bool(true) => Ok(NerdFonts::On),
            serde_yaml::Value::Bool(false) => Ok(NerdFonts::Off),
            serde_yaml::Value::Null => Ok(NerdFonts::Auto),
            serde_yaml::Value::String(s) => match s.to_ascii_lowercase().as_str() {
                "auto" => Ok(NerdFonts::Auto),
                "on" | "true" | "yes" => Ok(NerdFonts::On),
                "off" | "false" | "no" => Ok(NerdFonts::Off),
                other => Err(D::Error::custom(format!(
                    "nerd_fonts must be auto, on, or off (got {other})"
                ))),
            },
            _ => Err(D::Error::custom("nerd_fonts must be auto, on, or off")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct SubscriptionEntry {
    pub name: String,
    #[serde(default)]
    pub update_url: Option<String>,
    pub file: PathBuf,
    #[serde(default)]
    pub auto_update: bool,
    #[serde(default)]
    pub default_node: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Settings {
    #[serde(default)]
    pub subscriptions: Vec<SubscriptionEntry>,
    #[serde(default)]
    pub default_subscription: Option<String>,
    #[serde(default = "default_mode")]
    pub default_mode: Mode,
    #[serde(default)]
    pub system_proxy: bool,
    #[serde(default)]
    pub proxy_backend: ProxyBackend,
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default)]
    pub nerd_fonts: NerdFonts,
    #[serde(default = "default_delay_refresh")]
    pub delay_refresh_seconds: u64,
}

fn default_mode() -> Mode {
    Mode::Tun
}

fn default_theme() -> String {
    "default".to_string()
}

fn default_delay_refresh() -> u64 {
    300
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            subscriptions: Vec::new(),
            default_subscription: None,
            default_mode: default_mode(),
            system_proxy: false,
            proxy_backend: ProxyBackend::default(),
            theme: default_theme(),
            nerd_fonts: NerdFonts::Auto,
            delay_refresh_seconds: default_delay_refresh(),
        }
    }
}

impl Settings {
    /// The subscription that should be active on launch: the configured default
    /// if it exists, otherwise the first subscription, otherwise None.
    pub fn active_subscription(&self) -> Option<&SubscriptionEntry> {
        if let Some(name) = &self.default_subscription {
            if let Some(entry) = self.subscriptions.iter().find(|s| &s.name == name) {
                return Some(entry);
            }
        }
        self.subscriptions.first()
    }

    pub fn subscription(&self, name: &str) -> Option<&SubscriptionEntry> {
        self.subscriptions.iter().find(|s| s.name == name)
    }
}

/// Runtime state, written freely by the client. Deliberately separate from the
/// user-edited `Settings` file (ADR-0006).
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
pub struct StateFile {
    /// Where the last launched core was listening, plus its process id — the
    /// re-attach pointer.
    #[serde(default)]
    pub core: Option<CorePointer>,
    /// Last-active node per subscription name.
    #[serde(default)]
    pub active_nodes: HashMap<String, String>,
    /// The mode chosen with `t`, persisted so the next launch starts there.
    #[serde(default)]
    pub default_mode: Option<Mode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct CorePointer {
    pub pid: Option<u32>,
    pub port: u16,
    pub secret: String,
    #[serde(default = "default_core_mode")]
    pub mode: Mode,
}

fn default_core_mode() -> Mode {
    Mode::Tun
}

/// Resolve the config directory: `$XDG_CONFIG_HOME/zenith` or `~/.config/zenith`.
pub fn config_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("zenith");
        }
    }
    dirs::home_dir()
        .map(|home| home.join(".config").join("zenith"))
        .unwrap_or_else(|| PathBuf::from(".zenith"))
}

/// Expand a leading `~` in a path against the home directory, leaving other
/// paths untouched. Used everywhere a subscription file path is consumed so a
/// `~/…` entry behaves the same in the editor, the fake, and the real core.
pub fn expand_home(path: &std::path::Path) -> PathBuf {
    if path.starts_with("~") {
        if let Some(home) = dirs::home_dir() {
            let stripped = path.strip_prefix("~").unwrap_or(path);
            return home.join(stripped);
        }
    }
    path.to_path_buf()
}

/// Where the client's runtime state lives for a given config directory. The
/// app and both core implementations share this single helper so the pointer
/// is always read and written in one place.
pub fn state_path(config_dir: &std::path::Path) -> PathBuf {
    config_dir.join("state.json")
}

/// The path the app derives for a newly added subscription (ADR-0007): under
/// `config_dir/subscriptions/`, slugified so it is safe on disk and stable
/// across renames (the file is fixed at add time, never re-derived).
pub fn subscription_file(config_dir: &std::path::Path, name: &str) -> PathBuf {
    config_dir
        .join("subscriptions")
        .join(format!("{}.json", slugify(name)))
}

/// Lowercase, collapse non-alphanumeric runs to `-`, trim the dashes.
pub fn slugify(name: &str) -> String {
    let mut slug = String::with_capacity(name.len());
    let mut prev_dash = false;
    for ch in name.chars() {
        if ch.is_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            slug.push('-');
            prev_dash = true;
        }
    }
    slug.trim_matches('-').to_string()
}

/// Persist the settings file. The app writes it only for subscription registry
/// mutations (ADR-0007), atomically so a crash never leaves a truncated file.
pub fn save_settings(path: &std::path::Path, settings: &Settings) -> Result<(), String> {
    let yaml = serde_yaml::to_string(settings).map_err(|e| e.to_string())?;
    let tmp = path.with_extension(format!("yaml.tmp{}", std::process::id()));
    std::fs::write(&tmp, yaml).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

pub fn load_settings(path: &std::path::Path) -> Result<Settings, String> {
    let contents = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {}", path.display(), e))?;
    serde_yaml::from_str(&contents).map_err(|e| format!("invalid settings YAML: {}", e))
}

pub fn load_state(path: &std::path::Path) -> Result<StateFile, String> {
    match std::fs::read_to_string(path) {
        Ok(contents) => {
            serde_json::from_str(&contents).map_err(|e| format!("invalid state: {}", e))
        }
        Err(_) => Ok(StateFile::default()),
    }
}

pub fn save_state(path: &std::path::Path, state: &StateFile) -> Result<(), String> {
    let json = serde_json::to_string_pretty(state).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_settings() -> Settings {
        serde_yaml::from_str(
            r#"
default_subscription: work
default_mode: proxy
system_proxy: true
subscriptions:
  - name: work
    update_url: https://example.com/sub
    file: /tmp/work.json
    auto_update: true
  - name: local
    file: /tmp/local.json
"#,
        )
        .unwrap()
    }

    #[test]
    fn active_subscription_prefers_default() {
        let settings = sample_settings();
        assert_eq!(settings.active_subscription().unwrap().name, "work");
    }

    #[test]
    fn active_subscription_falls_back_to_first() {
        let settings: Settings = serde_yaml::from_str(
            "subscriptions:\n  - name: a\n    file: /tmp/a.json\n  - name: b\n    file: /tmp/b.json\n",
        )
        .unwrap();
        assert_eq!(settings.active_subscription().unwrap().name, "a");
    }

    #[test]
    fn local_only_subscription_has_no_update_url() {
        let settings = sample_settings();
        let local = settings.subscription("local").unwrap();
        assert_eq!(local.update_url, None);
    }

    #[test]
    fn nerd_fonts_accepts_words_bools_and_defaults_to_auto() {
        let parse = |yaml: &str| -> NerdFonts {
            serde_yaml::from_str::<Settings>(yaml).unwrap().nerd_fonts
        };
        assert_eq!(parse("nerd_fonts: auto"), NerdFonts::Auto);
        assert_eq!(parse("nerd_fonts: on"), NerdFonts::On);
        assert_eq!(parse("nerd_fonts: off"), NerdFonts::Off);
        assert_eq!(parse("nerd_fonts: true"), NerdFonts::On);
        assert_eq!(parse("nerd_fonts: false"), NerdFonts::Off);
        assert_eq!(parse("{}"), NerdFonts::Auto, "absent defaults to auto");
    }

    #[test]
    fn proxy_backend_parses_words_and_defaults_to_gnome() {
        let parse = |yaml: &str| -> ProxyBackend {
            serde_yaml::from_str::<Settings>(yaml)
                .unwrap()
                .proxy_backend
        };
        assert_eq!(parse("proxy_backend: gnome"), ProxyBackend::Gnome);
        assert_eq!(parse("proxy_backend: env"), ProxyBackend::Env);
        assert_eq!(parse("proxy_backend: both"), ProxyBackend::Both);
        assert_eq!(parse("proxy_backend: none"), ProxyBackend::None);
        assert_eq!(parse("{}"), ProxyBackend::Gnome, "absent defaults to gnome");
    }

    #[test]
    fn state_roundtrips_through_json() {
        let state = StateFile {
            core: Some(CorePointer {
                pid: Some(42),
                port: 9090,
                secret: "sekrit".into(),
                mode: Mode::Proxy,
            }),
            active_nodes: HashMap::from([("work".to_string(), "us-1".to_string())]),
            default_mode: Some(Mode::Tun),
        };
        let json = serde_json::to_string(&state).unwrap();
        let back: StateFile = serde_json::from_str(&json).unwrap();
        assert_eq!(state, back);
    }

    #[test]
    fn core_pointer_defaults_mode_to_tun() {
        let pointer: CorePointer = serde_json::from_str(r#"{"port":1,"secret":"s"}"#).unwrap();
        assert_eq!(pointer.mode, Mode::Tun);
    }

    #[test]
    fn state_defaults_when_missing_fields() {
        let state: StateFile = serde_json::from_str("{}").unwrap();
        assert_eq!(state.core, None);
        assert!(state.active_nodes.is_empty());
        assert_eq!(state.default_mode, None);
    }

    #[test]
    fn expand_home_replaces_leading_tilde() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(
            expand_home(std::path::Path::new("~/sub.json")),
            home.join("sub.json")
        );
        assert_eq!(
            expand_home(std::path::Path::new("/abs/sub.json")),
            std::path::PathBuf::from("/abs/sub.json")
        );
    }

    #[test]
    fn state_path_lives_under_config_dir() {
        assert_eq!(
            state_path(std::path::Path::new("/tmp/zenith")),
            std::path::PathBuf::from("/tmp/zenith/state.json")
        );
    }

    #[test]
    fn slugify_produces_safe_filename() {
        assert_eq!(slugify("My VPN Sub"), "my-vpn-sub");
        assert_eq!(slugify("  A.B!C  "), "a-b-c");
        assert_eq!(slugify("already-good"), "already-good");
        assert_eq!(slugify("!!!"), "");
    }

    #[test]
    fn subscription_file_is_slugged_under_subscriptions_dir() {
        assert_eq!(
            subscription_file(std::path::Path::new("/tmp/zenith"), "Work VPN"),
            std::path::PathBuf::from("/tmp/zenith/subscriptions/work-vpn.json")
        );
    }

    #[test]
    fn save_settings_roundtrips_through_yaml() {
        let dir = std::env::temp_dir().join(format!("zenith-settings-save-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.yaml");
        let settings = sample_settings();
        save_settings(&path, &settings).unwrap();
        let loaded = load_settings(&path).unwrap();
        assert_eq!(loaded, settings);
    }
}
