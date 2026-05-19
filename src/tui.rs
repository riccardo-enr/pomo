/*
 * Terminal countdown UI. Renders a big ASCII clock, a progress gauge, the
 * label, and a hint line. Space pauses, q/Esc quits early, r restarts.
 */

use crate::state::StateFile;
use crate::Kind;
use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph},
    Terminal,
};
use std::{
    io::{self, Stdout},
    time::{Duration, Instant},
};

#[derive(PartialEq, Eq)]
pub enum Outcome {
    Finished,
    Aborted,
}

pub fn run(label: &str, total: Duration, kind: Kind, state: &mut StateFile) -> Result<Outcome> {
    let mut term = setup()?;
    let res = run_loop(&mut term, label, total, kind, state);
    teardown(&mut term)?;
    res
}

fn setup() -> Result<Terminal<ratatui::backend::CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut out = io::stdout();
    execute!(out, EnterAlternateScreen)?;
    Ok(Terminal::new(ratatui::backend::CrosstermBackend::new(out))?)
}

fn teardown(term: &mut Terminal<ratatui::backend::CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(term.backend_mut(), LeaveAlternateScreen)?;
    term.show_cursor()?;
    Ok(())
}

fn run_loop(
    term: &mut Terminal<ratatui::backend::CrosstermBackend<Stdout>>,
    label: &str,
    total: Duration,
    kind: Kind,
    state: &mut StateFile,
) -> Result<Outcome> {
    let mut start = Instant::now();
    let mut paused_at: Option<Instant> = None;
    let mut accumulated_pause = Duration::ZERO;

    let color = match kind {
        Kind::Work => Color::Red,
        Kind::Break => Color::Green,
        Kind::Timer => Color::Cyan,
    };

    loop {
        let elapsed = if let Some(p) = paused_at {
            p.duration_since(start) - accumulated_pause
        } else {
            start.elapsed() - accumulated_pause
        };
        let remaining = total.saturating_sub(elapsed);
        let ratio = (elapsed.as_secs_f64() / total.as_secs_f64()).clamp(0.0, 1.0);

        state.write(label, kind, remaining, total, paused_at.is_some());

        term.draw(|f| {
            let area = f.area();
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(1)
                .constraints([
                    Constraint::Length(2),
                    Constraint::Min(7),
                    Constraint::Length(3),
                    Constraint::Length(2),
                ])
                .split(area);

            let header = Paragraph::new(Line::from(vec![
                Span::styled(label.to_string(), Style::default().add_modifier(Modifier::BOLD).fg(color)),
                Span::raw("  "),
                Span::styled(
                    if paused_at.is_some() { "[PAUSED]" } else { "" },
                    Style::default().fg(Color::Yellow),
                ),
            ]))
            .alignment(Alignment::Center);
            f.render_widget(header, chunks[0]);

            let big = big_clock(remaining);
            let clock = Paragraph::new(big)
                .alignment(Alignment::Center)
                .style(Style::default().fg(color).add_modifier(Modifier::BOLD))
                .block(Block::default().borders(Borders::NONE));
            f.render_widget(clock, chunks[1]);

            let gauge = Gauge::default()
                .block(Block::default().borders(Borders::ALL))
                .gauge_style(Style::default().fg(color))
                .ratio(ratio)
                .label(format!("{:.0}%", ratio * 100.0));
            f.render_widget(gauge, chunks[2]);

            let hint = Paragraph::new(Line::from(vec![Span::styled(
                "[space] pause/resume   [r] restart   [q/esc] quit",
                Style::default().fg(Color::DarkGray),
            )]))
            .alignment(Alignment::Center);
            f.render_widget(hint, chunks[3]);
        })?;

        if remaining.is_zero() {
            return Ok(Outcome::Finished);
        }

        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(k) = event::read()? {
                if k.kind != KeyEventKind::Press {
                    continue;
                }
                match k.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(Outcome::Aborted),
                    KeyCode::Char(' ') => {
                        if let Some(p) = paused_at.take() {
                            accumulated_pause += p.elapsed();
                        } else {
                            paused_at = Some(Instant::now());
                        }
                    }
                    KeyCode::Char('r') => {
                        start = Instant::now();
                        paused_at = None;
                        accumulated_pause = Duration::ZERO;
                    }
                    _ => {}
                }
            }
        }
    }
}

fn big_clock(d: Duration) -> String {
    let total = d.as_secs();
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    let text = if h > 0 {
        format!("{:02}:{:02}:{:02}", h, m, s)
    } else {
        format!("{:02}:{:02}", m, s)
    };
    render_big(&text)
}

/*
 * Renders digits/colons as 5-row ASCII glyphs. Returns a single string with
 * embedded newlines, suitable for a centered Paragraph.
 */
fn render_big(text: &str) -> String {
    let rows: Vec<String> = (0..5)
        .map(|row| {
            text.chars()
                .map(|c| glyph(c)[row])
                .collect::<Vec<_>>()
                .join("  ")
        })
        .collect();
    rows.join("\n")
}

fn glyph(c: char) -> [&'static str; 5] {
    match c {
        '0' => [" ███ ", "█   █", "█   █", "█   █", " ███ "],
        '1' => ["  █  ", " ██  ", "  █  ", "  █  ", " ███ "],
        '2' => [" ███ ", "█   █", "   █ ", "  █  ", "█████"],
        '3' => ["████ ", "    █", " ███ ", "    █", "████ "],
        '4' => ["█   █", "█   █", "█████", "    █", "    █"],
        '5' => ["█████", "█    ", "████ ", "    █", "████ "],
        '6' => [" ███ ", "█    ", "████ ", "█   █", " ███ "],
        '7' => ["█████", "    █", "   █ ", "  █  ", " █   "],
        '8' => [" ███ ", "█   █", " ███ ", "█   █", " ███ "],
        '9' => [" ███ ", "█   █", " ████", "    █", " ███ "],
        ':' => ["     ", "  █  ", "     ", "  █  ", "     "],
        _ => ["     ", "     ", "     ", "     ", "     "],
    }
}
