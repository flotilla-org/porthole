//! Local operator UI. The daemon owns policy, scope and request state.
use std::{
    collections::HashMap,
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
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState, Wrap},
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
    list_durations: HashMap<String, (AgentPermissionRequestResponse, AgentPermissionDuration)>,
    connected: bool,
    busy: bool,
    completed: u64,
    message: String,
}

struct InboxRow {
    id: String,
    cells: Vec<String>,
}

fn short_actions(actions: &[porthole_core::agent_policy::ActionClass]) -> String {
    use porthole_core::agent_policy::ActionClass;
    actions
        .iter()
        .map(|action| match action {
            ActionClass::Observe => "observe",
            ActionClass::Drive => "drive",
            ActionClass::Manage => "manage",
            ActionClass::Record => "record",
        })
        .collect::<Vec<_>>()
        .join(" + ")
}

// Reserve column minima, then distribute extra space towards targets and operations.
fn inbox_widths(width: u16, grants: bool) -> Vec<u16> {
    let minima = if grants { vec![12, 16, 14, 18, 10] } else { vec![14, 18, 16, 18] };
    let columns = minima.len() as u16;
    let available = width.saturating_sub(columns * 2); // marker + column gaps
    let minimum: u16 = minima.iter().sum();
    if available < minimum {
        let mut widths = vec![available / columns; minima.len()];
        *widths.last_mut().unwrap() += available % columns;
        return widths;
    }
    let mut extra = available - minimum;
    let mut widths = minima;
    if grants {
        // Fit the common "Until window closes" label before widening free text.
        let duration_extra = extra.min(19 - widths[4]);
        widths[4] += duration_extra;
        extra -= duration_extra;
    }
    widths[0] += extra / 6;
    widths[1] += extra / 3;
    widths[2] += extra / 6;
    widths[3] += extra - extra / 6 * 2 - extra / 3;
    widths
}

fn wrapped_cell(value: &str, width: u16, max_lines: u16) -> (Cell<'static>, u16) {
    let mut lines = textwrap::wrap(value, usize::from(width.max(1)));
    if lines.len() > usize::from(max_lines) {
        lines.truncate(usize::from(max_lines));
        if let Some(last) = lines.last_mut() {
            let prefix = if width > 1 {
                textwrap::wrap(last, usize::from(width - 1))
                    .first()
                    .map(|line| line.to_string())
                    .unwrap_or_default()
            } else {
                String::new()
            };
            *last = format!("{prefix}…").into();
        }
    }
    let height = lines.len().max(1) as u16;
    (Cell::from(lines.join("\n")), height)
}

impl Inbox {
    fn index(&self) -> usize {
        usize::from(self.grants)
    }
    fn entries(&self) -> Vec<InboxRow> {
        let filter = self.scopes[self.index()].filter.to_lowercase();
        let rows: Vec<_> = if self.grants {
            self.snapshot
                .grants
                .iter()
                .map(|g| InboxRow {
                    id: g.grant_id.to_string(),
                    cells: vec![
                        display::clean(g.description.agent_name.as_deref().unwrap_or(g.agent_id.as_str())),
                        compact_target(&g.target, &g.description),
                        short_actions(&g.actions),
                        display::operation(&g.description, g.origin_reason.as_deref()),
                        match g.duration {
                            AgentPermissionDuration::Once => "Once".into(),
                            AgentPermissionDuration::UntilSurfaceGone => "Until window closes".into(),
                            AgentPermissionDuration::Persistent => "Persistent".into(),
                            _ => display::duration(&g.duration),
                        },
                    ],
                })
                .collect()
        } else {
            self.snapshot
                .requests
                .iter()
                .map(|r| InboxRow {
                    id: r.request_id.to_string(),
                    cells: vec![
                        display::clean(r.description.agent_name.as_deref().unwrap_or(r.agent_id.as_str())),
                        compact_target(&r.target, &r.description),
                        short_actions(&r.actions),
                        display::operation(&r.description, r.reason.as_deref()),
                    ],
                })
                .collect()
        };
        rows.into_iter()
            .filter(|row| row.id.to_lowercase().contains(&filter) || row.cells.iter().any(|cell| cell.to_lowercase().contains(&filter)))
            .collect()
    }

    fn reconcile(&mut self) -> Vec<InboxRow> {
        let rows = self.entries();
        let scope = &mut self.scopes[self.index()];
        if !rows.iter().any(|row| Some(&row.id) == scope.selected.as_ref()) {
            scope.selected = rows.first().map(|row| row.id.clone());
            scope.offset = 0;
        }
        rows
    }
    fn update(&mut self, update: Update) {
        if let Some(snapshot) = update.snapshot {
            self.snapshot = snapshot;
            let requests: HashMap<_, _> = self.snapshot.requests.iter().map(|r| (r.request_id.as_str(), r)).collect();
            self.list_durations
                .retain(|id, (request, _)| requests.get(id.as_str()).is_some_and(|current| **current == *request));
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
        let current = rows.iter().position(|row| Some(&row.id) == scope.selected.as_ref()).unwrap_or(0);
        let next = if down {
            (current + 1).min(rows.len().saturating_sub(1))
        } else {
            current.saturating_sub(1)
        };
        scope.selected = rows.get(next).map(|row| row.id.clone());
    }
    fn selected_detail(&self) -> Option<Detail> {
        let selected = self.scopes[self.index()].selected.as_deref();
        if self.grants {
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
                    let duration = self
                        .list_durations
                        .get(r.request_id.as_str())
                        .filter(|(request, _)| request == &r)
                        .map(|(_, duration)| duration.clone())
                        .unwrap_or_else(|| display::default_duration(&r.target));
                    Detail::Request(r, duration)
                })
        }
    }
    fn active_detail(&self) -> Option<Detail> {
        self.detail.clone().or_else(|| self.selected_detail())
    }
    fn open(&mut self) {
        self.detail = self.selected_detail();
        self.detail_scroll = 0;
    }
    fn horizontal(&mut self, right: bool) {
        if self.detail.is_some() {
            if !right {
                self.detail = None;
            }
        } else {
            self.grants = right;
            self.reconcile();
        }
    }
    fn choose_duration(&mut self, key: char) {
        if self.busy {
            return;
        }
        if let Some(Detail::Request(r, _)) = self.active_detail() {
            let duration = match key {
                '1' => AgentPermissionDuration::Once,
                '2' if matches!(r.target, AgentPermissionTarget::Surface { .. }) => AgentPermissionDuration::UntilSurfaceGone,
                '3' => AgentPermissionDuration::Persistent,
                _ => return,
            };
            self.list_durations.insert(r.request_id.to_string(), (r.clone(), duration.clone()));
            if self.detail.is_some() {
                self.detail = Some(Detail::Request(r, duration));
            }
        }
    }
    fn actionable(&self) -> bool {
        if !self.connected || self.busy {
            return false;
        }
        match &self.active_detail() {
            Some(Detail::Request(r, _)) => self
                .snapshot
                .requests
                .iter()
                .any(|current| current == r && current.status == "pending"),
            Some(Detail::Grant(g)) => self.snapshot.grants.iter().any(|current| current == g),
            None => false,
        }
    }
    fn decision(&self, key: char) -> Result<Decision, &'static str> {
        if !self.connected {
            return Err("offline; waiting to reconnect.");
        }
        if self.busy {
            return Err("a decision is already in progress.");
        }
        let detail = self.active_detail().ok_or("no entry selected.")?;
        if !self.actionable() {
            return Err("entry changed or was resolved; return to the list to refresh it.");
        }
        match (detail, key) {
            (Detail::Request(r, duration), 'a') => {
                if r.description.agent_revoked {
                    return Err("requester unavailable; it was revoked or removed.");
                }
                if r.description.surface_available == Some(false) {
                    return Err("window unavailable; the agent must find it again and retry.");
                }
                Ok(Decision::Approve(r, duration))
            }
            (Detail::Request(r, _), 'd') => Ok(Decision::Deny(r)),
            (Detail::Grant(g), 'r') => Ok(Decision::Revoke(g.grant_id.to_string())),
            _ => Err("action does not apply to this entry."),
        }
    }
    /// Escape leaves the current nested interaction, then exits from the list.
    fn escape(&mut self) -> bool {
        if self.filtering {
            self.filtering = false;
            false
        } else {
            self.detail.take().is_none()
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
            Ok("Approved.".into())
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
            Ok("Denied this request.".into())
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
                    state.message = "Live. Approval grants the permission scope; Operation shows what prompted it.".into();
                }
                state.connected = true;
            }
            result => {
                state.connected = false;
                state.message = match result {
                    Ok(Ok(_)) => "Event stream unavailable. Retrying; actions disabled.".into(),
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
    match (target, &description.surface) {
        (AgentPermissionTarget::Surface { .. }, Some(s)) => display::clean(&format!(
            "{}: {}{}",
            s.app_name.as_deref().unwrap_or("unknown app"),
            s.title.as_deref().unwrap_or("untitled"),
            if description.surface_available == Some(false) {
                " [closed/unavailable]"
            } else {
                ""
            }
        )),
        // A triggering window is context, not the grant's scope. In compact
        // rows keep broader selectors visible instead of substituting its title.
        _ => display::target(
            target,
            &porthole_protocol::agent_permissions::PermissionDescription {
                surface_available: description.surface_available,
                ..Default::default()
            },
        ),
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
    let context = match (app.active_detail(), app.decision('a')) {
        (Some(Detail::Request(..)), Err(reason)) if !app.filtering => {
            format!("Cannot approve: {reason}")
        }
        (Some(Detail::Request(_, duration)), _) if !app.filtering => {
            let filter = &app.scopes[app.index()].filter;
            format!(
                "APPROVE FOR: {}{}",
                display::duration(&duration),
                if filter.is_empty() {
                    String::new()
                } else {
                    format!(" | Filter: {filter}")
                }
            )
        }
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
                let description = app
                    .snapshot
                    .requests
                    .iter()
                    .find(|current| current.request_id == r.request_id)
                    .map(|current| &current.description)
                    .unwrap_or(&r.description);
                if description.agent_revoked || description.surface_available == Some(false) {
                    lines.push("Approval unavailable: requester revoked or window unavailable.".into());
                }
                lines
            }
            Detail::Grant(g) => display::grant_lines(g),
        };
        if !app.actionable() {
            lines.push("Actions unavailable: resolved, changed, offline, or target unavailable. Esc to refresh selection.".into());
        }
        let label_width = areas[2].width.saturating_sub(3).min(14);
        let value_width = areas[2].width.saturating_sub(label_width + 2).max(1);
        let mut rows = Vec::new();
        for line in lines {
            let (label, value) = line.split_once(": ").unwrap_or(("", &line));
            let label = match label {
                "request_id" => "Request ID",
                "grant_id" => "Grant ID",
                "requester" => "Requester",
                "target" => "Target",
                "first requested operation" => "Requested",
                "permission scope" => "Allows",
                "status" => "Status",
                "requested" => "Created",
                "duration" => "Duration",
                "granted" => "Granted",
                "constraint" => "Constraint",
                "Actions unavailable" | "Approval unavailable" => "Unavailable",
                other => other,
            };
            for (index, wrapped) in textwrap::wrap(value, usize::from(value_width)).into_iter().enumerate() {
                rows.push(Row::new(vec![
                    Cell::from(if index == 0 { label.to_owned() } else { String::new() }).style(Style::default().fg(Color::Cyan)),
                    Cell::from(wrapped.into_owned()),
                ]));
            }
        }
        let visible = usize::from(areas[2].height.saturating_sub(1));
        app.detail_scroll = app
            .detail_scroll
            .min(rows.len().saturating_sub(visible).min(u16::MAX as usize) as u16);
        let table = Table::new(
            rows.into_iter().skip(usize::from(app.detail_scroll)),
            [Constraint::Length(label_width), Constraint::Min(1)],
        )
        .column_spacing(2)
        .block(Block::default().borders(Borders::TOP));
        frame.render_widget(table, areas[2]);
    } else {
        let entries = app.reconcile();
        let scope = &mut app.scopes[app.index()];
        let mut state = TableState::default()
            .with_offset(scope.offset)
            .with_selected(entries.iter().position(|row| Some(&row.id) == scope.selected.as_ref()));
        let widths = inbox_widths(areas[2].width, app.grants);
        let max_lines = areas[2].height.saturating_sub(3).clamp(1, 3);
        let rows = entries.iter().map(|entry| {
            let mut height = 1;
            let cells = entry
                .cells
                .iter()
                .zip(widths.iter().copied())
                .map(|(text, width)| {
                    let (cell, lines) = wrapped_cell(text, width, max_lines);
                    height = height.max(lines);
                    cell
                })
                .collect::<Vec<_>>();
            Row::new(cells).height(height)
        });
        let mut headers = vec!["Requester", "Target", "Permissions", "Operation"];
        if app.grants {
            headers.push("Duration");
        }
        let header = Row::new(headers)
            .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
            .bottom_margin(1);
        let table = Table::new(rows, widths.iter().copied().map(Constraint::Length))
            .header(header)
            .column_spacing(2)
            .row_highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White).add_modifier(Modifier::BOLD))
            .highlight_symbol("> ")
            .block(Block::default().borders(Borders::TOP));
        frame.render_stateful_widget(table, areas[2], &mut state);
        scope.offset = state.offset();
        if entries.is_empty() && areas[2].height > 3 {
            let mut empty = areas[2];
            empty.y += 3;
            empty.height -= 3;
            frame.render_widget(Paragraph::new("No matching entries. Waiting for updates..."), empty);
        }
    }
    let actions = match app.active_detail() {
        _ if app.filtering => "Type to filter  Ctrl-U clear  Enter/Esc finish",
        Some(Detail::Request(..)) if app.decision('a').is_err() => {
            if app.decision('d').is_ok() {
                "d deny request"
            } else {
                "Actions unavailable"
            }
        }
        Some(Detail::Grant(..)) if app.decision('r').is_err() => "Actions unavailable",
        Some(Detail::Request(ref r, _)) if matches!(r.target, AgentPermissionTarget::Surface { .. }) => {
            "a approve  d deny  1 once  2 until window closes  3 persistent"
        }
        Some(Detail::Request(..)) => "a approve  d deny  1 once  3 persistent",
        Some(Detail::Grant(..)) => "r revoke grant (future checks only)",
        None => "",
    };
    let navigation = if app.detail.is_some() {
        "↑↓ scroll  ←/Esc back  q/Ctrl-C quit"
    } else {
        "↑↓ select  ←→/Tab tabs  Enter details  / filter  Esc/q quit"
    };
    let help = format!("{actions}\n{navigation}");
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
                // Ignore repeat events from terminals that report them; busy also prevents overlapping decisions.
                if key.kind != KeyEventKind::Press { continue; }
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) { break; }
                if key.code == KeyCode::Esc {
                    if app.escape() { break; }
                    continue;
                }
                if app.filtering {
                    match key.code {
                        KeyCode::Enter => app.filtering = false,
                        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            let index = app.index(); app.scopes[index].filter.clear(); app.reconcile();
                        }
                        KeyCode::Backspace => { let index = app.index(); app.scopes[index].filter.pop(); app.reconcile(); }
                        KeyCode::Char(c) if !c.is_control() => { let index = app.index(); app.scopes[index].filter.push(c); app.reconcile(); }
                        _ => {}
                    }
                    continue;
                }
                match key.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Tab | KeyCode::BackTab if app.detail.is_none() => { app.grants = !app.grants; app.reconcile(); }
                    KeyCode::Left => app.horizontal(false),
                    KeyCode::Right => app.horizontal(true),
                    KeyCode::Char('/') if app.detail.is_none() => app.filtering = true,
                    KeyCode::Down | KeyCode::Char('j') => app.navigate(true),
                    KeyCode::Up | KeyCode::Char('k') => app.navigate(false),
                    KeyCode::Enter if app.detail.is_none() => app.open(),
                    KeyCode::Char(c @ ('1' | '2' | '3')) => app.choose_duration(c),
                    KeyCode::Char(c @ ('a' | 'd' | 'r')) => {
                        match app.decision(c) {
                            Ok(decision) => match commands.try_send(decision) {
                                Ok(()) => { app.busy = true; app.message = "Sending decision...".into(); }
                                Err(mpsc::error::TrySendError::Full(_)) => app.message = "Not sent: another decision is queued. Try again after it completes.".into(),
                                Err(mpsc::error::TrySendError::Closed(_)) => app.message = "Not sent: approval worker stopped. Reopen the inbox.".into(),
                            },
                            Err(reason) => {
                                let action = match c { 'a' => "approve", 'd' => "deny", _ => "revoke" };
                                app.message = format!("Cannot {action}: {reason}");
                            }
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
            invalidation_reason: None,
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
            matches!(inbox.decision('a'), Ok(Decision::Approve(r, AgentPermissionDuration::UntilSurfaceGone)) if r.request_id.as_str() == "b")
        );
        inbox.update(live(vec![request("new"), request("a")]));
        assert!(inbox.decision('a').is_err());
        assert!(inbox.decision('d').is_err());
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
        assert!(inbox.decision('a').is_err());
        inbox.update(live(vec![request("a")]));
        let mut changed = request("a");
        changed.actions = vec![ActionClass::Manage];
        inbox.update(live(vec![changed]));
        assert!(inbox.decision('a').is_err());
        let mut closed = request("a");
        closed.description.surface_available = Some(false);
        inbox.update(live(vec![closed]));
        inbox.open();
        assert!(inbox.decision('a').is_err());
        assert!(matches!(inbox.decision('d'), Ok(Decision::Deny(_))));
    }
    #[test]
    fn open_details_explain_window_closure_without_replacing_the_reviewed_request() {
        let mut inbox = Inbox::default();
        inbox.update(live(vec![request("window")]));
        inbox.open();
        let mut closed = request("window");
        closed.description.surface_available = Some(false);
        inbox.update(live(vec![closed]));
        let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(110, 30)).unwrap();
        terminal.draw(|frame| draw(frame, &mut inbox)).unwrap();
        let screen = terminal.backend().buffer().content.iter().map(|c| c.symbol()).collect::<String>();
        assert!(screen.contains("requester revoked or window unavailable"));
        assert!(matches!(inbox.detail, Some(Detail::Request(ref r, _)) if r.description.surface_available.is_none()));
        assert!(inbox.decision('a').is_err());
    }

    #[test]
    fn list_actions_use_selected_request_and_duration_without_opening_details() {
        let mut inbox = Inbox::default();
        let mut broad = request("broad");
        broad.target = AgentPermissionTarget::AllSurfaces;
        inbox.update(live(vec![request("window"), broad]));
        assert!(
            matches!(inbox.decision('a'), Ok(Decision::Approve(r, AgentPermissionDuration::UntilSurfaceGone)) if r.request_id.as_str() == "window")
        );
        inbox.choose_duration('3');
        inbox.open();
        inbox.horizontal(false);
        assert!(inbox.detail.is_none());
        assert!(matches!(
            inbox.decision('a'),
            Ok(Decision::Approve(_, AgentPermissionDuration::Persistent))
        ));
        inbox.navigate(true);
        inbox.choose_duration('2'); // Invalid for all windows; retain its default.
        assert!(matches!(inbox.decision('a'), Ok(Decision::Approve(r, AgentPermissionDuration::Once)) if r.request_id.as_str() == "broad"));
        assert!(matches!(inbox.decision('d'), Ok(Decision::Deny(r)) if r.request_id.as_str() == "broad"));
        inbox.busy = true;
        assert!(inbox.decision('a').is_err());
        inbox.busy = false;
        inbox.connected = false;
        assert!(inbox.decision('d').is_err());
    }

    #[test]
    fn duration_choices_survive_other_choices_but_not_request_changes_or_removal() {
        let mut inbox = Inbox::default();
        inbox.update(live(vec![request("a"), request("b")]));
        inbox.choose_duration('3');
        inbox.navigate(true);
        inbox.choose_duration('1');
        inbox.navigate(false);
        assert!(matches!(
            inbox.decision('a'),
            Ok(Decision::Approve(_, AgentPermissionDuration::Persistent))
        ));
        let mut changed = request("a");
        changed.actions = vec![ActionClass::Observe];
        inbox.update(live(vec![changed, request("b")]));
        assert!(matches!(
            inbox.decision('a'),
            Ok(Decision::Approve(_, AgentPermissionDuration::UntilSurfaceGone))
        ));
        inbox.navigate(true);
        assert!(matches!(
            inbox.decision('a'),
            Ok(Decision::Approve(_, AgentPermissionDuration::Once))
        ));
        inbox.update(live(vec![]));
        assert!(inbox.list_durations.is_empty());
    }

    #[test]
    fn arrows_switch_views_and_grant_list_retains_operation_and_duration() {
        let mut inbox = Inbox::default();
        inbox.update(live(vec![request("window")]));
        inbox.snapshot.grants.push(AgentGrantResponse {
            description: Default::default(),
            grant_id: "grant_1".into(),
            agent_id: "agent_1".into(),
            origin_request_id: Some("window".into()),
            origin_reason: Some("search surfaces".into()),
            target: AgentPermissionTarget::AllSurfaces,
            actions: vec![ActionClass::Observe],
            duration: AgentPermissionDuration::Persistent,
            constraints: Default::default(),
            created_at_unix_ms: 1000,
            expires_at_unix_ms: None,
            consumed_at_unix_ms: None,
            revoked_at_unix_ms: None,
        });
        inbox.horizontal(true);
        assert!(inbox.grants);
        assert!(matches!(inbox.decision('r'), Ok(Decision::Revoke(id)) if id == "grant_1"));
        let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(110, 18)).unwrap();
        terminal.draw(|frame| draw(frame, &mut inbox)).unwrap();
        let screen = terminal.backend().buffer().content.iter().map(|c| c.symbol()).collect::<String>();
        assert!(screen.contains("search surfaces"));
        assert!(screen.contains("Persistent"));
        inbox.open();
        inbox.horizontal(true); // Right does not leave details.
        assert!(inbox.detail.is_some());
        inbox.horizontal(false);
        assert!(inbox.detail.is_none());
        assert!(inbox.grants);
        inbox.horizontal(false);
        assert!(!inbox.grants);
        assert_eq!(inbox.scopes[0].selected.as_deref(), Some("window"));
    }

    #[test]
    fn broad_scope_is_visible_even_when_the_trigger_has_window_metadata() {
        use porthole_protocol::agent_permissions::{AgentPermissionAppSelector, PermissionDescription, PermissionSurface};
        let description = PermissionDescription {
            surface: Some(PermissionSurface {
                app_name: Some("Kitty".into()),
                title: Some("project".into()),
                pid: Some(42),
            }),
            ..Default::default()
        };
        assert_eq!(compact_target(&AgentPermissionTarget::AllSurfaces, &description), "all windows");
        assert_eq!(
            compact_target(
                &AgentPermissionTarget::App {
                    app: AgentPermissionAppSelector::AppName { app_name: "Kitty".into() },
                },
                &description
            ),
            "all windows of app named Kitty"
        );
        assert!(
            compact_target(
                &AgentPermissionTarget::FrontmostOnce {
                    surface_id: "surf_1".into(),
                },
                &description
            )
            .starts_with("selected frontmost window")
        );
        assert_eq!(
            compact_target(
                &AgentPermissionTarget::Surface {
                    surface_id: "surf_1".into(),
                },
                &description
            ),
            "Kitty: project"
        );
    }

    #[test]
    fn unavailable_request_explains_why_approval_is_blocked_and_only_offers_denial() {
        let mut inbox = Inbox::default();
        let mut unavailable = request("old_request");
        unavailable.description.surface_available = Some(false);
        inbox.update(live(vec![unavailable]));
        assert!(inbox.decision('a').is_err());
        assert!(matches!(inbox.decision('d'), Ok(Decision::Deny(_))));
        let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(110, 18)).unwrap();
        terminal.draw(|frame| draw(frame, &mut inbox)).unwrap();
        let screen = terminal.backend().buffer().content.iter().map(|c| c.symbol()).collect::<String>();
        assert!(screen.contains("Cannot approve: window unavailable"), "{screen}");
        assert!(!screen.contains("a approve"));
        assert!(screen.contains("d deny"));
    }

    #[test]
    fn escape_leaves_filter_or_details_then_quits_even_with_a_filter() {
        let mut inbox = Inbox::default();
        inbox.update(live(vec![request("a")]));
        inbox.scopes[0].filter = "a".into();
        inbox.filtering = true;
        assert!(!inbox.escape());
        assert!(!inbox.filtering);
        assert_eq!(inbox.scopes[0].filter, "a");
        inbox.open();
        assert!(!inbox.escape());
        assert!(inbox.detail.is_none());
        assert!(inbox.escape());
        assert_eq!(inbox.scopes[0].filter, "a");
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
