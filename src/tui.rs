use anyhow::{Context, Result};
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};
use std::{
    io::{self, Stdout},
    time::Duration,
};

use crate::fleet::{FleetDevice, fetch_devices};

pub fn run(registry_url: &str, token: &str) -> Result<()> {
    let mut devices = fetch_devices(registry_url, token).context("load fleet devices")?;
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let result = run_loop(&mut terminal, registry_url, token, &mut devices);
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    registry_url: &str,
    token: &str,
    devices: &mut Vec<FleetDevice>,
) -> Result<()> {
    let mut state = ListState::default();
    if !devices.is_empty() {
        state.select(Some(0));
    }
    let mut status = format!("Connected to {registry_url}. Enter opens browser URL hint; q quits.");

    loop {
        terminal.draw(|frame| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(5), Constraint::Length(3)])
                .split(frame.area());

            let items: Vec<ListItem> = devices
                .iter()
                .map(|device| {
                    let mark = if device.online { "online " } else { "offline" };
                    ListItem::new(Line::from(vec![
                        Span::raw(format!("{mark}  ")),
                        Span::styled(
                            device.display_name.clone(),
                            Style::default().add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(format!("  {}  {}", device.os, device.shell_url)),
                    ]))
                })
                .collect();

            let list = List::new(items)
                .block(Block::default().borders(Borders::ALL).title("Russht fleet"))
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
            frame.render_stateful_widget(list, chunks[0], &mut state);

            let help = Paragraph::new(status.as_str())
                .wrap(Wrap { trim: true })
                .block(Block::default().borders(Borders::ALL).title("status"));
            frame.render_widget(help, chunks[1]);
        })?;

        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('r') => {
                        match fetch_devices(registry_url, token) {
                            Ok(next) => {
                                *devices = next;
                                if devices.is_empty() {
                                    state.select(None);
                                } else if state.selected().is_none() {
                                    state.select(Some(0));
                                }
                                status = "Refreshed device list.".into();
                            }
                            Err(err) => status = format!("Refresh failed: {err}"),
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        let i = state.selected().unwrap_or(0);
                        let next = if devices.is_empty() {
                            None
                        } else {
                            Some((i + 1).min(devices.len() - 1))
                        };
                        state.select(next);
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        let i = state.selected().unwrap_or(0);
                        state.select(Some(i.saturating_sub(1)));
                    }
                    KeyCode::Enter => {
                        if let Some(index) = state.selected() {
                            if let Some(device) = devices.get(index) {
                                status = format!(
                                    "Open shell: russhtviewer connect {}   or browser {}#token=...",
                                    device.device_id, device.shell_url
                                );
                            }
                        }
                    }
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(());
                    }
                    _ => {}
                }
            }
        }
    }
}
