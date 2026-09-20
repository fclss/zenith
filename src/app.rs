use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::core::{Core, Mode, Node, Rule, TrafficSample, TunnelState};
use crate::settings::{self, Settings};
use crate::symbols::Symbols;
use crate::theme::Theme;

/// Events delivered from background worker threads back to the UI loop.
pub enum AppEvent {
    ConnectResult(Result<(), String>, Mode),
    Updated(Result<(), String>),
    DelaysDone,
    Traffic(TrafficSample),
}

/// The user-visible actions. `main` maps keys and mouse to these; tests call
/// `dispatch` directly.
pub enum Action {
    Up,
    Down,
    SelectNode,
    Connect,
    Disconnect,
    ToggleConnect,
    ToggleMode,
    Update,
    PingAll,
    ToggleRules,
    ToggleLogs,
    ToggleHelp,
    ResetTraffic,
    OpenEditor,
    SwitchSubscription,
    SwitchSubscriptionPrev,
    ToggleManage,
    SelectSubscription,
    AddSubscription,
    EditSubscription,
    DeleteSubscription,
    FormFieldNext,
    FormFieldPrev,
    FormChar(char),
    FormBackspace,
    FormSubmit,
    FormCancel,
    Quit,
}

const TRAFFIC_RING: usize = 90;
const NOTIFICATION_TICKS: u8 = 5;
const FORM_FIELDS: usize = 3;

/// The in-progress add/edit of a subscription (ADR-0007). `index` is `None`
/// while adding; the `file` path is derived at add time and never edited here.
pub struct SubscriptionDraft {
    pub index: Option<usize>,
    pub name: String,
    pub url: String,
    pub auto_update: bool,
    pub focus: usize,
}

pub struct App {
    pub core: Arc<dyn Core>,
    pub settings: Settings,
    pub config_dir: PathBuf,
    pub theme: Theme,
    pub symbols: Symbols,
    pub active_subscription: String,
    pub mode: Mode,
    pub selected_node: usize,
    pub notifications: VecDeque<(String, u8)>,
    pub traffic: VecDeque<TrafficSample>,
    /// Bytes transferred since the last `z` reset, accumulated from samples.
    pub total_up: u64,
    pub total_down: u64,
    pub showing_rules: bool,
    pub showing_logs: bool,
    pub showing_help: bool,
    /// Lines scrolled back from the newest log line (0 = following).
    pub log_scroll: usize,
    pub tunnel_state: TunnelState,
    pub connecting: bool,
    pub nodes: Vec<Node>,
    pub active_node: Option<String>,
    pub rules: Vec<Rule>,
    pub should_exit: bool,
    /// When the last delay test (manual or auto) was kicked off — the US27
    /// auto-refresh gate.
    pub last_delay_refresh: Instant,
    pub delay_test_in_flight: bool,
    delay_refresh_silent: bool,
    pub manage_subscriptions: bool,
    pub draft: Option<SubscriptionDraft>,
    pub selected_subscription: usize,
    event_tx: mpsc::Sender<AppEvent>,
    event_rx: mpsc::Receiver<AppEvent>,
}

impl App {
    pub fn new(settings: Settings, core: Arc<dyn Core>, config_dir: PathBuf) -> Self {
        let active_subscription = settings
            .active_subscription()
            .map(|s| s.name.clone())
            .unwrap_or_default();
        let mode = settings.default_mode;
        let theme = Theme::load(&config_dir, &settings.theme);
        let nerd_fonts = match settings.nerd_fonts {
            settings::NerdFonts::On => true,
            settings::NerdFonts::Off => false,
            settings::NerdFonts::Auto => core.nerd_font_available(),
        };
        let symbols = Symbols::for_setting(nerd_fonts);
        let mut app = App {
            core,
            settings,
            config_dir,
            theme,
            symbols,
            active_subscription,
            mode,
            selected_node: 0,
            notifications: VecDeque::new(),
            traffic: VecDeque::new(),
            total_up: 0,
            total_down: 0,
            showing_rules: false,
            showing_logs: false,
            showing_help: false,
            log_scroll: 0,
            tunnel_state: TunnelState::Down,
            connecting: false,
            nodes: Vec::new(),
            active_node: None,
            rules: Vec::new(),
            should_exit: false,
            last_delay_refresh: Instant::now(),
            delay_test_in_flight: false,
            delay_refresh_silent: false,
            manage_subscriptions: false,
            draft: None,
            selected_subscription: 0,
            event_tx: mpsc::channel().0,
            event_rx: mpsc::channel().1,
        };
        let (tx, rx) = mpsc::channel();
        app.event_tx = tx;
        app.event_rx = rx;
        app.load_active_subscription();
        app.refresh();
        app
    }

    fn state_path(&self) -> PathBuf {
        self.config_dir.join("state.json")
    }

    fn load_active_subscription(&mut self) {
        let Some(entry) = self
            .settings
            .subscription(&self.active_subscription)
            .cloned()
        else {
            return;
        };
        if let Err(e) = self.core.set_subscription(&entry) {
            if entry.update_url.is_some() {
                // No local file yet — fetch it, then the update event reloads.
                self.start_update();
                return;
            }
            self.notify(format!("subscription error: {}", e));
            return;
        }
        let state = settings::load_state(&self.state_path()).unwrap_or_default();
        if let Some(name) = state.active_nodes.get(&self.active_subscription) {
            let _ = self.core.switch_node(name);
        }
    }

    fn notify(&mut self, message: impl Into<String>) {
        self.notifications
            .push_back((message.into(), NOTIFICATION_TICKS));
        while self.notifications.len() > 4 {
            self.notifications.pop_front();
        }
    }

    pub fn refresh(&mut self) {
        self.tunnel_state = self.core.tunnel_state();
        self.nodes = self.core.nodes();
        self.active_node = self.core.active_node();
        self.rules = self.core.rules();
        if self.nodes.is_empty() {
            self.selected_node = 0;
        } else if self.selected_node >= self.nodes.len() {
            self.selected_node = self.nodes.len() - 1;
        }
    }

    pub fn dispatch(&mut self, action: Action) {
        match action {
            Action::Up => self.move_up(),
            Action::Down => self.move_down(),
            Action::SelectNode => self.select_node(),
            Action::Connect => self.connect(),
            Action::Disconnect => self.disconnect(),
            Action::ToggleConnect => {
                if self.tunnel_state.is_up() {
                    self.disconnect();
                } else {
                    self.connect();
                }
            }
            Action::ToggleMode => self.toggle_mode(),
            Action::Update => self.start_update(),
            Action::PingAll => self.start_delay_test(false),
            Action::ToggleRules => {
                self.showing_rules = !self.showing_rules;
                self.showing_logs = false;
            }
            Action::ToggleLogs => {
                self.showing_logs = !self.showing_logs;
                self.log_scroll = 0;
                self.showing_rules = false;
            }
            Action::ToggleHelp => self.showing_help = !self.showing_help,
            Action::ResetTraffic => {
                self.total_up = 0;
                self.total_down = 0;
                self.notify("traffic counters reset");
            }
            Action::OpenEditor => self.open_in_editor(),
            Action::SwitchSubscription => self.switch_subscription(1),
            Action::SwitchSubscriptionPrev => self.switch_subscription(-1),
            Action::ToggleManage => self.toggle_manage(),
            Action::SelectSubscription => self.select_subscription(),
            Action::AddSubscription => self.open_draft(None),
            Action::EditSubscription => self.open_draft(Some(self.selected_subscription)),
            Action::DeleteSubscription => self.delete_subscription(self.selected_subscription),
            Action::FormFieldNext => {
                if let Some(draft) = &mut self.draft {
                    draft.focus = (draft.focus + 1) % FORM_FIELDS;
                }
            }
            Action::FormFieldPrev => {
                if let Some(draft) = &mut self.draft {
                    draft.focus = (draft.focus + FORM_FIELDS - 1) % FORM_FIELDS;
                }
            }
            Action::FormChar(ch) => self.form_char(ch),
            Action::FormBackspace => self.form_backspace(),
            Action::FormSubmit => self.commit_draft(),
            Action::FormCancel => self.draft = None,
            Action::Quit => {
                if self.tunnel_state.is_up() {
                    let _ = self.core.detach();
                }
                self.should_exit = true;
            }
        }
    }

    fn move_up(&mut self) {
        if self.draft.is_some() {
            self.dispatch(Action::FormFieldPrev);
        } else if self.showing_help {
            // Help is modal; arrows do nothing.
        } else if self.showing_logs {
            self.log_scroll = self.log_scroll.saturating_add(1);
        } else if self.manage_subscriptions {
            self.move_subscription_selection(-1);
        } else {
            self.move_selection(-1);
        }
    }

    fn move_down(&mut self) {
        if self.draft.is_some() {
            self.dispatch(Action::FormFieldNext);
        } else if self.showing_help {
            // Help is modal; arrows do nothing.
        } else if self.showing_logs {
            self.log_scroll = self.log_scroll.saturating_sub(1);
        } else if self.manage_subscriptions {
            self.move_subscription_selection(1);
        } else {
            self.move_selection(1);
        }
    }

    fn move_selection(&mut self, delta: isize) {
        if self.nodes.is_empty() {
            return;
        }
        let len = self.nodes.len() as isize;
        let next = (self.selected_node as isize + delta).rem_euclid(len) as usize;
        self.selected_node = next;
    }

    fn select_node(&mut self) {
        let Some(name) = self.nodes.get(self.selected_node).map(|n| n.name.clone()) else {
            return;
        };
        match self.core.switch_node(&name) {
            Ok(()) => {
                self.active_node = Some(name.clone());
                self.save_active_node(&name);
                self.notify(format!("selected {}", name));
            }
            Err(e) => self.notify(format!("select failed: {}", e)),
        }
    }

    /// Kick off a connect on a worker thread so a slow or failing sing-box
    /// startup never freezes the UI; `ConnectResult` applies the outcome.
    fn connect(&mut self) {
        if self.connecting || self.tunnel_state.is_up() {
            return;
        }
        self.connecting = true;
        self.notify("connecting…");
        let core = Arc::clone(&self.core);
        let mode = self.mode;
        let tx = self.event_tx.clone();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                core.connect(mode).map(|_| ()).map_err(|e| e.to_string())
            }))
            .unwrap_or_else(|_| Err("thread panicked".to_string()));
            let _ = tx.send(AppEvent::ConnectResult(result, mode));
        });
    }

    fn apply_connect_result(&mut self, result: Result<(), String>, mode: Mode) {
        self.connecting = false;
        match result {
            Ok(()) => {
                self.tunnel_state = TunnelState::Up(mode);
                self.mode = mode;
                if self.settings.system_proxy && mode == Mode::Proxy {
                    if let Err(e) = self.core.apply_system_proxy(self.settings.proxy_backend) {
                        self.notify(format!("system proxy: {}", e));
                    }
                } else {
                    // Switching back to TUN (or with system proxy disabled) must
                    // drop the old setting, or apps keep dialing the dead
                    // proxy port and get "connection refused".
                    let _ = self.core.clear_system_proxy();
                }
                if let Some(rx) = self.core.traffic_rx() {
                    let tx = self.event_tx.clone();
                    std::thread::spawn(move || {
                        while let Ok(sample) = rx.recv() {
                            if tx.send(AppEvent::Traffic(sample)).is_err() {
                                break;
                            }
                        }
                    });
                }
                self.notify(format!("connected ({})", mode));
            }
            Err(e) => {
                let _ = self.core.clear_system_proxy();
                self.notify(format!("connect failed: {}", e));
            }
        }
        self.refresh();
    }

    fn disconnect(&mut self) {
        if self.connecting {
            self.notify("still connecting…");
            return;
        }
        let _ = self.core.disconnect();
        let _ = self.core.clear_system_proxy();
        self.tunnel_state = TunnelState::Down;
        self.notify("disconnected");
        self.refresh();
    }

    fn toggle_mode(&mut self) {
        self.mode = match self.mode {
            Mode::Tun => Mode::Proxy,
            Mode::Proxy => Mode::Tun,
        };
        if self.tunnel_state.is_up() {
            let _ = self.core.disconnect();
            // Drop the system proxy immediately: the port is dead until the new
            // core is up, and browsers would otherwise get "connection refused".
            let _ = self.core.clear_system_proxy();
            self.tunnel_state = TunnelState::Down;
            self.connect();
        } else {
            self.notify(format!("mode: {}", self.mode));
        }
    }

    fn start_update(&mut self) {
        self.notify("updating…");
        let core = Arc::clone(&self.core);
        let tx = self.event_tx.clone();
        std::thread::spawn(move || {
            let result = core.update().map(|_| ()).map_err(|e| e.to_string());
            let _ = tx.send(AppEvent::Updated(result));
        });
    }

    /// Apply a completed update: the background thread already re-fetched and
    /// replaced the node list, so this only surfaces the result (05).
    fn apply_update(&mut self) {
        self.refresh();
        self.notify("subscription updated");
    }

    fn start_delay_test(&mut self, silent: bool) {
        if !silent {
            self.notify("pinging…");
        }
        self.delay_test_in_flight = true;
        self.delay_refresh_silent = silent;
        let core = Arc::clone(&self.core);
        let tx = self.event_tx.clone();
        std::thread::spawn(move || {
            let _ = core.test_delays();
            let _ = tx.send(AppEvent::DelaysDone);
        });
    }

    /// US27: while the tunnel is up, re-run the delay test once the configured
    /// interval elapses — `delay_refresh_seconds: 0` disables it, and an
    /// in-flight test is never overlapped. Silent, so the notification bar
    /// stays calm on the automatic path.
    fn maybe_auto_refresh_delays(&mut self) {
        let interval = self.settings.delay_refresh_seconds;
        if interval == 0
            || self.delay_test_in_flight
            || !self.tunnel_state.is_up()
            || self.last_delay_refresh.elapsed() < Duration::from_secs(interval)
        {
            return;
        }
        self.last_delay_refresh = Instant::now();
        self.start_delay_test(true);
    }

    fn open_in_editor(&mut self) {
        let Some(entry) = self.settings.subscription(&self.active_subscription) else {
            self.notify("no subscription loaded");
            return;
        };
        let file = expand_home(&entry.file);
        self.notify(format!("editing {}…", file.display()));
        match self.core.edit_file(&file) {
            Ok(()) => {
                self.notify("reloading after edit");
                match self.core.reload() {
                    Ok(()) => self.notify("reloaded"),
                    Err(e) => self.notify(format!("reload failed: {}", e)),
                }
            }
            Err(e) => self.notify(format!("edit failed: {}", e)),
        }
        self.refresh();
    }

    /// Activate the subscription `delta` positions away (Tab / Shift+Tab cycle
    /// forward and back). Disconnects first, like selecting from the menu.
    fn switch_subscription(&mut self, delta: isize) {
        if self.settings.subscriptions.len() < 2 {
            self.notify("only one subscription");
            return;
        }
        let len = self.settings.subscriptions.len() as isize;
        let current = self
            .settings
            .subscriptions
            .iter()
            .position(|s| s.name == self.active_subscription)
            .unwrap_or(0) as isize;
        let next = (current + delta).rem_euclid(len) as usize;
        if self.tunnel_state.is_up() {
            let _ = self.core.disconnect();
            self.tunnel_state = TunnelState::Down;
        }
        self.active_subscription = self.settings.subscriptions[next].name.clone();
        self.load_active_subscription();
        self.notify(format!("subscription: {}", self.active_subscription));
        self.refresh();
    }

    fn settings_path(&self) -> PathBuf {
        self.config_dir.join("settings.yaml")
    }

    fn toggle_manage(&mut self) {
        if self.draft.is_some() {
            self.draft = None;
            return;
        }
        self.manage_subscriptions = !self.manage_subscriptions;
        if self.manage_subscriptions {
            self.showing_rules = false;
            self.selected_subscription = self
                .settings
                .subscriptions
                .iter()
                .position(|s| s.name == self.active_subscription)
                .unwrap_or(0);
        }
    }

    fn move_subscription_selection(&mut self, delta: isize) {
        if self.settings.subscriptions.is_empty() {
            return;
        }
        let len = self.settings.subscriptions.len() as isize;
        let next = (self.selected_subscription as isize + delta).rem_euclid(len) as usize;
        self.selected_subscription = next;
    }

    /// Make the highlighted subscription active (Enter in the manage view).
    /// Like `s`, disconnects first so `set_subscription` is safe.
    fn select_subscription(&mut self) {
        let Some(sub) = self
            .settings
            .subscriptions
            .get(self.selected_subscription)
            .cloned()
        else {
            return;
        };
        if sub.name == self.active_subscription {
            self.toggle_manage();
            return;
        }
        if self.tunnel_state.is_up() {
            let _ = self.core.disconnect();
            self.tunnel_state = TunnelState::Down;
        }
        self.active_subscription = sub.name.clone();
        self.load_active_subscription();
        self.notify(format!("subscription: {}", sub.name));
        self.toggle_manage();
        self.refresh();
    }

    fn open_draft(&mut self, index: Option<usize>) {
        if !self.manage_subscriptions {
            return;
        }
        match index {
            None => {
                self.draft = Some(SubscriptionDraft {
                    index: None,
                    name: String::new(),
                    url: String::new(),
                    auto_update: false,
                    focus: 0,
                })
            }
            Some(i) => {
                let Some(sub) = self.settings.subscriptions.get(i).cloned() else {
                    return;
                };
                self.draft = Some(SubscriptionDraft {
                    index: Some(i),
                    name: sub.name,
                    url: sub.update_url.unwrap_or_default(),
                    auto_update: sub.auto_update,
                    focus: 0,
                });
            }
        }
    }

    fn form_char(&mut self, ch: char) {
        let Some(draft) = &mut self.draft else {
            return;
        };
        match draft.focus {
            0 => draft.name.push(ch),
            1 => draft.url.push(ch),
            _ => draft.auto_update = !draft.auto_update,
        }
    }

    fn form_backspace(&mut self) {
        let Some(draft) = &mut self.draft else {
            return;
        };
        match draft.focus {
            0 => {
                draft.name.pop();
            }
            1 => {
                draft.url.pop();
            }
            _ => draft.auto_update = !draft.auto_update,
        }
    }

    /// Save the draft: mutate `settings.subscriptions`, persist settings.yaml,
    /// make the result the active subscription (when the tunnel is down), and
    /// fetch the file for a brand-new URL subscription.
    fn commit_draft(&mut self) {
        let Some(draft) = self.draft.take() else {
            return;
        };
        let editing_active = match draft.index {
            Some(i) => self
                .settings
                .subscriptions
                .get(i)
                .map(|s| s.name == self.active_subscription)
                .unwrap_or(false),
            None => false,
        };
        if editing_active && self.tunnel_state.is_up() {
            self.notify("disconnect before editing the active subscription");
            self.draft = Some(draft);
            return;
        }
        let name = draft.name.trim().to_string();
        if name.is_empty() {
            self.notify("name is required");
            self.draft = Some(draft);
            return;
        }
        let duplicate = self
            .settings
            .subscriptions
            .iter()
            .enumerate()
            .any(|(i, s)| s.name == name && Some(i) != draft.index);
        if duplicate {
            self.notify(format!("subscription '{}' already exists", name));
            self.draft = Some(draft);
            return;
        }
        let url = if draft.url.trim().is_empty() {
            None
        } else {
            Some(draft.url.trim().to_string())
        };
        let sub = match draft.index {
            Some(i) => {
                let mut s = self.settings.subscriptions[i].clone();
                s.name = name.clone();
                s.update_url = url;
                s.auto_update = draft.auto_update;
                s
            }
            None => settings::SubscriptionEntry {
                name: name.clone(),
                update_url: url,
                file: settings::subscription_file(&self.config_dir, &name),
                auto_update: draft.auto_update,
                default_node: None,
            },
        };
        let index = draft.index;
        let old_name = index.map(|i| self.settings.subscriptions[i].name.clone());
        if let Some(i) = index {
            self.settings.subscriptions[i] = sub.clone();
        } else {
            let dir = self.config_dir.join("subscriptions");
            let _ = fs::create_dir_all(&dir);
            self.settings.subscriptions.push(sub.clone());
        }
        if let Some(old) = old_name.clone().filter(|old| old != &name) {
            let mut state = settings::load_state(&self.state_path()).unwrap_or_default();
            if let Some(node) = state.active_nodes.remove(&old) {
                state.active_nodes.insert(name.clone(), node);
            }
            let _ = settings::save_state(&self.state_path(), &state);
        }
        if let Err(e) = settings::save_settings(&self.settings_path(), &self.settings) {
            self.notify(format!("could not save settings: {}", e));
            self.manage_subscriptions = false;
            return;
        }
        let is_new = index.is_none();
        let was_active = old_name.as_deref() == Some(self.active_subscription.as_str());
        if is_new || was_active {
            if self.tunnel_state.is_up() {
                self.notify("disconnect before changing the active subscription");
            } else {
                self.active_subscription = sub.name.clone();
                if is_new && sub.update_url.is_some() {
                    // File likely doesn't exist yet: point the core at the new
                    // entry, then fetch it (update writes the file and loads).
                    let _ = self.core.set_subscription(&sub);
                    self.start_update();
                } else {
                    self.load_active_subscription();
                }
            }
        }
        self.selected_subscription = self
            .settings
            .subscriptions
            .iter()
            .position(|s| s.name == name)
            .unwrap_or(self.selected_subscription);
        self.notify(format!("saved subscription: {}", name));
        self.toggle_manage();
        self.refresh();
    }

    fn delete_subscription(&mut self, index: usize) {
        let Some(sub) = self.settings.subscriptions.get(index).cloned() else {
            return;
        };
        if sub.name == self.active_subscription && self.tunnel_state.is_up() {
            self.notify("disconnect before removing the active subscription");
            return;
        }
        self.settings.subscriptions.remove(index);
        let mut state = settings::load_state(&self.state_path()).unwrap_or_default();
        state.active_nodes.remove(&sub.name);
        let _ = settings::save_state(&self.state_path(), &state);
        if let Err(e) = settings::save_settings(&self.settings_path(), &self.settings) {
            self.notify(format!("could not save settings: {}", e));
            return;
        }
        if sub.name == self.active_subscription {
            self.active_subscription = self
                .settings
                .subscriptions
                .first()
                .map(|s| s.name.clone())
                .unwrap_or_default();
            if !self.active_subscription.is_empty() {
                self.load_active_subscription();
            }
        }
        if self.selected_subscription >= self.settings.subscriptions.len() {
            self.selected_subscription = self.settings.subscriptions.len().saturating_sub(1);
        }
        self.notify(format!("deleted subscription: {}", sub.name));
        self.refresh();
    }

    fn save_active_node(&self, name: &str) {
        let mut state = settings::load_state(&self.state_path()).unwrap_or_default();
        state
            .active_nodes
            .insert(self.active_subscription.clone(), name.to_string());
        let _ = settings::save_state(&self.state_path(), &state);
    }

    pub fn reattach_if_possible(&mut self) {
        match self.core.reattach() {
            Ok(true) => {
                self.refresh();
                self.notify("re-attached to running tunnel");
            }
            Ok(false) => {}
            Err(e) => self.notify(format!("re-attach failed: {}", e)),
        }
    }

    pub fn maybe_auto_update(&mut self) {
        let auto = self
            .settings
            .subscription(&self.active_subscription)
            .map(|s| s.auto_update)
            .unwrap_or(false);
        if auto {
            self.start_update();
        }
    }

    /// Drain events from worker threads, updating UI state. Call once per tick.
    pub fn poll_events(&mut self) {
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                AppEvent::ConnectResult(result, mode) => self.apply_connect_result(result, mode),
                AppEvent::Updated(Ok(())) => self.apply_update(),
                AppEvent::Updated(Err(e)) => self.notify(format!("update failed: {}", e)),
                AppEvent::DelaysDone => {
                    let silent = self.delay_refresh_silent;
                    self.delay_test_in_flight = false;
                    // The worker already recorded delays on the core; pulling
                    // them in with refresh() keeps this off the UI thread.
                    self.refresh();
                    if !silent {
                        self.notify("pings updated");
                    }
                }
                AppEvent::Traffic(sample) => self.push_traffic(sample),
            }
        }
        self.maybe_auto_refresh_delays();
        for (_, ticks) in self.notifications.iter_mut() {
            *ticks = ticks.saturating_sub(1);
        }
        self.notifications.retain(|(_, ticks)| *ticks > 0);
    }

    fn push_traffic(&mut self, sample: TrafficSample) {
        self.total_up = self.total_up.saturating_add(sample.up);
        self.total_down = self.total_down.saturating_add(sample.down);
        self.traffic.push_back(sample);
        while self.traffic.len() > TRAFFIC_RING {
            self.traffic.pop_front();
        }
    }

    #[cfg(test)]
    /// Wait (bounded) for a background event to land — test helper.
    pub fn drain_until<F: Fn(&mut App) -> bool>(&mut self, deadline: std::time::Duration, done: F) {
        let start = std::time::Instant::now();
        while start.elapsed() < deadline {
            self.poll_events();
            if done(self) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
}

fn expand_home(path: &std::path::Path) -> PathBuf {
    if path.starts_with("~") {
        if let Some(home) = dirs::home_dir() {
            let stripped = path.strip_prefix("~").unwrap_or(path);
            return home.join(stripped);
        }
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::fake::{FakeCore, FakeWorld};
    use std::sync::Mutex;

    fn app_with(fake: FakeCore) -> App {
        app_at("/tmp/zenith-app-test", fake)
    }

    /// Connect and wait for the background thread's result to land.
    fn connect(app: &mut App) {
        app.dispatch(Action::Connect);
        app.drain_until(std::time::Duration::from_secs(2), |a| {
            a.tunnel_state.is_up()
        });
        assert!(app.tunnel_state.is_up(), "connect never finished");
    }

    fn app_at(dir: &str, fake: FakeCore) -> App {
        fs::create_dir_all(dir).unwrap();
        let settings = Settings {
            subscriptions: vec![settings::SubscriptionEntry {
                name: "work".into(),
                update_url: Some("https://example.com/sub".into()),
                file: "/nonexistent/work.json".into(),
                auto_update: false,
                default_node: None,
            }],
            default_subscription: Some("work".into()),
            default_mode: Mode::Tun,
            system_proxy: false,
            proxy_backend: settings::ProxyBackend::Gnome,
            theme: "default".into(),
            nerd_fonts: settings::NerdFonts::Off,
            delay_refresh_seconds: 300,
        };
        App::new(settings, Arc::new(fake), PathBuf::from(dir))
    }

    #[test]
    fn app_loads_subscription_nodes_on_start() {
        let app = app_with(FakeCore::fresh());
        assert_eq!(app.nodes.len(), 3);
        assert!(app.active_subscription == "work");
    }

    #[test]
    fn connect_surfaces_state_and_notification() {
        let mut app = app_with(FakeCore::fresh());
        connect(&mut app);
        assert!(app.tunnel_state.is_up());
        assert!(app
            .notifications
            .iter()
            .any(|(m, _)| m.contains("connected")));
        app.dispatch(Action::Disconnect);
        assert_eq!(app.tunnel_state, TunnelState::Down);
    }

    #[test]
    fn select_node_changes_core_active() {
        let mut app = app_with(FakeCore::fresh());
        let expected = app.nodes[2].name.clone();
        app.selected_node = 2;
        app.dispatch(Action::SelectNode);
        assert_eq!(app.core.active_node().unwrap(), expected);
    }

    #[test]
    fn toggle_mode_flips_mode() {
        let mut app = app_with(FakeCore::fresh());
        assert_eq!(app.mode, Mode::Tun);
        app.dispatch(Action::ToggleMode);
        assert_eq!(app.mode, Mode::Proxy);
    }

    #[test]
    fn update_replaces_node_list_synchronously() {
        let mut app = app_with(FakeCore::fresh());
        app.dispatch(Action::Update);
        app.drain_until(std::time::Duration::from_secs(2), |a| {
            a.nodes.iter().any(|n| n.name.ends_with("v1"))
        });
        assert!(app.nodes[0].name.starts_with("work-node-0-v1"));
        assert!(app.notifications.iter().any(|(m, _)| m.contains("updated")));
    }

    #[test]
    fn delayed_events_land_via_poll() {
        let mut app = app_with(FakeCore::fresh());
        app.dispatch(Action::PingAll);
        app.drain_until(std::time::Duration::from_secs(2), |a| {
            a.nodes.iter().any(|n| n.delay.is_some())
        });
        assert!(app.nodes.iter().all(|n| n.delay.is_some()));
    }

    #[test]
    fn auto_refresh_disabled_when_interval_is_zero() {
        let mut app = app_with(FakeCore::fresh());
        app.settings.delay_refresh_seconds = 0;
        app.last_delay_refresh = Instant::now() - Duration::from_secs(60);
        connect(&mut app);
        app.maybe_auto_refresh_delays();
        assert!(!app.delay_test_in_flight);
    }

    #[test]
    fn auto_refresh_skipped_when_tunnel_down() {
        let mut app = app_with(FakeCore::fresh());
        app.settings.delay_refresh_seconds = 1;
        app.last_delay_refresh = Instant::now() - Duration::from_secs(2);
        app.maybe_auto_refresh_delays();
        assert!(!app.delay_test_in_flight);
    }

    #[test]
    fn auto_refresh_skipped_before_interval_elapses() {
        let mut app = app_with(FakeCore::fresh());
        app.settings.delay_refresh_seconds = 300;
        connect(&mut app);
        app.maybe_auto_refresh_delays();
        assert!(!app.delay_test_in_flight);
    }

    #[test]
    fn auto_refresh_skipped_while_test_in_flight() {
        let mut app = app_with(FakeCore::fresh());
        app.settings.delay_refresh_seconds = 1;
        connect(&mut app);
        app.last_delay_refresh = Instant::now() - Duration::from_secs(2);
        app.delay_test_in_flight = true;
        app.maybe_auto_refresh_delays();
        assert!(app.delay_test_in_flight);
    }

    #[test]
    fn auto_refresh_updates_delays_after_interval() {
        let mut app = app_with(FakeCore::fresh());
        app.settings.delay_refresh_seconds = 1;
        connect(&mut app);
        app.last_delay_refresh = Instant::now() - Duration::from_secs(2);
        app.maybe_auto_refresh_delays();
        assert!(app.delay_test_in_flight);
        app.drain_until(std::time::Duration::from_secs(2), |a| {
            a.nodes.iter().any(|n| n.delay.is_some())
        });
        assert!(app.nodes.iter().all(|n| n.delay.is_some()));
        assert!(!app.delay_test_in_flight);
        assert!(
            app.notifications
                .iter()
                .all(|(m, _)| !m.contains("pinging")),
            "auto refresh must not spam the notification bar"
        );
    }

    #[test]
    fn open_editor_reloads_changes_from_file() {
        let dir = std::env::temp_dir().join(format!("zenith-editor-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("sub.json");
        std::fs::write(&file, r#"{"outbounds":[{"type":"vless","tag":"old"}]}"#).unwrap();
        let fake = FakeCore::fresh();
        let entry = settings::SubscriptionEntry {
            name: "work".into(),
            update_url: None,
            file: file.clone(),
            auto_update: false,
            default_node: None,
        };
        let settings = Settings {
            subscriptions: vec![entry],
            default_subscription: Some("work".into()),
            default_mode: Mode::Tun,
            system_proxy: false,
            proxy_backend: settings::ProxyBackend::Gnome,
            theme: "default".into(),
            nerd_fonts: settings::NerdFonts::Off,
            delay_refresh_seconds: 300,
        };
        let mut app = App::new(settings, Arc::new(fake), dir.clone());
        // Simulate the user editing the file, then reload.
        std::fs::write(
            &file,
            r#"{"outbounds":[{"type":"vless","tag":"new"},{"type":"vless","tag":"newer"}]}"#,
        )
        .unwrap();
        app.core.reload().unwrap();
        app.refresh();
        assert_eq!(app.nodes.len(), 2);
        assert_eq!(app.nodes[0].name, "new");
    }

    #[test]
    fn switch_subscription_cycles_and_loads() {
        let settings = Settings {
            subscriptions: vec![
                settings::SubscriptionEntry {
                    name: "a".into(),
                    update_url: None,
                    file: "/nonexistent/a.json".into(),
                    auto_update: false,
                    default_node: None,
                },
                settings::SubscriptionEntry {
                    name: "b".into(),
                    update_url: None,
                    file: "/nonexistent/b.json".into(),
                    auto_update: false,
                    default_node: None,
                },
            ],
            default_subscription: Some("a".into()),
            default_mode: Mode::Tun,
            system_proxy: false,
            proxy_backend: settings::ProxyBackend::Gnome,
            theme: "default".into(),
            nerd_fonts: settings::NerdFonts::Off,
            delay_refresh_seconds: 300,
        };
        let mut app = App::new(
            settings,
            Arc::new(FakeCore::fresh()),
            PathBuf::from("/tmp/zenith-app-test"),
        );
        app.dispatch(Action::SwitchSubscription);
        assert_eq!(app.active_subscription, "b");
        assert!(app.nodes[0].name.starts_with("b-node-0"));
    }

    #[test]
    fn quit_detaches_when_connected() {
        let fake = FakeCore::fresh();
        let world = fake.world();
        let mut app = app_with(fake);
        connect(&mut app);
        app.dispatch(Action::Quit);
        assert!(app.should_exit);
        let world = world.lock().unwrap();
        assert!(world.detached);
        assert!(world.tunnel.is_up());
    }

    #[test]
    fn reattach_recovers_live_tunnel() {
        let world = Arc::new(Mutex::new(FakeWorld::default()));
        let first = FakeCore::new(Arc::clone(&world));
        let mut app_a = app_with(first);
        connect(&mut app_a);
        app_a.dispatch(Action::Quit);

        let second = FakeCore::new(Arc::clone(&world));
        let mut app_b = app_with(second);
        app_b.reattach_if_possible();
        assert!(app_b.tunnel_state.is_up());
        assert!(app_b
            .notifications
            .iter()
            .any(|(m, _)| m.contains("re-attached")));
    }

    #[test]
    fn traffic_ring_bounds_and_order() {
        let mut app = app_with(FakeCore::fresh());
        for i in 0..(TRAFFIC_RING + 20) {
            app.push_traffic(TrafficSample {
                up: i as u64,
                down: 0,
            });
        }
        assert_eq!(app.traffic.len(), TRAFFIC_RING);
        assert_eq!(app.traffic.front().unwrap().up, 20);
        assert_eq!(app.traffic.back().unwrap().up, TRAFFIC_RING as u64 + 19);
    }

    #[test]
    fn traffic_totals_accumulate_and_reset() {
        let mut app = app_with(FakeCore::fresh());
        app.push_traffic(TrafficSample { up: 100, down: 300 });
        app.push_traffic(TrafficSample { up: 50, down: 25 });
        assert_eq!(app.total_up, 150);
        assert_eq!(app.total_down, 325);
        app.dispatch(Action::ResetTraffic);
        assert_eq!(app.total_up, 0);
        assert_eq!(app.total_down, 0);
        assert!(app.notifications.iter().any(|(m, _)| m.contains("reset")));
    }

    #[test]
    fn toggle_connect_connects_then_disconnects() {
        let mut app = app_with(FakeCore::fresh());
        app.dispatch(Action::ToggleConnect);
        app.drain_until(std::time::Duration::from_secs(2), |a| {
            a.tunnel_state.is_up()
        });
        assert!(app.tunnel_state.is_up());
        app.dispatch(Action::ToggleConnect);
        assert_eq!(app.tunnel_state, TunnelState::Down);
    }

    #[test]
    fn ping_runs_once_in_the_worker_not_on_the_ui_thread() {
        let dir = std::env::temp_dir().join(format!("zenith-ping-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let settings = Settings {
            subscriptions: vec![settings::SubscriptionEntry {
                name: "work".into(),
                update_url: None,
                file: "/nonexistent/work.json".into(),
                auto_update: false,
                default_node: None,
            }],
            default_subscription: Some("work".into()),
            default_mode: Mode::Tun,
            system_proxy: false,
            proxy_backend: settings::ProxyBackend::Gnome,
            theme: "default".into(),
            nerd_fonts: settings::NerdFonts::Off,
            delay_refresh_seconds: 300,
        };
        let fake = Arc::new(FakeCore::fresh());
        let core: Arc<dyn Core> = fake.clone();
        let mut app = App::new(settings, core, dir);
        connect(&mut app);
        app.dispatch(Action::PingAll);
        app.drain_until(std::time::Duration::from_secs(2), |a| {
            !a.delay_test_in_flight && a.nodes.iter().all(|n| n.delay.is_some())
        });
        assert_eq!(
            fake.delay_test_count(),
            1,
            "ping must run once, in the worker thread"
        );
    }

    #[test]
    fn log_scroll_moves_and_toggles_with_log_view() {
        let mut app = app_with(FakeCore::fresh());
        app.dispatch(Action::ToggleLogs);
        assert!(app.showing_logs);
        app.dispatch(Action::Up);
        app.dispatch(Action::Up);
        assert_eq!(app.log_scroll, 2);
        app.dispatch(Action::Down);
        assert_eq!(app.log_scroll, 1);
        app.dispatch(Action::ToggleLogs);
        assert!(!app.showing_logs);
        assert_eq!(app.log_scroll, 0);
    }

    fn manage(app: &mut App) {
        app.dispatch(Action::ToggleManage);
        assert!(app.manage_subscriptions);
    }

    fn type_field(app: &mut App, chars: &str) {
        for ch in chars.chars() {
            app.dispatch(Action::FormChar(ch));
        }
    }

    #[test]
    fn add_subscription_persists_activates_and_fetches() {
        let mut app = app_at("/tmp/zenith-app-add-test", FakeCore::fresh());
        manage(&mut app);
        app.dispatch(Action::AddSubscription);
        assert!(app.draft.is_some());
        type_field(&mut app, "new-sub");
        app.dispatch(Action::FormFieldNext);
        type_field(&mut app, "https://example.com/new");
        app.dispatch(Action::FormFieldNext);
        app.dispatch(Action::FormChar(' '));
        app.dispatch(Action::FormSubmit);
        assert!(app.draft.is_none());
        assert!(!app.manage_subscriptions);
        assert_eq!(app.active_subscription, "new-sub");
        assert_eq!(app.settings.subscriptions.len(), 2);
        let sub = &app.settings.subscriptions[1];
        assert_eq!(sub.name, "new-sub");
        assert_eq!(sub.update_url.as_deref(), Some("https://example.com/new"));
        assert!(sub.auto_update);
        assert_eq!(
            sub.file,
            settings::subscription_file(&app.config_dir, "new-sub")
        );
        // The URL subscription fetched its file through the fake core.
        app.drain_until(std::time::Duration::from_secs(2), |a| {
            a.nodes.iter().any(|n| n.name.ends_with("v1"))
        });
        assert!(app.nodes[0].name.starts_with("new-sub-node-0-v1"));
        // And settings.yaml was persisted.
        let on_disk = settings::load_settings(&app.settings_path()).unwrap();
        assert_eq!(on_disk.subscriptions.len(), 2);
        assert_eq!(on_disk.subscriptions[1].name, "new-sub");
    }

    #[test]
    fn edit_subscription_renames_keeps_file_and_swaps_state_key() {
        let mut app = app_with(FakeCore::fresh());
        let original_file = app.settings.subscriptions[0].file.clone();
        manage(&mut app);
        app.dispatch(Action::EditSubscription);
        for _ in 0..4 {
            app.dispatch(Action::FormBackspace);
        }
        type_field(&mut app, "home");
        app.dispatch(Action::FormSubmit);
        assert_eq!(app.active_subscription, "home");
        assert_eq!(app.settings.subscriptions.len(), 1);
        let sub = &app.settings.subscriptions[0];
        assert_eq!(sub.name, "home");
        assert_eq!(sub.file, original_file, "renaming must not move the file");
        assert!(app.nodes[0].name.starts_with("home-node-0-v0"));
    }

    #[test]
    fn delete_subscription_falls_back_active_to_another() {
        let settings = Settings {
            subscriptions: vec![
                settings::SubscriptionEntry {
                    name: "a".into(),
                    update_url: None,
                    file: "/nonexistent/a.json".into(),
                    auto_update: false,
                    default_node: None,
                },
                settings::SubscriptionEntry {
                    name: "b".into(),
                    update_url: None,
                    file: "/nonexistent/b.json".into(),
                    auto_update: false,
                    default_node: None,
                },
            ],
            default_subscription: Some("a".into()),
            default_mode: Mode::Tun,
            system_proxy: false,
            proxy_backend: settings::ProxyBackend::Gnome,
            theme: "default".into(),
            nerd_fonts: settings::NerdFonts::Off,
            delay_refresh_seconds: 300,
        };
        fs::create_dir_all("/tmp/zenith-app-del-test").unwrap();
        let mut app = App::new(
            settings,
            Arc::new(FakeCore::fresh()),
            PathBuf::from("/tmp/zenith-app-del-test"),
        );
        manage(&mut app);
        app.dispatch(Action::DeleteSubscription);
        assert_eq!(app.settings.subscriptions.len(), 1);
        assert_eq!(app.settings.subscriptions[0].name, "b");
        assert_eq!(app.active_subscription, "b");
        assert!(app.nodes[0].name.starts_with("b-node-0-v0"));
    }

    #[test]
    fn delete_active_while_connected_is_refused() {
        let mut app = app_with(FakeCore::fresh());
        connect(&mut app);
        manage(&mut app);
        app.dispatch(Action::DeleteSubscription);
        assert_eq!(app.settings.subscriptions.len(), 1);
        assert!(app
            .notifications
            .iter()
            .any(|(m, _)| m.contains("disconnect before removing")));
    }

    #[test]
    fn edit_active_while_connected_is_refused() {
        let mut app = app_with(FakeCore::fresh());
        connect(&mut app);
        manage(&mut app);
        app.dispatch(Action::EditSubscription);
        type_field(&mut app, "home");
        app.dispatch(Action::FormSubmit);
        assert!(app.draft.is_some(), "form must stay open while connected");
        assert_eq!(app.settings.subscriptions[0].name, "work");
    }

    #[test]
    fn commit_requires_a_name() {
        let mut app = app_with(FakeCore::fresh());
        manage(&mut app);
        app.dispatch(Action::AddSubscription);
        app.dispatch(Action::FormSubmit);
        assert!(app.draft.is_some(), "empty name must keep the form open");
        assert_eq!(app.settings.subscriptions.len(), 1);
    }

    #[test]
    fn duplicate_name_is_rejected() {
        let mut app = app_with(FakeCore::fresh());
        manage(&mut app);
        app.dispatch(Action::AddSubscription);
        type_field(&mut app, "work");
        app.dispatch(Action::FormSubmit);
        assert!(app.draft.is_some());
        assert_eq!(app.settings.subscriptions.len(), 1);
    }

    #[test]
    fn cancel_closes_the_form_without_changes() {
        let mut app = app_with(FakeCore::fresh());
        manage(&mut app);
        app.dispatch(Action::AddSubscription);
        type_field(&mut app, "discarded");
        app.dispatch(Action::FormCancel);
        assert!(app.draft.is_none());
        assert!(app.manage_subscriptions);
        assert_eq!(app.settings.subscriptions.len(), 1);
    }
}
