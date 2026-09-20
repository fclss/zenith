mod app;
mod core;
mod settings;
mod subscription;
mod symbols;
mod theme;
mod ui;

use std::io;
use std::sync::Arc;
use std::time::Instant;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use app::{Action, App};
use core::fake::FakeCore;
use core::singbox::SingBoxCore;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // `zenith --proxy-env` prints shell exports for `eval`, for console setups
    // without GNOME (see `proxy_backend: env`).
    if std::env::args().any(|a| a == "--proxy-env") {
        print!("{}", core::singbox::proxy_env_script());
        return Ok(());
    }

    let config_dir = settings::config_dir();
    std::fs::create_dir_all(&config_dir)?;
    let settings_path = config_dir.join("settings.yaml");
    let settings = match settings::load_settings(&settings_path) {
        Ok(s) => s,
        Err(_) => {
            let default = default_settings();
            std::fs::write(&settings_path, default)?;
            settings::load_settings(&settings_path)?
        }
    };

    let use_fake = std::env::args().any(|a| a == "--fake");
    let core: Arc<dyn core::Core> = if use_fake {
        Arc::new(FakeCore::fresh())
    } else {
        Arc::new(SingBoxCore::new(config_dir.clone()))
    };

    let mut app = App::new(settings, core, config_dir.clone());
    app.reattach_if_possible();
    app.maybe_auto_update();

    let mut terminal = setup_terminal()?;
    let result = run_loop(&mut terminal, &mut app);
    restore_terminal(&mut terminal)?;
    result
}

fn default_settings() -> String {
    r#"# Zenith settings. Hand-edited; the client only ever reads this file.
# Add or remove subscriptions here — there is no in-app subscription UI.

default_subscription: ""       # optional; first entry is used otherwise
default_mode: tun              # tun | proxy
system_proxy: false            # proxy mode may also write the system proxy
proxy_backend: gnome           # gnome | env | both | none (env = proxy.env for Telegram etc.)
theme: default                 # themes/<name>.yaml under this config dir
nerd_fonts: auto                # auto | on | off — logo glyph if a Nerd Font is present
delay_refresh_seconds: 300        # auto ping-test interval; 0 disables

# subscriptions:
#   - name: work
#     update_url: https://example.com/sub        # omit for a local-only subscription
#     file: ~/.config/zenith/work.json
#     auto_update: true
#     default_node: ""                           # optional
"#
    .to_string()
}

fn setup_terminal() -> io::Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    Terminal::new(CrosstermBackend::new(io::stdout()))
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut last_tick = Instant::now();
    loop {
        terminal.draw(|f| ui::render(f, app))?;
        app.poll_events();
        if app.should_exit {
            break;
        }
        let tick_interval = std::time::Duration::from_millis(50);
        if event::poll(tick_interval)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    let action = if app.draft.is_some() {
                        map_form_key(key)
                    } else if app.showing_help {
                        map_help_key(key)
                    } else if app.manage_subscriptions {
                        map_manage_key(key)
                    } else {
                        map_key(key)
                    };
                    if let Some(action) = action {
                        app.dispatch(action);
                    }
                }
                Event::Resize(..) => {}
                _ => {}
            }
        }
        if last_tick.elapsed() >= tick_interval {
            last_tick = Instant::now();
        }
    }
    Ok(())
}

fn map_key(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => Some(Action::Quit),
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Some(Action::Quit),
        KeyCode::Char('c') => Some(Action::Connect),
        KeyCode::Char('d') => Some(Action::Disconnect),
        KeyCode::Char('t') => Some(Action::ToggleMode),
        KeyCode::Char('u') => Some(Action::Update),
        KeyCode::Char('p') => Some(Action::PingAll),
        KeyCode::Char('r') => Some(Action::ToggleRules),
        KeyCode::Char('l') => Some(Action::ToggleLogs),
        KeyCode::Char('?') | KeyCode::Char('h') => Some(Action::ToggleHelp),
        KeyCode::Char('z') => Some(Action::ResetTraffic),
        KeyCode::Char('e') => Some(Action::OpenEditor),
        KeyCode::Char('s') => Some(Action::ToggleManage),
        KeyCode::Tab => Some(Action::SwitchSubscription),
        KeyCode::BackTab => Some(Action::SwitchSubscriptionPrev),
        KeyCode::Char('j') | KeyCode::Down => Some(Action::Down),
        KeyCode::Char('k') | KeyCode::Up => Some(Action::Up),
        KeyCode::Char(' ') => Some(Action::SelectNode),
        KeyCode::Enter => Some(Action::ToggleConnect),
        _ => None,
    }
}

/// Any key dismisses the help overlay.
fn map_help_key(_key: KeyEvent) -> Option<Action> {
    Some(Action::ToggleHelp)
}

fn map_manage_key(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Char('q') | KeyCode::Char('s') | KeyCode::Esc => Some(Action::ToggleManage),
        KeyCode::Char('a') => Some(Action::AddSubscription),
        KeyCode::Char('e') => Some(Action::EditSubscription),
        KeyCode::Char('x') => Some(Action::DeleteSubscription),
        KeyCode::Enter => Some(Action::SelectSubscription),
        KeyCode::Tab | KeyCode::Char('j') | KeyCode::Down => Some(Action::Down),
        KeyCode::BackTab | KeyCode::Char('k') | KeyCode::Up => Some(Action::Up),
        _ => None,
    }
}

fn map_form_key(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Esc => Some(Action::FormCancel),
        KeyCode::Enter => Some(Action::FormSubmit),
        KeyCode::Tab => Some(Action::FormFieldNext),
        KeyCode::Backspace => Some(Action::FormBackspace),
        KeyCode::Char(' ') => Some(Action::FormChar(' ')),
        KeyCode::Char(c) => Some(Action::FormChar(c)),
        KeyCode::Down => Some(Action::FormFieldNext),
        KeyCode::Up => Some(Action::FormFieldPrev),
        _ => None,
    }
}
