use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Cell, Clear, Paragraph, Row, Table, TableState,
};
use ratatui::Frame;

use crate::app::App;
use crate::settings;
use crate::theme::Theme;

pub fn render(frame: &mut Frame, app: &App) {
    let constraints = if app.notifications.is_empty() {
        vec![
            Constraint::Length(5),
            Constraint::Min(4),
            Constraint::Length(1),
        ]
    } else {
        vec![
            Constraint::Length(5),
            Constraint::Min(3),
            Constraint::Length(1),
            Constraint::Length(1),
        ]
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(frame.area());
    render_header(frame, app, chunks[0]);
    render_main(frame, app, chunks[1]);
    if app.notifications.is_empty() {
        render_footer(frame, app, chunks[2]);
    } else {
        render_notifications(frame, app, chunks[2]);
        render_footer(frame, app, chunks[3]);
    }
    if app.showing_help {
        render_help(frame, app, frame.area());
    }
}

/// A rounded panel in the theme's accent color.
fn panel<'a>(title: impl Into<Line<'a>>, theme: &Theme) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.accent))
        .title(title)
}

/// Left column of the info panel: tunnel state, mode, active node.
fn info_lines<'a>(app: &App) -> Vec<Line<'a>> {
    let theme = &app.theme;
    let sym = &app.symbols;
    let (state, state_color) = match app.tunnel_state {
        crate::core::TunnelState::Up(_) => (format!("{} connected", sym.connected), theme.ok),
        crate::core::TunnelState::Down => (format!("{} disconnected", sym.disconnected), theme.dim),
    };
    let mode = match (app.mode, app.settings.system_proxy) {
        (crate::core::Mode::Proxy, true) => "system proxy".to_string(),
        (crate::core::Mode::Proxy, false) => "proxy".to_string(),
        (crate::core::Mode::Tun, _) => "tun".to_string(),
    };
    let active = app.active_node.clone().unwrap_or_else(|| "-".to_string());
    vec![
        Line::from(vec![
            Span::styled("status: ", Style::default().fg(theme.dim)),
            Span::styled(
                state,
                Style::default()
                    .fg(state_color)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("mode: ", Style::default().fg(theme.dim)),
            Span::styled(mode, Style::default().fg(theme.text)),
        ]),
        Line::from(vec![
            Span::styled("node: ", Style::default().fg(theme.dim)),
            Span::styled(active, Style::default().fg(theme.text)),
        ]),
    ]
}

/// Right column of the info panel: current rates, peak rates, totals.
fn traffic_lines<'a>(app: &App) -> Vec<Line<'a>> {
    let theme = &app.theme;
    let sym = &app.symbols;
    let up = app.traffic.back().map(|t| t.up).unwrap_or(0);
    let down = app.traffic.back().map(|t| t.down).unwrap_or(0);
    let (up_top, down_top) = peaks(app);
    vec![
        Line::from(vec![
            Span::styled("current ", Style::default().fg(theme.dim)),
            Span::styled(
                format!("{} {} ", sym.up, rate_label(up)),
                Style::default().fg(theme.up),
            ),
            Span::styled(
                format!("{} {}", sym.down, rate_label(down)),
                Style::default().fg(theme.down),
            ),
        ]),
        Line::from(vec![
            Span::styled("top ", Style::default().fg(theme.dim)),
            Span::styled(
                format!("{} {} ", sym.up, rate_label(up_top)),
                Style::default().fg(theme.up),
            ),
            Span::styled(
                format!("{} {}", sym.down, rate_label(down_top)),
                Style::default().fg(theme.down),
            ),
        ]),
        Line::from(vec![
            Span::styled("total ", Style::default().fg(theme.dim)),
            Span::styled(
                format!("{} {} ", sym.up, bytes_label(app.total_up)),
                Style::default().fg(theme.up),
            ),
            Span::styled(
                format!("{} {} ", sym.down, bytes_label(app.total_down)),
                Style::default().fg(theme.down),
            ),
            Span::styled("(z reset)", Style::default().fg(theme.dim)),
        ]),
    ]
}

/// The highest upload/download rate in the current history window.
fn peaks(app: &App) -> (u64, u64) {
    (
        app.traffic.iter().map(|t| t.up).max().unwrap_or(0),
        app.traffic.iter().map(|t| t.down).max().unwrap_or(0),
    )
}

/// A per-second rate, always scaled to kB/MB/GB.
fn rate_label(bytes: u64) -> String {
    format!("{}/s", bytes_label(bytes))
}

/// A byte count, always scaled to kB/MB/GB with one decimal.
fn bytes_label(bytes: u64) -> String {
    const KI: f64 = 1024.0;
    let b = bytes as f64;
    if b >= KI * KI * KI {
        format!("{:.1} GB", b / (KI * KI * KI))
    } else if b >= KI * KI {
        format!("{:.1} MB", b / (KI * KI))
    } else {
        format!("{:.1} kB", b / KI)
    }
}

/// Below this width the traffic-stats column is dropped rather than cramped.
const TRAFFIC_STATS_MIN_WIDTH: u16 = 60;

fn render_header(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;
    let outer = panel(format!(" {} ", app.symbols.zenith), theme);
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    frame.render_widget(Paragraph::new(info_lines(app)), inner);

    if area.width >= TRAFFIC_STATS_MIN_WIDTH {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(0), Constraint::Length(38)])
            .split(inner);
        frame.render_widget(Paragraph::new(traffic_lines(app)), cols[1]);
    }
}

fn render_main(frame: &mut Frame, app: &App, area: Rect) {
    if let Some(draft) = &app.draft {
        render_subscription_form(frame, app, draft, area);
    } else if app.manage_subscriptions {
        render_subscriptions(frame, app, area);
    } else if app.showing_logs {
        render_logs(frame, app, area);
    } else if app.showing_rules {
        render_rules(frame, app, area);
    } else {
        render_nodes(frame, app, area);
    }
}

fn render_nodes(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;
    let title = format!(" nodes · {} ", app.active_subscription);
    let block = panel(title, theme);
    let selected_style = Style::default().fg(theme.selected_fg).bg(theme.selected_bg);
    let header = Row::new(vec![Cell::from("node"), Cell::from("ping")])
        .style(Style::default().fg(theme.dim));
    let rows: Vec<Row> = app
        .nodes
        .iter()
        .map(|node| {
            let is_active = app.active_node.as_deref() == Some(node.name.as_str());
            let name = if is_active {
                format!("{} {}", app.symbols.active, node.name)
            } else {
                node.name.clone()
            };
            let ping = node
                .delay
                .map(|d| format!("{} ms", d))
                .unwrap_or_else(|| "—".to_string());
            let style = if is_active {
                Style::default().fg(theme.ok).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text)
            };
            Row::new(vec![Cell::from(name), Cell::from(ping)]).style(style)
        })
        .collect();
    let table = Table::new(
        rows,
        [Constraint::Percentage(80), Constraint::Percentage(20)],
    )
    .header(header)
    .block(block)
    .row_highlight_style(selected_style)
    .highlight_symbol("");
    let mut state = TableState::default();
    if !app.nodes.is_empty() {
        state.select(Some(app.selected_node));
    }
    frame.render_stateful_widget(table, area, &mut state);
}

fn render_rules(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;
    let block = panel(" routing rules ", theme);
    let header = Row::new(vec![Cell::from("rule-set"), Cell::from("policy")])
        .style(Style::default().fg(theme.dim));
    let rows: Vec<Row> = app
        .rules
        .iter()
        .map(|rule| {
            let policy_style = match rule.policy {
                crate::core::RulePolicy::Proxied => Style::default().fg(theme.ok),
                crate::core::RulePolicy::Direct => Style::default().fg(theme.warn),
            };
            Row::new(vec![
                Cell::from(rule.name.clone()),
                Cell::from(rule.policy.as_str()).style(policy_style),
            ])
        })
        .collect();
    let table = Table::new(
        rows,
        [Constraint::Percentage(70), Constraint::Percentage(30)],
    )
    .header(header)
    .block(block);
    frame.render_widget(table, area);
}

fn render_logs(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;
    let content = app.core.log_content();
    let lines: Vec<Line> = content
        .lines()
        .rev()
        .skip(app.log_scroll)
        .take(area.height.saturating_sub(2) as usize)
        .map(|l| Line::from(Span::styled(l.to_string(), Style::default().fg(theme.text))))
        .collect();
    let block = panel(" logs ", theme).border_style(Style::default().fg(theme.dim));
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn render_notifications(frame: &mut Frame, app: &App, area: Rect) {
    let text = app
        .notifications
        .iter()
        .map(|(msg, _)| {
            Line::from(Span::styled(
                msg.as_str(),
                Style::default().fg(app.theme.warn),
            ))
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(text), area);
}

fn render_subscriptions(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;
    let block = panel(" subscriptions ", theme);
    let selected_style = Style::default().fg(theme.selected_fg).bg(theme.selected_bg);
    let header = Row::new(vec![Cell::from("subscription"), Cell::from("update url")])
        .style(Style::default().fg(theme.dim));
    let rows: Vec<Row> = app
        .settings
        .subscriptions
        .iter()
        .map(|sub| {
            let is_active = sub.name == app.active_subscription;
            let marker = if is_active {
                format!("{} ", app.symbols.active)
            } else {
                String::new()
            };
            let auto = if sub.auto_update { " ⟳" } else { "" };
            let name = format!("{}{}{}", marker, sub.name, auto);
            let url = sub
                .update_url
                .clone()
                .unwrap_or_else(|| "— local".to_string());
            let style = if is_active {
                Style::default().fg(theme.ok).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text)
            };
            Row::new(vec![Cell::from(name), Cell::from(url)]).style(style)
        })
        .collect();
    let table = Table::new(
        rows,
        [Constraint::Percentage(45), Constraint::Percentage(55)],
    )
    .header(header)
    .block(block)
    .row_highlight_style(selected_style)
    .highlight_symbol("");
    let mut state = TableState::default();
    if !app.settings.subscriptions.is_empty() {
        state.select(Some(app.selected_subscription));
    }
    frame.render_stateful_widget(table, area, &mut state);
}

fn render_subscription_form(
    frame: &mut Frame,
    app: &App,
    draft: &crate::app::SubscriptionDraft,
    area: Rect,
) {
    let theme = &app.theme;
    let title = if draft.index.is_some() {
        " edit subscription "
    } else {
        " add subscription "
    };
    let block = panel(title, theme);
    let focus_style = Style::default().fg(theme.selected_fg).bg(theme.selected_bg);
    let label = |text: &str, value: String, focused: bool| {
        let value_style = if focused {
            focus_style
        } else {
            Style::default().fg(theme.text)
        };
        Line::from(vec![
            Span::styled(format!(" {:<11} ", text), Style::default().fg(theme.dim)),
            Span::styled(format!(" [{}] ", value), value_style),
        ])
    };
    let auto_value = if draft.auto_update { "yes" } else { "no " };
    let mut lines = vec![label("name", draft.name.clone(), draft.focus == 0)];
    lines.push(label("update url", draft.url.clone(), draft.focus == 1));
    lines.push(label(
        "auto update",
        auto_value.to_string(),
        draft.focus == 2,
    ));
    lines.push(Line::from(Span::styled(
        format!(
            " file will be {}",
            settings::subscription_file(
                &app.config_dir,
                if draft.name.is_empty() {
                    "_"
                } else {
                    &draft.name
                }
            )
            .display()
        ),
        Style::default().fg(theme.dim),
    )));
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

/// The keybinding reference shown by `?`. Kept in one place so the footer hint
/// and the overlay cannot drift.
const BINDINGS: &[(&str, &str)] = &[
    ("↑ / k, ↓ / j", "move selection · scroll logs"),
    ("↵ (enter)", "connect / disconnect"),
    ("space", "switch to the selected node"),
    ("c / d", "connect / disconnect"),
    ("t", "toggle tun / proxy mode"),
    ("tab / ⇧tab", "next / previous subscription"),
    ("p", "ping all nodes"),
    ("u", "update subscription"),
    ("z", "reset traffic totals"),
    ("r", "routing rules"),
    ("l", "logs"),
    ("e", "edit subscription file in $EDITOR"),
    ("s", "subscriptions"),
    ("? / h", "this help"),
    ("q / esc", "quit (detaches the tunnel)"),
];

fn render_help(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;
    let width = 54.min(area.width);
    let height = (BINDINGS.len() as u16 + 4).min(area.height);
    let popup = centered(area, width, height);
    frame.render_widget(Clear, popup);
    let block = panel(" help · any key to close ", theme);
    let lines: Vec<Line> = BINDINGS
        .iter()
        .map(|(key, desc)| {
            Line::from(vec![
                Span::styled(
                    format!(" {:<14}", key),
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(*desc, Style::default().fg(theme.text)),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(area.height.saturating_sub(height) / 2),
            Constraint::Length(height),
            Constraint::Min(0),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(area.width.saturating_sub(width) / 2),
            Constraint::Length(width),
            Constraint::Min(0),
        ])
        .split(vert[1])[1]
}

/// Clip `s` to `width` columns, replacing the last with an ellipsis if cut.
fn truncate_to(s: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if s.chars().count() <= width {
        return s.to_string();
    }
    let mut out: String = s.chars().take(width - 1).collect();
    out.push('…');
    out
}

fn render_footer(frame: &mut Frame, app: &App, area: Rect) {
    let hint = if app.draft.is_some() {
        " type to edit · tab/↑↓ fields · space/type toggles auto · ↵ save · esc cancel "
    } else if app.showing_help {
        " any key closes help "
    } else if app.manage_subscriptions {
        " ↑↓/tab select · ⇧tab up · ↵ activate · a add · e edit · x delete · s/esc back "
    } else if app.showing_logs {
        " ↑↓ scroll · l close logs · ? help "
    } else if app.showing_rules {
        " r rules · l logs · e edit · u update · s subscriptions · q quit "
    } else {
        " ↑↓ select · ⇥/⇧⇥ subscription · space switch · ↵ connect/disconnect · c/d · t mode · p ping · u update · r rules · l logs · s subs · ? help · q quit "
    };
    let hint = truncate_to(hint, area.width as usize);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            hint,
            Style::default().fg(app.theme.dim),
        ))),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Action, App};
    use crate::core::fake::FakeCore;
    use crate::settings::{self, Settings};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn app() -> App {
        let settings = Settings {
            subscriptions: vec![settings::SubscriptionEntry {
                name: "work".into(),
                update_url: None,
                file: "/nonexistent/work.json".into(),
                auto_update: false,
                default_node: None,
            }],
            default_subscription: Some("work".into()),
            default_mode: crate::core::Mode::Tun,
            system_proxy: false,
            proxy_backend: settings::ProxyBackend::Gnome,
            theme: "default".into(),
            nerd_fonts: settings::NerdFonts::Off,
            delay_refresh_seconds: 300,
        };
        App::new(
            settings,
            std::sync::Arc::new(FakeCore::fresh()),
            "/tmp/zenith-ui-test".into(),
        )
    }

    fn render_to_string(app: &App) -> String {
        let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
        terminal.draw(|frame| render(frame, app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect::<String>()
    }

    #[test]
    fn header_shows_disconnected_and_node_list_shows_nodes() {
        let out = render_to_string(&app());
        assert!(out.contains("disconnected"));
        assert!(out.contains("work-node-0-v0"));
        assert!(out.contains("work-node-1-v0"));
        assert!(out.contains("work-node-2-v0"));
    }

    #[test]
    fn header_shows_connected_after_connect() {
        let mut app = app();
        app.dispatch(Action::Connect);
        app.drain_until(std::time::Duration::from_secs(2), |a| {
            a.tunnel_state.is_up()
        });
        let out = render_to_string(&app);
        assert!(out.contains("connected"));
        assert!(out.contains("tun"));
    }

    #[test]
    fn header_shows_info_left_and_traffic_stats_right() {
        let out = render_to_string(&app());
        assert!(out.contains("zenith"), "block title");
        assert!(out.contains("status:"), "status row");
        assert!(out.contains("mode:"), "mode row");
        assert!(out.contains("node:"), "node row");
        assert!(out.contains("current"), "traffic current row");
        assert!(out.contains("top"), "traffic top row");
        assert!(out.contains("total"), "traffic total row");
    }

    #[test]
    fn traffic_stats_hidden_when_narrow() {
        let mut terminal = Terminal::new(TestBackend::new(40, 20)).unwrap();
        terminal.draw(|frame| render(frame, &app())).unwrap();
        let out: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(out.contains("status:"));
        assert!(!out.contains("current"), "stats need room to render");
    }

    #[test]
    fn system_proxy_mode_reads_system_proxy() {
        let mut app = app();
        app.mode = crate::core::Mode::Proxy;
        app.settings.system_proxy = true;
        let out = render_to_string(&app);
        assert!(out.contains("system proxy"));
    }

    #[test]
    fn rate_label_scales_to_kb() {
        assert_eq!(rate_label(2048), "2.0 kB/s");
        assert_eq!(rate_label(2 * 1024 * 1024), "2.0 MB/s");
    }

    #[test]
    fn truncate_to_cuts_with_an_ellipsis() {
        assert_eq!(truncate_to("hello", 10), "hello");
        assert_eq!(truncate_to("hello world", 5), "hell…");
        assert_eq!(truncate_to("abc", 0), "");
    }

    #[test]
    fn bytes_label_is_always_kb_or_above() {
        assert_eq!(bytes_label(512), "0.5 kB");
        assert_eq!(bytes_label(1536), "1.5 kB");
        assert_eq!(bytes_label(3 * 1024 * 1024), "3.0 MB");
    }

    #[test]
    fn nodes_header_says_ping_not_delay() {
        let out = render_to_string(&app());
        assert!(out.contains("ping"), "column renamed to ping");
        assert!(!out.contains("delay"), "old column label is gone");
    }

    #[test]
    fn help_overlay_lists_bindings() {
        let mut app = app();
        app.dispatch(Action::ToggleHelp);
        let out = render_to_string(&app);
        assert!(out.contains("help"));
        assert!(out.contains("connect / disconnect"));
        assert!(out.contains("reset traffic totals"));
    }

    #[test]
    fn rules_view_renders_policies() {
        let mut app = app();
        app.dispatch(Action::ToggleRules);
        let out = render_to_string(&app);
        assert!(out.contains("rule-set"));
        assert!(out.contains("proxied"));
    }

    #[test]
    fn manage_view_lists_subscriptions() {
        let mut app = app();
        app.dispatch(Action::ToggleManage);
        let out = render_to_string(&app);
        assert!(out.contains("subscriptions"));
        assert!(out.contains("update url"));
        assert!(out.contains("work"));
    }

    #[test]
    fn add_form_renders_fields() {
        let mut app = app();
        app.dispatch(Action::ToggleManage);
        app.dispatch(Action::AddSubscription);
        let out = render_to_string(&app);
        assert!(out.contains("add subscription"));
        assert!(out.contains("name"));
        assert!(out.contains("update url"));
        assert!(out.contains("auto update"));
        assert!(out.contains("subscriptions/"));
    }

    #[test]
    fn edit_form_prefills_name() {
        let mut app = app();
        app.dispatch(Action::ToggleManage);
        app.dispatch(Action::EditSubscription);
        let out = render_to_string(&app);
        assert!(out.contains("edit subscription"));
        assert!(out.contains("work"));
    }
}
