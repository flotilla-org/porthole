//! Local operator UI. The daemon owns policy, scope and request state.
use std::{
    io::{self, IsTerminal},
    time::Duration,
};

use crossterm::{
    event::{Event, EventStream, KeyCode, KeyEventKind, KeyModifiers},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use futures_util::StreamExt;
use http_body_util::BodyExt;
use porthole_protocol::agent_permissions::{
    AgentGrantResponse, AgentPermissionDuration, AgentPermissionRequestResponse, AgentPermissionTarget, ApproveAgentPermissionRequest,
    DenyAgentPermissionRequest, RevocationResponse,
};
use ratatui::{
    Terminal, TerminalOptions, Viewport,
    backend::CrosstermBackend,
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};
use tokio::sync::{mpsc, watch};

use super::permission_display as display;
use crate::client::{ClientError, DaemonClient};

#[derive(Clone, Default)]
struct Snapshot {
    requests: Vec<AgentPermissionRequestResponse>,
    grants: Vec<AgentGrantResponse>,
}
#[derive(Clone, Default)]
struct Update {
    snapshot: Option<Snapshot>,
    connected: bool,
    message: String,
    /// Incremented only after a mutation completes (including errors).
    completed: u64,
}
#[derive(Clone)]
enum Decision {
    Approve(AgentPermissionRequestResponse, AgentPermissionDuration),
    Deny(AgentPermissionRequestResponse),
    Revoke(String),
}
#[derive(Clone)]
enum Detail {
    Request(AgentPermissionRequestResponse, AgentPermissionDuration),
    Grant(AgentGrantResponse),
}
#[derive(Default)]
struct Scope {
    selected: Option<String>,
    filter: String,
    offset: usize,
}
#[derive(Default)]
struct Inbox {
    snapshot: Snapshot,
    scopes: [Scope; 2],
    grants: bool,
    filtering: bool,
    detail: Option<Detail>,
    detail_scroll: u16,
    connected: bool,
    busy: bool,
    completed: u64,
    message: String,
}

impl Inbox {
    fn index(&self) -> usize {
        usize::from(self.grants)
    }
    fn entries(&self) -> Vec<(String, String)> {
        let filter = self.scopes[self.index()].filter.to_lowercase();
        let rows: Vec<_> = if self.grants {
            self.snapshot
                .grants
                .iter()
                .map(|g| {
                    (
                        g.grant_id.to_string(),
                        format!(
                            "{} | {} | {}",
                            display::clean(g.description.agent_name.as_deref().unwrap_or(g.agent_id.as_str())),
                            display::actions(&g.actions),
                            compact_target(&g.target, &g.description)
                        ),
                    )
                })
                .collect()
        } else {
            self.snapshot
                .requests
                .iter()
                .map(|r| {
                    (
                        r.request_id.to_string(),
                        format!(
                            "{} | {} | {}",
                            display::clean(r.description.agent_name.as_deref().unwrap_or(r.agent_id.as_str())),
                            display::operation(&r.description, r.reason.as_deref()),
                            compact_target(&r.target, &r.description)
                        ),
                    )
                })
                .collect()
        };
        rows.into_iter().filter(|(_, text)| text.to_lowercase().contains(&filter)).collect()
    }
    fn reconcile(&mut self) {
        let rows = self.entries();
        let scope = &mut self.scopes[self.index()];
        if !rows.iter().any(|(id, _)| Some(id) == scope.selected.as_ref()) {
            scope.selected = rows.first().map(|(id, _)| id.clone());
            scope.offset = 0;
        }
    }
    fn update(&mut self, update: Update) {
        if let Some(snapshot) = update.snapshot {
            self.snapshot = snapshot;
        }
        if update.completed != self.completed {
            self.busy = false;
        }
        self.completed = update.completed;
        self.connected = update.connected;
        self.message = update.message;
        self.reconcile();
    }
    fn navigate(&mut self, down: bool) {
        if self.detail.is_some() {
            self.detail_scroll = if down {
                self.detail_scroll.saturating_add(1)
            } else {
                self.detail_scroll.saturating_sub(1)
            };
            return;
        }
        let rows = self.entries();
        let scope = &mut self.scopes[self.index()];
        let current = rows.iter().position(|(id, _)| Some(id) == scope.selected.as_ref()).unwrap_or(0);
        let next = if down {
            (current + 1).min(rows.len().saturating_sub(1))
        } else {
            current.saturating_sub(1)
        };
        scope.selected = rows.get(next).map(|(id, _)| id.clone());
    }
    fn open(&mut self) {
        let selected = self.scopes[self.index()].selected.as_deref();
        self.detail = if self.grants {
            self.snapshot
                .grants
                .iter()
                .find(|g| Some(g.grant_id.as_str()) == selected)
                .cloned()
                .map(Detail::Grant)
        } else {
            self.snapshot
                .requests
                .iter()
                .find(|r| Some(r.request_id.as_str()) == selected)
                .cloned()
                .map(|r| {
                    let duration = display::default_duration(&r.target);
                    Detail::Request(r, duration)
                })
        };
        self.detail_scroll = 0;
    }
    fn actionable(&self) -> bool {
        if !self.connected || self.busy {
            return false;
        }
        match &self.detail {
            Some(Detail::Request(r, _)) => self
                .snapshot
                .requests
                .iter()
                .any(|current| current == r && current.status == "pending"),
            Some(Detail::Grant(g)) => self.snapshot.grants.iter().any(|current| current == g),
            None => false,
        }
    }
    fn decision(&self, key: char) -> Option<Decision> {
        if !self.actionable() {
            return None;
        }
        match (&self.detail, key) {
            (Some(Detail::Request(r, duration)), 'a') if !r.description.agent_revoked && r.description.surface_available != Some(false) => {
                Some(Decision::Approve(r.clone(), duration.clone()))
            }
            (Some(Detail::Request(r, _)), 'd') => Some(Decision::Deny(r.clone())),
            (Some(Detail::Grant(g)), 'r') => Some(Decision::Revoke(g.grant_id.to_string())),
            _ => None,
        }
    }
}

const NETWORK_TIMEOUT: Duration = Duration::from_secs(3);

async fn snapshot(client: &DaemonClient) -> Result<Snapshot, ClientError> {
    let (requests, grants) = tokio::try_join!(
        client.get_json("/agent-permissions/requests"),
        client.get_json("/agent-permissions/grants")
    )?;
    Ok(Snapshot { requests, grants })
}
async fn decide(client: &DaemonClient, decision: Decision) -> Result<String, ClientError> {
    match decision {
        Decision::Approve(r, duration) => {
            let _: AgentGrantResponse = client
                .post_json(
                    &format!("/agent-permissions/requests/{}/approve", r.request_id),
                    &ApproveAgentPermissionRequest {
                        target: r.target,
                        actions: r.actions,
                        duration,
                        constraints: Default::default(),
                    },
                )
                .await?;
            Ok("Approved. Esc returns to the inbox.".into())
        }
        Decision::Deny(r) => {
            let _: AgentPermissionRequestResponse = client
                .post_json(
                    &format!("/agent-permissions/requests/{}/deny", r.request_id),
                    &DenyAgentPermissionRequest {
                        remember: false,
                        reason: None,
                    },
                )
                .await?;
            Ok("Denied this request. Esc returns to the inbox.".into())
        }
        Decision::Revoke(id) => {
            let result: RevocationResponse = client
                .post_json(&format!("/agent-permissions/grants/{id}/revoke"), &serde_json::json!({}))
                .await?;
            Ok(if result.revoked {
                "Revoked for future checks; existing work continues."
            } else {
                "Grant was already revoked."
            }
            .into())
        }
    }
}

async fn network(client: DaemonClient, tx: watch::Sender<Update>, mut commands: mpsc::Receiver<Decision>) {
    let mut state = Update::default();
    let mut events = None;
    let mut tick = tokio::time::interval(Duration::from_secs(2));
    loop {
        // Subscribe before the initial/reconnected snapshot, closing the lost-update gap.
        if events.is_none() {
            events = match tokio::time::timeout(NETWORK_TIMEOUT, client.events()).await {
                Ok(Ok(stream)) => Some(stream),
                _ => None,
            };
        }
        let refresh = tokio::time::timeout(NETWORK_TIMEOUT, snapshot(&client)).await;
        match refresh {
            Ok(Ok(snapshot)) if events.is_some() => {
                state.snapshot = Some(snapshot);
                if !state.connected {
                    state.message = "Live. Enter opens details; no actions are sent from the list.".into();
                }
                state.connected = true;
            }
            result => {
                state.connected = false;
                state.message = match result {
                    Ok(Err(error)) => format!("Disconnected: {error}. Retrying; actions disabled."),
                    _ => "Disconnected or timed out. Retrying; actions disabled.".into(),
                };
                events = None;
            }
        }
        if tx.send(state.clone()).is_err() {
            return;
        }
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else { return; };
                // The UI already disables disconnected actions, but state can change in transit.
                state.message = if !state.connected {
                    "Not sent: connection unavailable.".into()
                } else {
                    match tokio::time::timeout(NETWORK_TIMEOUT, decide(&client, command)).await {
                        Ok(Ok(message)) => message,
                        Ok(Err(error)) => format!("Action failed: {error}"),
                        Err(_) => "Action timed out; outcome unknown. Refreshing, without retrying it.".into(),
                    }
                };
                state.completed += 1;
            }
            frame = async { match events.as_mut() { Some(body) => body.frame().await, None => std::future::pending().await } } => {
                // Any SSE frame (including resync_required) invalidates the snapshot.
                if !matches!(frame, Some(Ok(_))) {
                    events = None;
                    state.connected = false;
                    state.message = "Event connection lost. Resynchronizing; actions disabled.".into();
                    if tx.send(state.clone()).is_err() { return; }
                }
            }
            _ = tick.tick() => {}
        }
    }
}

fn compact_target(target: &AgentPermissionTarget, description: &porthole_protocol::agent_permissions::PermissionDescription) -> String {
    match &description.surface {
        Some(s) => display::clean(&format!(
            "{}: {}{}",
            s.app_name.as_deref().unwrap_or("unknown app"),
            s.title.as_deref().unwrap_or("untitled"),
            if description.surface_available == Some(false) {
                " [closed/unavailable]"
            } else {
                ""
            }
        )),
        None => display::target(target, description),
    }
}

fn draw(frame: &mut ratatui::Frame, app: &mut Inbox) {
    let areas = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(2),
        Constraint::Length(2),
    ])
    .split(frame.area());
    let scopes = if app.grants {
        " Requests   [Grants] "
    } else {
        " [Requests]   Grants "
    };
    frame.render_widget(
        Paragraph::new(format!(
            "{scopes}  {}  | local daemon",
            if app.connected { "LIVE" } else { "OFFLINE" }
        ))
        .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        areas[0],
    );
    let context = match &app.detail {
        Some(Detail::Request(_, duration)) => format!("APPROVE FOR: {}", display::duration(duration)),
        _ => format!(
            "{}: {}",
            if app.filtering { "Filter (Enter to finish)" } else { "Filter" },
            app.scopes[app.index()].filter
        ),
    };
    frame.render_widget(Paragraph::new(context).style(Style::default().fg(Color::Yellow)), areas[1]);
    if let Some(detail) = &app.detail {
        let mut lines = match detail {
            Detail::Request(r, _) => {
                let mut lines = display::request_lines(r);
                lines.retain(|line| !line.starts_with("suggested duration:"));
                if let Some(status) = lines.iter_mut().find(|line| line.starts_with("status:")) {
                    if !app.connected {
                        *status = "status: last seen pending (offline)".into();
                    } else if !app.snapshot.requests.iter().any(|current| current.request_id == r.request_id) {
                        *status = "status: no longer pending".into();
                    }
                }
                lines.push("This grants the capability scope above, not just the triggering operation.".into());
                if r.description.agent_revoked || r.description.surface_available == Some(false) {
                    lines.push("Approval unavailable: requester revoked or window unavailable. You can still deny this request.".into());
                }
                lines
            }
            Detail::Grant(g) => display::grant_lines(g),
        };
        if !app.actionable() {
            lines.push("Actions unavailable: resolved, changed, offline, or target unavailable. Esc to refresh selection.".into());
        }
        frame.render_widget(
            Paragraph::new(lines.join("\n"))
                .wrap(Wrap { trim: false })
                .scroll((app.detail_scroll, 0))
                .block(Block::default().borders(Borders::TOP)),
            areas[2],
        );
    } else {
        app.reconcile();
        let entries = app.entries();
        let scope = &mut app.scopes[app.index()];
        let mut state = ListState::default()
            .with_offset(scope.offset)
            .with_selected(entries.iter().position(|(id, _)| Some(id) == scope.selected.as_ref()));
        if entries.is_empty() {
            frame.render_widget(
                Paragraph::new("No matching entries. Waiting for updates...").block(Block::default().borders(Borders::TOP)),
                areas[2],
            );
        } else {
            let list = List::new(entries.iter().map(|(_, text)| ListItem::new(text.as_str())))
                .highlight_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
                .highlight_symbol("> ")
                .block(Block::default().borders(Borders::TOP));
            frame.render_stateful_widget(list, areas[2], &mut state);
            scope.offset = state.offset();
        }
    }
    let help = match app.detail {
        Some(Detail::Request(ref r, _)) if matches!(r.target, AgentPermissionTarget::Surface { .. }) => {
            "a approve  d deny  1 once  2 until window closes  3 persistent\n↑↓ scroll  Esc back  Ctrl-C quit"
        }
        Some(Detail::Request(..)) => "a approve  d deny  1 once  3 persistent\n↑↓ scroll  Esc back  Ctrl-C quit",
        Some(Detail::Grant(..)) => "r revoke grant (future checks only)\n↑↓ scroll  Esc back  Ctrl-C quit",
        None => "↑↓ select  Enter details  Tab Requests/Grants  / filter  Esc clear\nq quit  Ctrl-C quit",
    };
    frame.render_widget(Paragraph::new(help), areas[3]);
    frame.render_widget(
        Paragraph::new(display::clean(&app.message))
            .wrap(Wrap { trim: false })
            .style(Style::default().fg(if app.connected { Color::Gray } else { Color::Yellow })),
        areas[4],
    );
}

struct RestoreTerminal;
impl Drop for RestoreTerminal {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = crossterm::execute!(io::stdout(), crossterm::cursor::Show);
    }
}
struct Worker(tokio::task::JoinHandle<()>);
impl Drop for Worker {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub(super) async fn run(client: &DaemonClient, height: u16) -> Result<(), ClientError> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(ClientError::Local(
            "agents review requires a terminal; use agents requests --json for scripts".into(),
        ));
    }
    let io_error = |e: io::Error| ClientError::Local(e.to_string());
    enable_raw_mode().map_err(io_error)?;
    let _restore = RestoreTerminal;
    let rows = crossterm::terminal::size().map_err(io_error)?.1;
    let mut terminal = Terminal::with_options(
        CrosstermBackend::new(io::stdout()),
        TerminalOptions {
            viewport: Viewport::Inline(height.min(rows.max(1))),
        },
    )
    .map_err(io_error)?;
    terminal.hide_cursor().map_err(io_error)?;
    let (updates_tx, mut updates) = watch::channel(Update::default());
    let (commands, commands_rx) = mpsc::channel(1);
    let _worker = Worker(tokio::spawn(network(client.clone(), updates_tx, commands_rx)));
    let mut app = Inbox {
        message: "Connecting...".into(),
        ..Default::default()
    };
    let mut input = EventStream::new();
    loop {
        terminal.draw(|frame| draw(frame, &mut app)).map_err(io_error)?;
        tokio::select! {
            result = updates.changed() => {
                if result.is_err() { return Err(ClientError::Local("approval update worker stopped".into())); }
                let update = updates.borrow_and_update().clone();
                app.update(update);
            }
            event = input.next() => {
                let Some(event) = event else { break; };
                let Event::Key(key) = event.map_err(io_error)? else { continue; };
                // Holding down an approval key must not consume subsequent requests.
                if key.kind != KeyEventKind::Press { continue; }
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) { break; }
                if app.filtering {
                    match key.code {
                        KeyCode::Esc | KeyCode::Enter => app.filtering = false,
                        KeyCode::Backspace => { let index = app.index(); app.scopes[index].filter.pop(); app.reconcile(); }
                        KeyCode::Char(c) if !c.is_control() => { let index = app.index(); app.scopes[index].filter.push(c); app.reconcile(); }
                        _ => {}
                    }
                    continue;
                }
                match key.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Esc => { if app.detail.take().is_none() { let index = app.index(); app.scopes[index].filter.clear(); app.reconcile(); } }
                    KeyCode::Tab | KeyCode::BackTab if app.detail.is_none() => { app.grants = !app.grants; app.reconcile(); }
                    KeyCode::Char('/') if app.detail.is_none() => app.filtering = true,
                    KeyCode::Down | KeyCode::Char('j') => app.navigate(true),
                    KeyCode::Up | KeyCode::Char('k') => app.navigate(false),
                    KeyCode::Enter if app.detail.is_none() => app.open(),
                    KeyCode::Char(c @ ('1' | '2' | '3')) if !app.busy => {
                        if let Some(Detail::Request(r, duration)) = &mut app.detail {
                            match c {
                                '1' => *duration = AgentPermissionDuration::Once,
                                '2' if matches!(r.target, AgentPermissionTarget::Surface { .. }) => *duration = AgentPermissionDuration::UntilSurfaceGone,
                                '3' => *duration = AgentPermissionDuration::Persistent,
                                _ => {}
                            }
                        }
                    }
                    KeyCode::Char(c) => {
                        if let Some(decision) = app.decision(c) {
                            if commands.try_send(decision).is_ok() { app.busy = true; app.message = "Sending decision...".into(); }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    terminal.clear().map_err(io_error)?;
    terminal.show_cursor().map_err(io_error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use porthole_core::{
        SurfaceId,
        agent_policy::{ActionClass, AgentId, PermissionRequestId},
    };

    use super::*;

    fn request(id: &str) -> AgentPermissionRequestResponse {
        AgentPermissionRequestResponse {
            request_id: PermissionRequestId::from(id),
            agent_id: AgentId::from("agent_1"),
            target: AgentPermissionTarget::Surface {
                surface_id: SurfaceId::from("surf_1"),
            },
            description: Default::default(),
            actions: vec![ActionClass::Drive],
            reason: Some(id.into()),
            status: "pending".into(),
            created_at_unix_ms: 1000,
            resolved_at_unix_ms: None,
        }
    }
    fn live(requests: Vec<AgentPermissionRequestResponse>) -> Update {
        Update {
            snapshot: Some(Snapshot { requests, grants: vec![] }),
            connected: true,
            ..Default::default()
        }
    }
    #[test]
    fn arrivals_do_not_move_selection_and_resolution_never_approves_next_request() {
        let mut inbox = Inbox::default();
        inbox.update(live(vec![request("a"), request("b")]));
        inbox.navigate(true);
        inbox.open();
        inbox.update(live(vec![request("new"), request("a"), request("b")]));
        assert_eq!(inbox.scopes[0].selected.as_deref(), Some("b"));
        assert!(
            matches!(inbox.decision('a'), Some(Decision::Approve(r, AgentPermissionDuration::UntilSurfaceGone)) if r.request_id.as_str() == "b")
        );
        inbox.update(live(vec![request("new"), request("a")]));
        assert!(inbox.decision('a').is_none());
        assert!(inbox.decision('d').is_none());
    }
    #[test]
    fn changed_scope_and_disconnect_disable_actions_but_closed_requests_can_be_denied() {
        let mut inbox = Inbox::default();
        inbox.update(live(vec![request("a")]));
        inbox.open();
        inbox.update(Update {
            connected: false,
            ..Default::default()
        });
        assert!(inbox.decision('a').is_none());
        inbox.update(live(vec![request("a")]));
        let mut changed = request("a");
        changed.actions = vec![ActionClass::Manage];
        inbox.update(live(vec![changed]));
        assert!(inbox.decision('a').is_none());
        let mut closed = request("a");
        closed.description.surface_available = Some(false);
        inbox.update(live(vec![closed]));
        inbox.open();
        assert!(inbox.decision('a').is_none());
        assert!(matches!(inbox.decision('d'), Some(Decision::Deny(_))));
    }
    #[test]
    fn each_scope_retains_its_filter_and_small_terminals_render() {
        let mut inbox = Inbox::default();
        inbox.update(live(vec![request("alpha"), request("beta")]));
        inbox.scopes[0].filter = "beta".into();
        inbox.reconcile();
        inbox.grants = true;
        inbox.scopes[1].filter = "other".into();
        inbox.reconcile();
        inbox.grants = false;
        inbox.reconcile();
        assert_eq!(inbox.scopes[0].filter, "beta");
        assert_eq!(inbox.scopes[0].selected.as_deref(), Some("beta"));
        for (width, height) in [(80, 18), (40, 8), (10, 3)] {
            let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| draw(frame, &mut inbox)).unwrap();
            inbox.open();
            terminal.draw(|frame| draw(frame, &mut inbox)).unwrap();
            inbox.detail = None;
        }
    }
}
