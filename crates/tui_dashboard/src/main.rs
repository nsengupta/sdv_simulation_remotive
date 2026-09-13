//! Observation-only Dashboard: connects to Gateway over UDS or Zenoh and renders twin emissions.
//!
//! Lifecycle (PowerOn/PowerOff) remains emulator-driven over CAN. Only **`q`** / Esc quit the TUI.

mod cli;
mod view;

use std::time::Duration;

use anyhow::{Context, Result, bail};
use common::observation_records::diagnostic::elapsed_since_session;
use crossterm::event::{self, Event, KeyCode};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use observation::{
    AnyLiveSource, LiveMessage, LiveRecordDto, LiveStream, UdsLiveSource, ZenohLiveSource,
    diagnostic_from_envelope, ledger_from_envelope,
};
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::{Frame, Terminal};
use tokio::sync::mpsc;

use common::facade::{PublishedFsmState, PublishedTransitionRecord, UnixTimestamp};
use common::{DiagnosticKind, DiagnosticRecord};
use view::{PaneLine, SegmentContent, SegmentStyle};

const VIRTUAL_CAR_IDENTITY: &str = "My-Opel-Corsa-1.4-GSi";
const BOOT_DIAGNOSTIC_WAIT: Duration = Duration::from_secs(5);

type DashboardTerminal = Terminal<ratatui::backend::CrosstermBackend<std::io::Stderr>>;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
enum ConnectionStatus {
    Connected { detail: String },
    #[default]
    Disconnected,
}

struct DashboardState {
    latest_diagnostic: Option<DiagnosticRecord>,
    latest_transition: Option<PublishedTransitionRecord>,
    ledger_tail: view::LedgerTail,
    connection: ConnectionStatus,
}

impl Default for DashboardState {
    fn default() -> Self {
        Self {
            latest_diagnostic: None,
            latest_transition: None,
            ledger_tail: view::LedgerTail::default(),
            connection: ConnectionStatus::Disconnected,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = cli::parse_args(std::env::args_os().skip(1))?;
    let detail = match &args.live {
        cli::DashboardLiveMode::Uds(path) => path.display().to_string(),
        cli::DashboardLiveMode::Zenoh { keyexpr } => format!("zenoh:{keyexpr}"),
    };
    let mut source = open_live_source(&args.live)
        .await
        .with_context(|| format!("connect live source ({detail})"))?;

    let hello = tokio::time::timeout(BOOT_DIAGNOSTIC_WAIT, source.recv())
        .await
        .context("timed out waiting for hello from Gateway")?
        .context("live link closed before hello")?
        .context("empty hello")?;
    match hello {
        LiveMessage::Hello { .. } => {}
        other => bail!("expected hello from Gateway, got {other:?}"),
    }

    let mut state = DashboardState {
        connection: ConnectionStatus::Connected {
            detail: detail.clone(),
        },
        ..DashboardState::default()
    };

    let boot = require_boot_from_source(&mut source, BOOT_DIAGNOSTIC_WAIT).await?;
    apply_diagnostic(boot, &mut state);

    let (live_tx, live_rx) = mpsc::unbounded_channel::<LiveMessage>();
    let (status_tx, status_rx) = mpsc::unbounded_channel::<ConnectionStatus>();
    tokio::spawn(async move {
        forward_live_source(source, live_tx, status_tx).await;
    });

    run_dashboard(live_rx, status_rx, state).await
}

async fn open_live_source(live: &cli::DashboardLiveMode) -> Result<AnyLiveSource> {
    match live {
        cli::DashboardLiveMode::Uds(path) => Ok(AnyLiveSource::Uds(
            UdsLiveSource::connect(path)
                .await
                .with_context(|| format!("UDS connect {}", path.display()))?,
        )),
        cli::DashboardLiveMode::Zenoh { keyexpr } => Ok(AnyLiveSource::Zenoh(
            ZenohLiveSource::subscribe(keyexpr.clone())
                .await
                .with_context(|| format!("Zenoh subscribe {keyexpr}"))?,
        )),
    }
}

async fn forward_live_source(
    mut source: AnyLiveSource,
    live_tx: mpsc::UnboundedSender<LiveMessage>,
    status_tx: mpsc::UnboundedSender<ConnectionStatus>,
) {
    loop {
        match source.recv().await {
            Ok(Some(message)) => {
                if live_tx.send(message).is_err() {
                    break;
                }
            }
            Ok(None) => {
                let _ = status_tx.send(ConnectionStatus::Disconnected);
                break;
            }
            Err(_) => {
                let _ = status_tx.send(ConnectionStatus::Disconnected);
                break;
            }
        }
    }
}

async fn require_boot_from_source(
    source: &mut AnyLiveSource,
    wait: Duration,
) -> Result<DiagnosticRecord> {
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            bail!("timed out waiting for Twin boot diagnostic over live link");
        }
        match tokio::time::timeout(remaining, source.recv()).await {
            Ok(Ok(Some(LiveMessage::Event {
                stream: LiveStream::Diagnostic,
                record: LiveRecordDto::Diagnostic(env),
            }))) => {
                let record = diagnostic_from_envelope(&env)?;
                if matches!(record.kind, DiagnosticKind::Boot) {
                    return Ok(record);
                }
 // Non-boot diagnostics before boot are unexpected; keep waiting.
            }
            Ok(Ok(Some(_))) => continue,
            Ok(Ok(None)) => bail!("live link closed before boot diagnostic"),
            Ok(Err(err)) => return Err(err.into()),
            Err(_) => bail!("timed out waiting for Twin boot diagnostic over live link"),
        }
    }
}

async fn run_dashboard(
    mut live_rx: mpsc::UnboundedReceiver<LiveMessage>,
    mut status_rx: mpsc::UnboundedReceiver<ConnectionStatus>,
    mut state: DashboardState,
) -> Result<()> {
    let mut terminal = setup_terminal()?;
    let loop_result = run_ui_loop(&mut terminal, &mut live_rx, &mut status_rx, &mut state).await;
    let restoration_result = restore_terminal(&mut terminal);
    preserve_primary_result(loop_result, restoration_result)
}

fn setup_terminal() -> Result<DashboardTerminal> {
    enable_raw_mode()?;
    let mut stderr = std::io::stderr();

    if let Err(error) = crossterm::execute!(stderr, EnterAlternateScreen) {
        best_effort_restore_after_setup_failure();
        return Err(error.into());
    }

    match Terminal::new(ratatui::backend::CrosstermBackend::new(stderr)) {
        Ok(terminal) => Ok(terminal),
        Err(error) => {
            best_effort_restore_after_setup_failure();
            Err(error.into())
        }
    }
}

fn best_effort_restore_after_setup_failure() {
    let mut stderr = std::io::stderr();
    let _ = stderr.execute(LeaveAlternateScreen);
    let _ = disable_raw_mode();
}

fn restore_terminal(terminal: &mut DashboardTerminal) -> Result<()> {
    let raw_mode_result = disable_raw_mode().map_err(anyhow::Error::from);
    let alternate_screen_result = terminal
        .backend_mut()
        .execute(LeaveAlternateScreen)
        .map(|_| ())
        .map_err(anyhow::Error::from);
    let cursor_result = terminal.show_cursor().map_err(anyhow::Error::from);

    let result = preserve_primary_result(raw_mode_result, alternate_screen_result);
    preserve_primary_result(result, cursor_result)
}

fn preserve_primary_result(primary: Result<()>, secondary: Result<()>) -> Result<()> {
    match primary {
        Err(error) => Err(error),
        Ok(()) => secondary,
    }
}

fn apply_diagnostic(record: DiagnosticRecord, state: &mut DashboardState) {
    if view::should_update_notice(&record.kind) {
        state.latest_diagnostic = Some(record);
    }
}

fn apply_ledger(record: PublishedTransitionRecord, state: &mut DashboardState) {
    state.ledger_tail.push(record.clone());
    state.latest_transition = Some(record);
}

fn apply_live_message(message: LiveMessage, state: &mut DashboardState) -> Result<()> {
    match message {
        LiveMessage::Hello { .. } => Ok(()),
        LiveMessage::Event {
            stream: LiveStream::Diagnostic,
            record: LiveRecordDto::Diagnostic(env),
        } => {
            apply_diagnostic(diagnostic_from_envelope(&env)?, state);
            Ok(())
        }
        LiveMessage::Event {
            stream: LiveStream::Ledger,
            record: LiveRecordDto::Ledger(env),
        } => {
            apply_ledger(ledger_from_envelope(&env)?, state);
            Ok(())
        }
        LiveMessage::Event { stream, .. } => {
            eprintln!("[dashboard] skipping mismatched live event for {stream:?}");
            Ok(())
        }
    }
}

fn drain_live_messages(
    live_rx: &mut mpsc::UnboundedReceiver<LiveMessage>,
    status_rx: &mut mpsc::UnboundedReceiver<ConnectionStatus>,
    state: &mut DashboardState,
) -> Result<()> {
    while let Ok(status) = status_rx.try_recv() {
        state.connection = status;
    }
    while let Ok(message) = live_rx.try_recv() {
        apply_live_message(message, state)?;
    }
    Ok(())
}

async fn run_ui_loop(
    terminal: &mut DashboardTerminal,
    live_rx: &mut mpsc::UnboundedReceiver<LiveMessage>,
    status_rx: &mut mpsc::UnboundedReceiver<ConnectionStatus>,
    state: &mut DashboardState,
) -> Result<()> {
    loop {
        drain_live_messages(live_rx, status_rx, state)?;

        terminal.draw(|f| {
            render_frame(f, state);
        })?;

        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

fn format_keys_footer(connection: &ConnectionStatus) -> String {
    match connection {
        ConnectionStatus::Connected { detail } => {
            format!("Connected to twin via {detail} | Keys: 'q' quit")
        }
        ConnectionStatus::Disconnected => "Disconnected from twin | Keys: 'q' quit".to_string(),
    }
}

fn render_frame(f: &mut Frame, state: &DashboardState) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(3),
        ])
        .split(f.size());

    let status = format_status_line(&state.latest_diagnostic, &state.latest_transition);
    let session_block = Block::default()
        .title(" Session ")
        .borders(Borders::ALL)
        .style(Style::default().fg(Color::Yellow));
    f.render_widget(
        Paragraph::new(Line::from(Span::raw(status))).block(session_block),
        outer[0],
    );

    let middle = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(outer[1]);

    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(middle[0]);

    let driver_width = top[0].width.saturating_sub(2) as usize;
    let engineer_width = top[1].width.saturating_sub(2) as usize;
    let ledger_width = middle[1].width.saturating_sub(2) as usize;
    let ledger_rows = middle[1].height.saturating_sub(2) as usize;

    let driver = view::driver_pane(
        state.latest_diagnostic.as_ref(),
        state.latest_transition.as_ref(),
        driver_width,
    );
    let engineer = view::engineer_pane(state.latest_transition.as_ref(), engineer_width);
    let ledger_lines = state.ledger_tail.visible_lines(ledger_width, ledger_rows);

    f.render_widget(Clear, top[0]);
    f.render_widget(Clear, top[1]);
    f.render_widget(Clear, middle[1]);

    let pane_title_style = Style::default()
        .fg(Color::White)
        .add_modifier(Modifier::BOLD);
    let driver_block = Block::default()
        .title(" Diagnostic/Telemetry ")
        .title_style(pane_title_style)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    f.render_widget(
        Paragraph::new(
            driver
                .lines
                .into_iter()
                .map(pane_line_to_ratatui)
                .collect::<Vec<_>>(),
        )
        .block(driver_block),
        top[0],
    );

    let engineer_block = Block::default()
        .title(" State Transitions ")
        .title_style(pane_title_style)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green));
    f.render_widget(
        Paragraph::new(
            engineer
                .lines
                .into_iter()
                .map(pane_line_to_ratatui)
                .collect::<Vec<_>>(),
        )
        .block(engineer_block),
        top[1],
    );

    let ledger_block = Block::default()
        .title(" Deterministic Transition Ledger (live) ")
        .borders(Borders::ALL)
        .style(Style::default().fg(Color::Magenta));
    f.render_widget(
        Paragraph::new(
            ledger_lines
                .into_iter()
                .map(pane_line_to_ratatui)
                .collect::<Vec<_>>(),
        )
        .block(ledger_block),
        middle[1],
    );

    let keys_block = Block::default()
        .borders(Borders::ALL)
        .style(Style::default().fg(Color::DarkGray));
    f.render_widget(
        Paragraph::new(Line::from(Span::raw(format_keys_footer(&state.connection))))
            .block(keys_block),
        outer[2],
    );
}

fn pane_line_to_ratatui(line: PaneLine) -> Line<'static> {
    let mut spans = Vec::new();
    for seg in line.segments {
        match seg.content {
            SegmentContent::Text(text) => {
                spans.push(Span::styled(text, segment_style(seg.style)));
            }
            SegmentContent::SpeedBar { cells } => {
                for cell in cells {
                    let ch = if cell.filled { "|" } else { "." };
                    let style = segment_style(SegmentStyle::from_speed_band(cell.band));
                    spans.push(Span::styled(ch, style));
                }
            }
            SegmentContent::Swatch => {}
            SegmentContent::Icon(icon) => {
                spans.push(Span::styled(icon.as_str().to_owned(), segment_style(seg.style)));
            }
        }
    }
    Line::from(spans)
}

fn segment_style(token: SegmentStyle) -> Style {
    match token {
        SegmentStyle::Default => Style::default(),
        SegmentStyle::Mute => Style::default().fg(Color::DarkGray),
        SegmentStyle::Label => Style::default().fg(Color::Cyan),
        SegmentStyle::ZoneGreen => Style::default().fg(Color::Green),
        SegmentStyle::ZoneYellow => Style::default().fg(Color::Yellow),
        SegmentStyle::ZoneRed => Style::default().fg(Color::Red),
    }
}

fn format_published_state(state: &PublishedFsmState) -> String {
    match state {
        PublishedFsmState::ExtremeOperationWarning { .. } => "ExtremeOpWarn".to_owned(),
        other => format!("{other:?}"),
    }
}

fn format_status_line(
    latest_diagnostic: &Option<DiagnosticRecord>,
    latest_transition: &Option<PublishedTransitionRecord>,
) -> String {
    let session_started_at = latest_transition
        .as_ref()
        .map(|t| t.session_started_at)
        .or_else(|| latest_diagnostic.as_ref().map(|d| d.session_started_at));

    let session_label = session_started_at
        .map(format_unix_timestamp_short)
        .unwrap_or_else(|| "awaiting twin…".to_string());

    let twin_elapsed = latest_twin_elapsed(latest_diagnostic, latest_transition)
        .map(format_elapsed)
        .unwrap_or_else(|| "—".to_string());

    let last_ledger = latest_transition
        .as_ref()
        .map(|t| format!("seq {}", t.record_seq))
        .unwrap_or_else(|| "—".to_string());

    let fsm_label = twin_fsm_status_label(latest_transition);

    format!(
        "Car: {VIRTUAL_CAR_IDENTITY}  │  Twin T+: {twin_elapsed}  │  Session start: {session_label}  │  FSM: {fsm_label}  │  Last ledger: {last_ledger}"
    )
}

fn twin_fsm_status_label(latest_transition: &Option<PublishedTransitionRecord>) -> String {
    match latest_transition {
        None => "standby (no ledger yet)".to_string(),
        Some(row) => format_published_state(&row.next_state),
    }
}

fn latest_twin_elapsed(
    latest_diagnostic: &Option<DiagnosticRecord>,
    latest_transition: &Option<PublishedTransitionRecord>,
) -> Option<Duration> {
    match (latest_diagnostic, latest_transition) {
        (Some(d), Some(t)) => Some(
            d.elapsed_since_session()
                .max(elapsed_since_session(t.recorded_at, t.session_started_at)),
        ),
        (Some(d), None) => Some(d.elapsed_since_session()),
        (None, Some(t)) => Some(elapsed_since_session(t.recorded_at, t.session_started_at)),
        (None, None) => None,
    }
}

fn format_elapsed(d: Duration) -> String {
    let secs = d.as_secs();
    let hours = secs / 3600;
    let mins = (secs % 3600) / 60;
    let secs = secs % 60;
    if hours > 0 {
        format!("{hours}h {mins:02}m {secs:02}s")
    } else if mins > 0 {
        format!("{mins}m {secs:02}s")
    } else {
        format!("{secs}s")
    }
}

fn format_unix_timestamp_short(timestamp: UnixTimestamp) -> String {
    let secs = timestamp.unix_seconds();
    format!(
        "{:02}:{:02}:{:02} UTC",
        (secs / 3600) % 24,
        (secs % 3600) / 60,
        secs % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::DiagnosticLevel;
    use common::facade::{PublishedDomainAction, PublishedFsmEvent};
    use observation::schema::CURRENT_SCHEMA_VERSION;
    use observation::schema::v1::{
        DiagnosticKindV1, DiagnosticLevelV1, DiagnosticPayloadV1, RunId, StreamEnvelopeV1,
        UnixTimestampV1,
    };
    use observation::{LiveMessage, LiveSink, MemoryLiveLink, MemoryLiveSource};

    const MAX_PANEL_LINE_CHARS: usize = 72;

    fn twin_pre_power_on(latest_transition: &Option<PublishedTransitionRecord>) -> bool {
        latest_transition.is_none()
    }

    fn truncate_line(s: &str) -> String {
        view::clip_line(s, MAX_PANEL_LINE_CHARS)
    }

    fn format_actions_summary(actions: &[PublishedDomainAction]) -> String {
        if actions.is_empty() {
            return "—".to_string();
        }
        actions
            .iter()
            .map(|action| match action {
                PublishedDomainAction::LogWarning(msg) => {
                    format!("LogWarning({})", truncate_line(msg))
                }
                other => format!("{other:?}"),
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn sample_boot_diagnostic() -> DiagnosticRecord {
        DiagnosticRecord {
            level: DiagnosticLevel::Info,
            source: "VirtualCarActor",
            kind: DiagnosticKind::Boot,
            session_started_at: UnixTimestamp::from_duration_since_epoch(Duration::new(
                1_700_000_000,
                0,
            )),
            recorded_at: UnixTimestamp::from_duration_since_epoch(Duration::new(
                1_700_000_000,
                50_000_000,
            )),
        }
    }

    #[test]
    fn status_line_uses_boot_diagnostic_without_ledger() {
        let diag = sample_boot_diagnostic();
        let line = format_status_line(&Some(diag.clone()), &None);
        assert!(line.contains("standby (no ledger yet)"));
        assert!(line.contains("Twin T+:"));
        assert!(!line.contains("awaiting twin"));
        assert!(line.contains("Last ledger: —"));
        assert_eq!(
            latest_twin_elapsed(&Some(diag), &None),
            Some(Duration::from_millis(50))
        );
    }

    #[test]
    fn status_line_fsm_from_latest_ledger_row() {
        let row = PublishedTransitionRecord {
            car_identity: "x".into(),
            session_started_at: UnixTimestamp::from_duration_since_epoch(Duration::from_nanos(1)),
            record_seq: 2,
            recorded_at: UnixTimestamp::from_duration_since_epoch(Duration::ZERO),
            event: PublishedFsmEvent::UpdateRpm(1500),
            old_state: PublishedFsmState::Idle,
            next_state: PublishedFsmState::Driving,
            old_ctx: empty_published_ctx(),
            current_ctx: empty_published_ctx(),
            actions: vec![],
        };
        let line = format_status_line(&None, &Some(row));
        assert!(line.contains("FSM: Driving"));
    }

    #[test]
    fn pre_power_panels_until_first_ledger_row() {
        assert!(twin_pre_power_on(&None));
        assert!(!twin_pre_power_on(&Some(sample_ledger_row())));
    }

    #[test]
    fn standby_panels_show_can_lifecycle_message() {
        let driver = view::driver_pane(None, None, 40);
        let engineer = view::engineer_pane(None, 40);
        assert_eq!(driver.lines.len(), 3);
        assert_eq!(engineer.lines.len(), 3);
        assert!(driver.lines[1].text().contains("PowerOn"));
        assert!(driver.lines[1].text().contains("CAN"));
        assert!(engineer.lines[0].text().starts_with("Twin installed."));
    }

    #[test]
    fn keys_footer_shows_connected_and_disconnected() {
        let uds = format_keys_footer(&ConnectionStatus::Connected {
            detail: "./tmp/observation.sock".into(),
        });
        assert!(uds.contains("Connected to twin via ./tmp/observation.sock"));
        assert!(uds.contains("Keys: 'q' quit"));

        let zenoh = format_keys_footer(&ConnectionStatus::Connected {
            detail: "zenoh:sdv/twin/observation".into(),
        });
        assert!(zenoh.contains("zenoh:sdv/twin/observation"));

        let disconnected = format_keys_footer(&ConnectionStatus::Disconnected);
        assert!(disconnected.starts_with("Disconnected from twin"));
        assert!(disconnected.contains("Keys: 'q' quit"));
    }

    #[test]
    fn status_line_prefixes_car_identity() {
        let line = format_status_line(&None, &None);
        assert!(line.starts_with("Car: My-Opel-Corsa"));
    }

    #[test]
    fn truncate_line_shortens_long_rejection_messages() {
        let long = "[REJECTED]: vehicle must be Idle before PowerOff; current state is DrivingDangerously with extra detail";
        let truncated = truncate_line(long);
        assert!(truncated.chars().count() <= MAX_PANEL_LINE_CHARS);
        assert!(truncated.ends_with('…'));
    }

    #[test]
    fn format_actions_summary_truncates_log_warning() {
        let summary = format_actions_summary(&[PublishedDomainAction::LogWarning(
            "[REJECTED]: vehicle must be Idle before PowerOff; current state is Driving".into(),
        )]);
        assert!(summary.starts_with("LogWarning("));
        assert!(summary.len() < 120);
    }

    #[test]
    fn apply_diagnostic_updates_notice_state() {
        let mut state = DashboardState::default();
        let record = sample_boot_diagnostic();
        apply_diagnostic(record.clone(), &mut state);
        assert_eq!(state.latest_diagnostic.as_ref().unwrap().kind, record.kind);
    }

    #[test]
    fn apply_diagnostic_skips_timer_tick_for_notice() {
        let previous = sample_boot_diagnostic();
        let mut state = DashboardState {
            latest_diagnostic: Some(previous.clone()),
            ..DashboardState::default()
        };
        let tick = DiagnosticRecord {
            level: DiagnosticLevel::Info,
            source: "VirtualCarActor",
            kind: DiagnosticKind::TimerTick,
            session_started_at: previous.session_started_at,
            recorded_at: previous.recorded_at,
        };
        apply_diagnostic(tick, &mut state);
        assert_eq!(state.latest_diagnostic.as_ref().unwrap().kind, previous.kind);
    }

    #[test]
    fn apply_ledger_updates_state_and_tail() {
        let mut state = DashboardState::default();
        let record = sample_ledger_row();
        apply_ledger(record.clone(), &mut state);
        assert_eq!(state.latest_transition, Some(record));
        assert_eq!(state.ledger_tail.lines(80).len(), 1);
    }

    #[test]
    fn mock_live_source_updates_dashboard_state() {
        let (mut sink, mut source) = MemoryLiveLink::pair();
        let mut state = DashboardState {
            connection: ConnectionStatus::Connected {
                detail: "./tmp/observation.sock".into(),
            },
            ..DashboardState::default()
        };

        sink.emit(&LiveMessage::hello(VIRTUAL_CAR_IDENTITY)).unwrap();
        let boot_env = StreamEnvelopeV1 {
            schema_version: CURRENT_SCHEMA_VERSION,
            run_id: RunId::parse("00000000-0000-4000-8000-000000000001").unwrap(),
            vehicle_identity: VIRTUAL_CAR_IDENTITY.into(),
            recorded_at: UnixTimestampV1::new(1_700_000_000, 50_000_000).unwrap(),
            payload: DiagnosticPayloadV1 {
                level: DiagnosticLevelV1::Info,
                source: "VirtualCarActor".into(),
                kind: DiagnosticKindV1::Boot,
                session_started_at: UnixTimestampV1::new(1_700_000_000, 0).unwrap(),
            },
        };
        sink.emit(&LiveMessage::diagnostic_event(boot_env)).unwrap();
        sink.finish().unwrap();

 // Consume hello (connection already set), then boot event.
        let _hello = observation::LiveSource::recv_blocking(&mut source)
            .unwrap()
            .unwrap();
        let boot_msg = observation::LiveSource::recv_blocking(&mut source)
            .unwrap()
            .unwrap();
        apply_live_message(boot_msg, &mut state).unwrap();

        assert!(matches!(
            state.connection,
            ConnectionStatus::Connected { .. }
        ));
        assert!(matches!(
            state.latest_diagnostic.as_ref().unwrap().kind,
            DiagnosticKind::Boot
        ));
        let _unused: MemoryLiveSource = source;
    }

    fn sample_ledger_row() -> PublishedTransitionRecord {
        PublishedTransitionRecord {
            car_identity: "x".into(),
            session_started_at: UnixTimestamp::from_duration_since_epoch(Duration::new(
                1_700_000_000,
                0,
            )),
            record_seq: 1,
            recorded_at: UnixTimestamp::from_duration_since_epoch(Duration::new(
                1_700_000_000,
                100_000_000,
            )),
            event: PublishedFsmEvent::PowerOn,
            old_state: PublishedFsmState::Off,
            next_state: PublishedFsmState::PreparingToStart,
            old_ctx: empty_published_ctx(),
            current_ctx: empty_published_ctx(),
            actions: vec![],
        }
    }

    #[test]
    fn dashboard_live_state_renders_sccm_bcm_focused_panes() {
        let mut state = DashboardState::default();
        let mut row = sample_ledger_row();
        row.event = PublishedFsmEvent::HazardButtonObserved(true);
        row.current_ctx.sccm.hazard_button_on = common::facade::PublishedObservedBool::On;
        row.current_ctx.bcm.state = common::facade::PublishedBcmState::Ready;
        row.current_ctx.bcm.left_turn_request_on = common::facade::PublishedObservedBool::On;
        row.current_ctx.bcm.right_turn_request_on = common::facade::PublishedObservedBool::Off;
        apply_ledger(row, &mut state);

        let driver = view::driver_pane(None, state.latest_transition.as_ref(), 72);
        let engineer = view::engineer_pane(state.latest_transition.as_ref(), 72);
        let driver_text = driver
            .lines
            .iter()
            .map(view::PaneLine::text)
            .collect::<Vec<_>>()
            .join("\n");
        let engineer_text = engineer
            .lines
            .iter()
            .map(view::PaneLine::text)
            .collect::<Vec<_>>()
            .join("\n");
        let ledger_text = state
            .ledger_tail
            .lines(120)
            .iter()
            .map(view::PaneLine::text)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(driver_text.contains("Hazard: ON"), "{driver_text}");
        assert!(driver_text.contains("Left request: ON"), "{driver_text}");
        assert!(driver_text.contains("Right request: OFF"), "{driver_text}");
        assert!(!driver_text.contains("Visibility:"), "{driver_text}");
        assert!(!driver_text.contains("Wipers:"), "{driver_text}");
        assert!(
            engineer_text.contains("Last event: HazardButtonObserved(true)"),
            "{engineer_text}"
        );
        assert!(engineer_text.contains("SCCM: Hazard ON"), "{engineer_text}");
        assert!(
            engineer_text.contains("BCM: Ready  Left ON  Right OFF"),
            "{engineer_text}"
        );
        assert!(ledger_text.contains("HazardButtonObserved(true)"), "{ledger_text}");
        assert!(ledger_text.contains("SCCM Hazard=ON"), "{ledger_text}");
        assert!(ledger_text.contains("BCM Ready"), "{ledger_text}");
    }

    fn empty_published_ctx() -> common::facade::PublishedVehicleContext {
        use common::facade::{
            PublishedBcmContext, PublishedBcmState, PublishedHeadlampContext,
            PublishedHeadlampState, PublishedHealthContext, PublishedObservedBool,
            PublishedPowertrainContext, PublishedSccmContext, PublishedVehicleContext,
            PublishedVisibilityContext,
            PublishedWeatherContext, PublishedWheelRpm, PublishedWiperContext, PublishedWiperState,
        };
        PublishedVehicleContext {
            sccm: PublishedSccmContext {
                hazard_button_on: PublishedObservedBool::Unknown,
            },
            bcm: PublishedBcmContext {
                state: PublishedBcmState::Off,
                left_turn_request_on: PublishedObservedBool::Unknown,
                right_turn_request_on: PublishedObservedBool::Unknown,
            },
            powertrain: PublishedPowertrainContext {
                wheel_rpm: PublishedWheelRpm {
                    front_left: 0,
                    front_right: 0,
                    rear_left: 0,
                    rear_right: 0,
                },
                speed_kph: 0,
            },
            health: PublishedHealthContext {
                fuel_level_pct: 100,
                oil_pressure_kpa: 100,
                tyre_pressure_ok: true,
            },
            visibility: PublishedVisibilityContext { ambient_lux: 0 },
            weather: PublishedWeatherContext { raining: false },
            headlamp: PublishedHeadlampContext {
                state: PublishedHeadlampState::Off,
                ack_pending_since: None,
            },
            wiper: PublishedWiperContext {
                state: PublishedWiperState::Off,
            },
        }
    }
}
