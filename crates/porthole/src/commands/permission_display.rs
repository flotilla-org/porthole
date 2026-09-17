use porthole_core::agent_policy::ActionClass;
use porthole_protocol::agent_permissions::{
    AgentGrantResponse, AgentPermissionAppSelector, AgentPermissionDuration, AgentPermissionRequestResponse, AgentPermissionTarget,
    PermissionDescription, PermissionOperation,
};

// Titles and registered names may contain terminal escapes or bidi controls.
// Never let display metadata become terminal commands or extra UI rows.
pub(super) fn clean(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') {
                ' '
            } else {
                c
            }
        })
        .collect()
}

pub(super) fn target(target: &AgentPermissionTarget, description: &PermissionDescription) -> String {
    let scope = match target {
        AgentPermissionTarget::Surface { surface_id } => format!("this window ({surface_id})"),
        AgentPermissionTarget::FrontmostOnce { surface_id } => format!("selected frontmost window ({surface_id})"),
        AgentPermissionTarget::App { app } => match app {
            AgentPermissionAppSelector::BundleId { bundle_id } => format!("all windows of app {bundle_id}"),
            AgentPermissionAppSelector::ExecutablePath { executable_path } => format!("all windows of executable {executable_path}"),
            AgentPermissionAppSelector::AppName { app_name } => format!("all windows of app named {app_name}"),
        },
        AgentPermissionTarget::LaunchedByAgent => "launches / windows launched by this agent".into(),
        AgentPermissionTarget::AllSurfaces => "all windows".into(),
    };
    let label = description
        .surface
        .as_ref()
        .map(|s| {
            format!(
                "{}: {} | ",
                s.app_name.as_deref().unwrap_or("unknown app"),
                s.title.as_deref().unwrap_or("untitled")
            )
        })
        .unwrap_or_default();
    clean(&format!(
        "{label}{scope}{}",
        if description.surface_available == Some(false) {
            " [closed/unavailable]"
        } else {
            ""
        }
    ))
}

pub(super) fn actions(actions: &[ActionClass]) -> String {
    actions
        .iter()
        .map(|a| match a {
            ActionClass::Observe => "observe",
            ActionClass::Drive => "drive (keyboard, text, pointer)",
            ActionClass::Manage => "manage (launch, place, close)",
            ActionClass::Record => "record",
        })
        .collect::<Vec<_>>()
        .join(" + ")
}

pub(super) fn operation(description: &PermissionDescription, reason: Option<&str>) -> String {
    clean(&match &description.operation {
        Some(PermissionOperation::Key { combinations, event_count }) => format!(
            "send {event_count} key events: {}{}",
            combinations.join(", "),
            if *event_count > combinations.len() { " ..." } else { "" }
        ),
        Some(PermissionOperation::Text { characters }) => format!("type {characters} characters (contents not retained)"),
        Some(PermissionOperation::Launch { application }) => format!("launch {application}"),
        Some(PermissionOperation::Capture { native }) => {
            format!("capture window ({})", if *native { "native frames" } else { "CPU frames" })
        }
        None => reason.unwrap_or("not recorded").into(),
    })
}

pub(super) fn duration(duration: &AgentPermissionDuration) -> String {
    match duration {
        AgentPermissionDuration::Once => "once (next matching operation)".into(),
        AgentPermissionDuration::UntilSurfaceGone => "until this window closes".into(),
        AgentPermissionDuration::Persistent => "persistent (until revoked)".into(),
        AgentPermissionDuration::Session { session } => format!("session {} (not currently enforced)", clean(session)),
        AgentPermissionDuration::TimeBounded { expires_at_unix_ms } => {
            format!("until {}", super::agents::format_unix_ms_utc(*expires_at_unix_ms))
        }
    }
}

pub(super) fn default_duration(target: &AgentPermissionTarget) -> AgentPermissionDuration {
    match target {
        AgentPermissionTarget::Surface { .. } => AgentPermissionDuration::UntilSurfaceGone,
        _ => AgentPermissionDuration::Once,
    }
}

pub(super) fn requester(id: &str, description: &PermissionDescription) -> String {
    clean(&format!(
        "{} ({id}){}",
        description.agent_name.as_deref().unwrap_or("unknown agent"),
        if description.agent_revoked { " [revoked/unavailable]" } else { "" }
    ))
}

pub(super) fn request_lines(r: &AgentPermissionRequestResponse) -> Vec<String> {
    vec![
        format!("request_id: {}", clean(r.request_id.as_str())),
        format!("requester: {}", requester(r.agent_id.as_str(), &r.description)),
        format!("target: {}", target(&r.target, &r.description)),
        format!("first requested operation: {}", operation(&r.description, r.reason.as_deref())),
        format!("permission scope: {}", actions(&r.actions)),
        format!("suggested duration: {}", duration(&default_duration(&r.target))),
        format!("status: {}", clean(&r.status)),
        format!("requested: {}", super::agents::format_unix_ms_utc(r.created_at_unix_ms)),
    ]
}

pub(super) fn grant_lines(g: &AgentGrantResponse) -> Vec<String> {
    let mut lines = vec![
        format!("grant_id: {}", clean(g.grant_id.as_str())),
        format!("requester: {}", requester(g.agent_id.as_str(), &g.description)),
        format!("target: {}", target(&g.target, &g.description)),
        format!("permission scope: {}", actions(&g.actions)),
        format!(
            "first requested operation: {}",
            operation(&g.description, g.origin_reason.as_deref())
        ),
        format!("duration: {}", duration(&g.duration)),
        format!("granted: {}", super::agents::format_unix_ms_utc(g.created_at_unix_ms)),
    ];
    if g.constraints.requires_frontmost {
        lines.push("constraint: requires frontmost".into());
    }
    if let Some(ms) = g.constraints.max_duration_ms {
        lines.push(format!("constraint: maximum duration {ms}ms"));
    }
    if !g.constraints.allowed_input.is_empty() {
        lines.push(format!(
            "constraint: allowed input {}",
            clean(&g.constraints.allowed_input.join(", "))
        ));
    }
    if g.actions.contains(&ActionClass::Record) {
        lines.push("Revoking affects future permission checks; running capture continues.".into());
    }
    lines
}
