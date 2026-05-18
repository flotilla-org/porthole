use std::fmt;

use serde::{Deserialize, Serialize};

use crate::SurfaceId;

macro_rules! string_id {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self(s)
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_string())
            }
        }
    };
}

string_id!(AgentId);
string_id!(GrantId);
string_id!(DenialId);
string_id!(PermissionRequestId);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionClass {
    Observe,
    Drive,
    Manage,
    Record,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppSelector {
    BundleId(String),
    ExecutablePath(String),
    AppName(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetSelector {
    Surface { surface_id: SurfaceId },
    App { app: AppSelector },
    LaunchedByAgent,
    FrontmostOnce { surface_id: SurfaceId },
    AllSurfaces,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DurationSpec {
    Once,
    UntilSurfaceGone,
    /// Reserved until porthole has an explicit daemon session tag. Session
    /// grants do not authorize routes until that concept exists.
    Session {
        session: String,
    },
    TimeBounded {
        expires_at_unix_ms: u64,
    },
    Persistent,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Constraint {
    RequiresFrontmost,
    /// Stored on grants and enforced by route-specific execution guards that
    /// know the requested operation duration.
    MaxDurationMs(u64),
    /// Stored on grants and enforced by route-specific drive guards that know
    /// the concrete input payload.
    AllowedInput(Vec<String>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthorizationDecision {
    Allowed { grant_id: GrantId, consumes_grant: bool },
    Denied { denial_id: DenialId },
    NeedsPermission,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentContext {
    pub agent_id: AgentId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetContext {
    pub surface_id: Option<SurfaceId>,
    pub app_bundle_id: Option<String>,
    pub executable_path: Option<String>,
    pub app_name: Option<String>,
    pub launched_by_agent: Option<AgentId>,
    pub frontmost_surface_id: Option<SurfaceId>,
    pub surface_alive: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    pub grant_id: GrantId,
    pub agent_id: AgentId,
    pub target: TargetSelector,
    pub actions: Vec<ActionClass>,
    pub duration: DurationSpec,
    pub constraints: Vec<Constraint>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Denial {
    pub denial_id: DenialId,
    pub agent_id: AgentId,
    pub target: TargetSelector,
    pub actions: Vec<ActionClass>,
    pub expires_at_unix_ms: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PolicySnapshot {
    pub grants: Vec<Grant>,
    pub denials: Vec<Denial>,
    pub consumed_once_grants: Vec<GrantId>,
}

impl PolicySnapshot {
    pub fn authorize(
        &self,
        agent: &AgentContext,
        target: &TargetContext,
        actions: &[ActionClass],
        now_unix_ms: u64,
    ) -> AuthorizationDecision {
        for denial in &self.denials {
            if denial_matches(denial, agent, target, actions, now_unix_ms) {
                return AuthorizationDecision::Denied {
                    denial_id: denial.denial_id.clone(),
                };
            }
        }

        for grant in &self.grants {
            if grant_matches(grant, agent, target, actions, now_unix_ms, &self.consumed_once_grants) {
                return AuthorizationDecision::Allowed {
                    grant_id: grant.grant_id.clone(),
                    consumes_grant: grant.duration == DurationSpec::Once,
                };
            }
        }

        AuthorizationDecision::NeedsPermission
    }
}

fn denial_matches(denial: &Denial, agent: &AgentContext, target: &TargetContext, actions: &[ActionClass], now_unix_ms: u64) -> bool {
    denial.agent_id == agent.agent_id
        && denial
            .expires_at_unix_ms
            .is_none_or(|expires_at_unix_ms| expires_at_unix_ms > now_unix_ms)
        && target_matches(&denial.target, agent, target)
        && actions_match(&denial.actions, actions)
}

fn grant_matches(
    grant: &Grant,
    agent: &AgentContext,
    target: &TargetContext,
    actions: &[ActionClass],
    now_unix_ms: u64,
    consumed_once_grants: &[GrantId],
) -> bool {
    grant.agent_id == agent.agent_id
        && !consumed_once_grants.contains(&grant.grant_id)
        && duration_matches(&grant.duration, target, now_unix_ms)
        && target_matches(&grant.target, agent, target)
        && actions_match(&grant.actions, actions)
        && constraints_match(&grant.constraints, target)
}

fn duration_matches(duration: &DurationSpec, target: &TargetContext, now_unix_ms: u64) -> bool {
    match duration {
        DurationSpec::Once => true,
        DurationSpec::UntilSurfaceGone => target.surface_alive,
        DurationSpec::Session { .. } => false,
        DurationSpec::TimeBounded { expires_at_unix_ms } => *expires_at_unix_ms > now_unix_ms,
        DurationSpec::Persistent => true,
    }
}

fn target_matches(selector: &TargetSelector, agent: &AgentContext, target: &TargetContext) -> bool {
    if !target.surface_alive {
        return false;
    }

    match selector {
        TargetSelector::Surface { surface_id } => target.surface_id.as_ref() == Some(surface_id),
        TargetSelector::App { app } => match app {
            AppSelector::BundleId(bundle_id) => target.app_bundle_id.as_ref() == Some(bundle_id),
            AppSelector::ExecutablePath(path) => target.executable_path.as_ref() == Some(path),
            AppSelector::AppName(name) => target.app_name.as_ref() == Some(name),
        },
        TargetSelector::LaunchedByAgent => target.launched_by_agent.as_ref() == Some(&agent.agent_id),
        TargetSelector::FrontmostOnce { surface_id } => {
            target.surface_id.as_ref() == Some(surface_id) && target.frontmost_surface_id.as_ref() == Some(surface_id)
        }
        TargetSelector::AllSurfaces => target.surface_id.is_some(),
    }
}

fn actions_match(allowed: &[ActionClass], requested: &[ActionClass]) -> bool {
    !requested.is_empty() && requested.iter().all(|action| allowed.contains(action))
}

fn constraints_match(constraints: &[Constraint], target: &TargetContext) -> bool {
    constraints.iter().all(|constraint| match constraint {
        Constraint::RequiresFrontmost => match (&target.surface_id, &target.frontmost_surface_id) {
            (Some(surface_id), Some(frontmost_surface_id)) => surface_id == frontmost_surface_id,
            _ => false,
        },
        Constraint::MaxDurationMs(_) | Constraint::AllowedInput(_) => true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_000;

    fn agent(id: &str) -> AgentId {
        AgentId::from(id)
    }

    fn grant_id(id: &str) -> GrantId {
        GrantId::from(id)
    }

    fn denial_id(id: &str) -> DenialId {
        DenialId::from(id)
    }

    fn surf(id: &str) -> SurfaceId {
        SurfaceId::from(id)
    }

    fn target_context(surface_id: &str) -> TargetContext {
        TargetContext {
            surface_id: Some(surf(surface_id)),
            app_bundle_id: None,
            executable_path: None,
            app_name: None,
            launched_by_agent: None,
            frontmost_surface_id: None,
            surface_alive: true,
        }
    }

    fn grant(target: TargetSelector, actions: Vec<ActionClass>) -> Grant {
        Grant {
            grant_id: grant_id("grant_1"),
            agent_id: agent("agent_1"),
            target,
            actions,
            duration: DurationSpec::UntilSurfaceGone,
            constraints: Vec::new(),
        }
    }

    fn denial(target: TargetSelector, actions: Vec<ActionClass>) -> Denial {
        Denial {
            denial_id: denial_id("denial_1"),
            agent_id: agent("agent_1"),
            target,
            actions,
            expires_at_unix_ms: None,
        }
    }

    #[test]
    fn allow_grant_matches_agent_surface_and_action() {
        let snapshot = PolicySnapshot {
            grants: vec![grant(
                TargetSelector::Surface {
                    surface_id: surf("surf_1"),
                },
                vec![ActionClass::Drive],
            )],
            denials: Vec::new(),
            consumed_once_grants: Vec::new(),
        };

        let decision = snapshot.authorize(
            &AgentContext {
                agent_id: agent("agent_1"),
            },
            &target_context("surf_1"),
            &[ActionClass::Drive],
            NOW,
        );

        assert_eq!(
            decision,
            AuthorizationDecision::Allowed {
                grant_id: grant_id("grant_1"),
                consumes_grant: false
            }
        );
    }

    #[test]
    fn denial_takes_precedence_over_allow() {
        let snapshot = PolicySnapshot {
            grants: vec![grant(
                TargetSelector::Surface {
                    surface_id: surf("surf_1"),
                },
                vec![ActionClass::Drive],
            )],
            denials: vec![denial(
                TargetSelector::Surface {
                    surface_id: surf("surf_1"),
                },
                vec![ActionClass::Drive],
            )],
            consumed_once_grants: Vec::new(),
        };

        let decision = snapshot.authorize(
            &AgentContext {
                agent_id: agent("agent_1"),
            },
            &target_context("surf_1"),
            &[ActionClass::Drive],
            NOW,
        );

        assert_eq!(
            decision,
            AuthorizationDecision::Denied {
                denial_id: denial_id("denial_1")
            }
        );
    }

    #[test]
    fn expired_grant_does_not_authorize() {
        let mut expired = grant(
            TargetSelector::Surface {
                surface_id: surf("surf_1"),
            },
            vec![ActionClass::Drive],
        );
        expired.duration = DurationSpec::TimeBounded {
            expires_at_unix_ms: NOW - 1,
        };
        let snapshot = PolicySnapshot {
            grants: vec![expired],
            denials: Vec::new(),
            consumed_once_grants: Vec::new(),
        };

        let decision = snapshot.authorize(
            &AgentContext {
                agent_id: agent("agent_1"),
            },
            &target_context("surf_1"),
            &[ActionClass::Drive],
            NOW,
        );

        assert_eq!(decision, AuthorizationDecision::NeedsPermission);
    }

    #[test]
    fn launched_by_agent_selector_matches_agent_owned_surface() {
        let snapshot = PolicySnapshot {
            grants: vec![grant(TargetSelector::LaunchedByAgent, vec![ActionClass::Drive])],
            denials: Vec::new(),
            consumed_once_grants: Vec::new(),
        };
        let mut target = target_context("surf_1");
        target.launched_by_agent = Some(agent("agent_1"));

        let decision = snapshot.authorize(
            &AgentContext {
                agent_id: agent("agent_1"),
            },
            &target,
            &[ActionClass::Drive],
            NOW,
        );

        assert_eq!(
            decision,
            AuthorizationDecision::Allowed {
                grant_id: grant_id("grant_1"),
                consumes_grant: false
            }
        );
    }

    #[test]
    fn app_selector_matches_bundle_executable_or_app_name() {
        let agent = AgentContext {
            agent_id: agent("agent_1"),
        };
        let cases = [
            (
                TargetSelector::App {
                    app: AppSelector::BundleId("com.example.Editor".into()),
                },
                TargetContext {
                    app_bundle_id: Some("com.example.Editor".into()),
                    ..target_context("surf_1")
                },
            ),
            (
                TargetSelector::App {
                    app: AppSelector::ExecutablePath("/Applications/Editor.app/Contents/MacOS/Editor".into()),
                },
                TargetContext {
                    executable_path: Some("/Applications/Editor.app/Contents/MacOS/Editor".into()),
                    ..target_context("surf_2")
                },
            ),
            (
                TargetSelector::App {
                    app: AppSelector::AppName("Editor".into()),
                },
                TargetContext {
                    app_name: Some("Editor".into()),
                    ..target_context("surf_3")
                },
            ),
        ];

        for (selector, target) in cases {
            let snapshot = PolicySnapshot {
                grants: vec![grant(selector, vec![ActionClass::Drive])],
                denials: Vec::new(),
                consumed_once_grants: Vec::new(),
            };
            assert!(matches!(
                snapshot.authorize(&agent, &target, &[ActionClass::Drive], NOW),
                AuthorizationDecision::Allowed { .. }
            ));
        }
    }

    #[test]
    fn frontmost_once_selector_matches_only_the_approved_frontmost_surface() {
        let snapshot = PolicySnapshot {
            grants: vec![grant(
                TargetSelector::FrontmostOnce {
                    surface_id: surf("surf_1"),
                },
                vec![ActionClass::Drive],
            )],
            denials: Vec::new(),
            consumed_once_grants: Vec::new(),
        };
        let mut frontmost = target_context("surf_1");
        frontmost.frontmost_surface_id = Some(surf("surf_1"));
        let mut background = target_context("surf_1");
        background.frontmost_surface_id = Some(surf("surf_2"));

        assert!(matches!(
            snapshot.authorize(
                &AgentContext {
                    agent_id: agent("agent_1"),
                },
                &frontmost,
                &[ActionClass::Drive],
                NOW
            ),
            AuthorizationDecision::Allowed { .. }
        ));
        assert_eq!(
            snapshot.authorize(
                &AgentContext {
                    agent_id: agent("agent_1"),
                },
                &background,
                &[ActionClass::Drive],
                NOW
            ),
            AuthorizationDecision::NeedsPermission
        );
    }

    #[test]
    fn constraints_are_carried_on_grants() {
        let grant = Grant {
            constraints: vec![
                Constraint::RequiresFrontmost,
                Constraint::MaxDurationMs(2_000),
                Constraint::AllowedInput(vec!["text".into()]),
            ],
            ..grant(
                TargetSelector::Surface {
                    surface_id: surf("surf_1"),
                },
                vec![ActionClass::Drive],
            )
        };

        assert_eq!(
            grant.constraints,
            vec![
                Constraint::RequiresFrontmost,
                Constraint::MaxDurationMs(2_000),
                Constraint::AllowedInput(vec!["text".into()])
            ]
        );
    }

    #[test]
    fn requires_frontmost_constraint_authorizes_only_frontmost_surface() {
        let constrained = Grant {
            constraints: vec![Constraint::RequiresFrontmost],
            ..grant(
                TargetSelector::Surface {
                    surface_id: surf("surf_1"),
                },
                vec![ActionClass::Drive],
            )
        };
        let snapshot = PolicySnapshot {
            grants: vec![constrained],
            denials: Vec::new(),
            consumed_once_grants: Vec::new(),
        };
        let mut frontmost = target_context("surf_1");
        frontmost.frontmost_surface_id = Some(surf("surf_1"));
        let mut background = target_context("surf_1");
        background.frontmost_surface_id = Some(surf("surf_2"));

        assert!(matches!(
            snapshot.authorize(
                &AgentContext {
                    agent_id: agent("agent_1"),
                },
                &frontmost,
                &[ActionClass::Drive],
                NOW
            ),
            AuthorizationDecision::Allowed { .. }
        ));
        assert_eq!(
            snapshot.authorize(
                &AgentContext {
                    agent_id: agent("agent_1"),
                },
                &background,
                &[ActionClass::Drive],
                NOW
            ),
            AuthorizationDecision::NeedsPermission
        );
    }

    #[test]
    fn all_surfaces_selector_authorizes_any_surface_for_agent() {
        let snapshot = PolicySnapshot {
            grants: vec![grant(TargetSelector::AllSurfaces, vec![ActionClass::Drive])],
            denials: Vec::new(),
            consumed_once_grants: Vec::new(),
        };

        for surface_id in ["surf_1", "surf_2"] {
            assert!(matches!(
                snapshot.authorize(
                    &AgentContext {
                        agent_id: agent("agent_1"),
                    },
                    &target_context(surface_id),
                    &[ActionClass::Drive],
                    NOW
                ),
                AuthorizationDecision::Allowed { .. }
            ));
        }
    }

    #[test]
    fn once_grant_marked_consumed_is_not_authorized_again() {
        let mut once = grant(
            TargetSelector::Surface {
                surface_id: surf("surf_1"),
            },
            vec![ActionClass::Drive],
        );
        once.duration = DurationSpec::Once;
        let snapshot = PolicySnapshot {
            grants: vec![once],
            denials: Vec::new(),
            consumed_once_grants: vec![grant_id("grant_1")],
        };

        let decision = snapshot.authorize(
            &AgentContext {
                agent_id: agent("agent_1"),
            },
            &target_context("surf_1"),
            &[ActionClass::Drive],
            NOW,
        );

        assert_eq!(decision, AuthorizationDecision::NeedsPermission);
    }
}
