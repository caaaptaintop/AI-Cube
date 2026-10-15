use crate::InstallTarget;
use std::path::{Path, PathBuf};

mod installed;
pub use installed::installed_agents;

/// Installed Agents first; names are alphabetical regardless of letter case.
pub fn display_sort_key(name: &str, installed: bool) -> (bool, String) {
    (!installed, name.to_ascii_lowercase())
}

/// User-level roots must not be derived from project-relative directories.
pub fn global_target_root(home: &Path, target: InstallTarget) -> PathBuf {
    assert_ne!(target, InstallTarget::Proma, "Proma has no global skill target");
    match target {
        InstallTarget::Codex => std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".codex"))
            .join("skills"),
        InstallTarget::Pi => std::env::var_os("PI_CODING_AGENT_DIR")
            .map(|value| {
                let value = value.to_string_lossy();
                if let Some(relative) = value.strip_prefix("~/") {
                    home.join(relative)
                } else {
                    home.join(value.as_ref())
                }
            })
            .unwrap_or_else(|| home.join(".pi/agent"))
            .join("skills"),
        InstallTarget::Omp => omp_agent_dir(home).join("skills"),
        InstallTarget::OpenCode => std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"))
            .join("opencode/skills"),
        InstallTarget::Antigravity => home.join(".gemini/config/skills"),
        InstallTarget::Copilot => home.join(".copilot/skills"),
        _ => home.join(target_directory(target)),
    }
}

/// OMP profiles use separate agent directories; the default profile honors Pi's
/// shared environment variable, but keeps its own default directory.
pub(crate) fn omp_agent_dir(home: &Path) -> PathBuf {
    let profile = std::env::var("OMP_PROFILE")
        .or_else(|_| std::env::var("PI_PROFILE"))
        .unwrap_or_default();
    let profile = profile.trim();
    if !profile.is_empty()
        && profile != "default"
        && profile.len() <= 64
        && profile
            .bytes()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && profile.bytes().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'-' | b'_' | b'.')
        })
    {
        return home.join(".omp/profiles").join(profile).join("agent");
    }
    std::env::var_os("PI_CODING_AGENT_DIR")
        .map(|value| {
            let value = value.to_string_lossy();
            value.strip_prefix("~/").map_or_else(
                || PathBuf::from(value.as_ref()),
                |relative| home.join(relative),
            )
        })
        .unwrap_or_else(|| home.join(".omp/agent"))
}

pub fn installation_root(project: &Path, target: InstallTarget) -> PathBuf {
    if target != InstallTarget::Proma && dirs::home_dir().as_deref() == Some(project) {
        global_target_root(project, target)
    } else {
        project.join(target_directory(target))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstallTargetInfo {
    pub target: InstallTarget,
    pub name: &'static str,
    pub icon_path: &'static str,
}

/// Every project directory Kitter can create, in the order used by project
/// snapshots and the install UI.  `Universal` is the shared compatibility
/// directory; the remaining entries are provider-specific project roots.
/// Proma is handled separately, only for registered native workspaces. In
/// particular, a business repository's plain `skills` is not an Agent root.
pub const PROJECT_INSTALL_TARGETS: &[InstallTarget] = &[
    InstallTarget::Universal,
    InstallTarget::Codex,
    InstallTarget::ClaudeCode,
    InstallTarget::Cursor,
    InstallTarget::OpenCode,
    InstallTarget::Pi,
    InstallTarget::Omp,
    InstallTarget::Grok,
    InstallTarget::Antigravity,
    InstallTarget::Droid,
    InstallTarget::Copilot,
];

pub const INDEPENDENT_INSTALL_TARGETS: &[InstallTargetInfo] = &[
    InstallTargetInfo {
        target: InstallTarget::Codex,
        name: "Codex",
        icon_path: "icons/provider-codex.svg",
    },
    InstallTargetInfo {
        target: InstallTarget::ClaudeCode,
        name: "Claude Code",
        icon_path: "icons/provider-claude.svg",
    },
    InstallTargetInfo {
        target: InstallTarget::Cursor,
        name: "Cursor",
        icon_path: "icons/provider-cursor.svg",
    },
    InstallTargetInfo {
        target: InstallTarget::OpenCode,
        name: "OpenCode",
        icon_path: "icons/provider-opencode.svg",
    },
    InstallTargetInfo {
        target: InstallTarget::Pi,
        name: "Pi",
        icon_path: "icons/provider-pi.svg",
    },
    InstallTargetInfo {
        target: InstallTarget::Proma,
        name: "Proma",
        icon_path: "icons/provider-proma-black.png",
    },
    InstallTargetInfo {
        target: InstallTarget::Omp,
        name: "omp",
        icon_path: "icons/provider-omp.svg",
    },
    InstallTargetInfo {
        target: InstallTarget::Grok,
        name: "Grok",
        icon_path: "icons/provider-grok.svg",
    },
    InstallTargetInfo {
        target: InstallTarget::Antigravity,
        name: "Antigravity",
        icon_path: "icons/provider-antigravity.svg",
    },
    InstallTargetInfo {
        target: InstallTarget::Droid,
        name: "Droid",
        icon_path: "icons/provider-droid.svg",
    },
    InstallTargetInfo {
        target: InstallTarget::Copilot,
        name: "GitHub Copilot",
        icon_path: "icons/provider-copilot.svg",
    },
];

/// Presentation metadata for the Agent badges used throughout the desktop UI.
///
/// An icon can describe an Agent that consumes the shared `.agents/skills`
/// directory without being an install target that Kitter manages directly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgentIconInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub icon_path: &'static str,
    /// Provider-specific project targets this Agent can consume.  A provider
    /// can list more than one because compatibility directories are shared;
    /// for example Cursor also reads `.claude/skills` and `.codex/skills`.
    pub install_targets: &'static [InstallTarget],
    /// Whether this Agent consumes Kitter's shared `.agents/skills` target.
    /// This is independent from `install_targets`: Pi has both a shared target
    /// and its own `.pi/skills` target.
    pub shared_agents: bool,
    pub global_only: bool,
}

impl AgentIconInfo {
    pub fn owns_target(self, target: InstallTarget) -> bool {
        let id = match target {
            InstallTarget::Universal => return false,
            InstallTarget::Codex => "codex",
            InstallTarget::ClaudeCode => "claude-code",
            InstallTarget::Cursor => "cursor",
            InstallTarget::OpenCode => "opencode",
            InstallTarget::Pi => "pi",
            InstallTarget::Proma => "proma",
            InstallTarget::Omp => "omp",
            InstallTarget::Grok => "grok",
            InstallTarget::Antigravity => "antigravity",
            InstallTarget::Droid => "droid",
            InstallTarget::Copilot => "copilot",
        };
        self.id == id
    }

    pub fn supports_target(self, target: InstallTarget) -> bool {
        self.install_targets.contains(&target)
            || (target == InstallTarget::Universal && self.shared_agents)
    }
}

/// Supported Agent metadata. Display order uses `display_sort_key` rather
/// than this catalog's declaration order.
pub const AGENT_ICON_ORDER: &[AgentIconInfo] = &[
    AgentIconInfo {
        id: "codex",
        name: "Codex",
        icon_path: "icons/provider-codex.svg",
        install_targets: &[InstallTarget::Codex],
        shared_agents: true,
        global_only: false,
    },
    AgentIconInfo {
        id: "claude-code",
        name: "Claude Code",
        icon_path: "icons/provider-claude.svg",
        install_targets: &[InstallTarget::ClaudeCode],
        shared_agents: false,
        global_only: false,
    },
    AgentIconInfo {
        id: "cursor",
        name: "Cursor",
        icon_path: "icons/provider-cursor.svg",
        install_targets: &[
            InstallTarget::Cursor,
            InstallTarget::ClaudeCode,
            InstallTarget::Codex,
        ],
        shared_agents: true,
        global_only: false,
    },
    AgentIconInfo {
        id: "opencode",
        name: "OpenCode",
        icon_path: "icons/provider-opencode.svg",
        install_targets: &[InstallTarget::OpenCode, InstallTarget::ClaudeCode],
        shared_agents: true,
        global_only: false,
    },
    AgentIconInfo {
        id: "pi",
        name: "Pi",
        icon_path: "icons/provider-pi.svg",
        install_targets: &[InstallTarget::Pi],
        shared_agents: true,
        global_only: false,
    },
    AgentIconInfo {
        id: "proma",
        name: "Proma",
        icon_path: "icons/provider-proma-black.png",
        install_targets: &[InstallTarget::Proma],
        shared_agents: false,
        global_only: false,
    },
    AgentIconInfo {
        id: "omp",
        name: "omp",
        icon_path: "icons/provider-omp.svg",
        install_targets: &[
            InstallTarget::Omp,
            InstallTarget::ClaudeCode,
            InstallTarget::Codex,
            InstallTarget::OpenCode,
            InstallTarget::Antigravity,
            InstallTarget::Copilot,
        ],
        shared_agents: true,
        global_only: false,
    },
    AgentIconInfo {
        id: "dimagent",
        name: "DimAgent",
        icon_path: "icons/provider-dimagent.svg",
        install_targets: &[],
        shared_agents: true,
        global_only: false,
    },
    AgentIconInfo {
        id: "grok",
        name: "Grok",
        icon_path: "icons/provider-grok.svg",
        install_targets: &[
            InstallTarget::Grok,
            InstallTarget::ClaudeCode,
            InstallTarget::Cursor,
        ],
        shared_agents: true,
        global_only: false,
    },
    AgentIconInfo {
        id: "openclaw",
        name: "OpenClaw",
        icon_path: "icons/provider-openclaw.svg",
        install_targets: &[],
        shared_agents: false,
        global_only: true,
    },
    AgentIconInfo {
        id: "hermes",
        name: "Hermes",
        icon_path: "icons/provider-hermes.svg",
        install_targets: &[],
        shared_agents: false,
        global_only: true,
    },
    AgentIconInfo {
        id: "droid",
        name: "Droid",
        icon_path: "icons/provider-droid.svg",
        install_targets: &[InstallTarget::Droid, InstallTarget::Antigravity],
        shared_agents: true,
        global_only: false,
    },
    AgentIconInfo {
        id: "amp",
        name: "Amp",
        icon_path: "icons/provider-amp.svg",
        install_targets: &[InstallTarget::ClaudeCode],
        shared_agents: true,
        global_only: false,
    },
    AgentIconInfo {
        id: "antigravity",
        name: "Antigravity",
        icon_path: "icons/provider-antigravity.svg",
        install_targets: &[InstallTarget::Antigravity],
        shared_agents: true,
        global_only: false,
    },
    AgentIconInfo {
        id: "copilot",
        name: "GitHub Copilot",
        icon_path: "icons/provider-copilot.svg",
        install_targets: &[InstallTarget::Copilot, InstallTarget::ClaudeCode],
        shared_agents: true,
        global_only: false,
    },
    AgentIconInfo {
        id: "trae",
        name: "Trae",
        icon_path: "icons/provider-trae.svg",
        install_targets: &[],
        shared_agents: true,
        global_only: false,
    },
];

pub fn target_directory(target: InstallTarget) -> &'static str {
    match target {
        InstallTarget::Universal => ".agents/skills",
        InstallTarget::Codex => ".codex/skills",
        InstallTarget::ClaudeCode => ".claude/skills",
        InstallTarget::Cursor => ".cursor/skills",
        InstallTarget::OpenCode => ".opencode/skills",
        InstallTarget::Pi => ".pi/skills",
        InstallTarget::Proma => "skills",
        InstallTarget::Omp => ".omp/skills",
        InstallTarget::Grok => ".grok/skills",
        InstallTarget::Antigravity => ".agent/skills",
        InstallTarget::Droid => ".factory/skills",
        InstallTarget::Copilot => ".github/skills",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_icon_order_matches_usage_priority() {
        let ids = AGENT_ICON_ORDER
            .iter()
            .map(|agent| agent.id)
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            vec![
                "codex",
                "claude-code",
                "cursor",
                "opencode",
                "pi",
                "proma",
                "omp",
                "dimagent",
                "grok",
                "openclaw",
                "hermes",
                "droid",
                "amp",
                "antigravity",
                "copilot",
                "trae",
            ]
        );
    }

    #[test]
    fn openclaw_and_hermes_are_global_only() {
        let global_only = AGENT_ICON_ORDER
            .iter()
            .filter(|agent| agent.global_only)
            .map(|agent| agent.id)
            .collect::<Vec<_>>();
        assert_eq!(global_only, vec!["openclaw", "hermes"]);
        assert!(
            AGENT_ICON_ORDER
                .iter()
                .filter(|agent| agent.global_only)
                .all(|agent| agent.install_targets.is_empty())
        );
    }

    #[test]
    fn shared_agents_also_have_their_provider_specific_targets() {
        for id in [
            "codex",
            "cursor",
            "opencode",
            "pi",
            "grok",
            "droid",
            "antigravity",
            "copilot",
        ] {
            let agent = AGENT_ICON_ORDER
                .iter()
                .find(|agent| agent.id == id)
                .unwrap();
            assert!(agent.shared_agents, "{id} should consume .agents/skills");
            assert!(
                agent
                    .install_targets
                    .iter()
                    .any(|target| *target != InstallTarget::Universal),
                "{id} should have a provider-specific project target"
            );
        }
    }

    #[test]
    fn display_order_keeps_installed_agents_first_and_each_group_alphabetical() {
        let installed = ["proma", "grok", "codex", "antigravity"];
        let mut agents = AGENT_ICON_ORDER.to_vec();
        agents.sort_by_key(|agent| display_sort_key(agent.name, installed.contains(&agent.id)));
        let names = agents.iter().map(|agent| agent.name).collect::<Vec<_>>();
        assert_eq!(&names[..4], ["Antigravity", "Codex", "Grok", "Proma"]);
        assert_eq!(&names[4..], ["Amp", "Claude Code", "Cursor", "DimAgent", "Droid",
            "GitHub Copilot", "Hermes", "omp", "OpenClaw", "OpenCode", "Pi", "Trae"]);
    }

    #[test]
    fn compatibility_does_not_make_a_provider_an_installed_target() {
        let grok = *AGENT_ICON_ORDER.iter().find(|agent| agent.id == "grok").unwrap();
        assert!(grok.supports_target(InstallTarget::ClaudeCode));
        assert!(!grok.owns_target(InstallTarget::ClaudeCode));
        assert!(grok.owns_target(InstallTarget::Grok));
        for target in INDEPENDENT_INSTALL_TARGETS {
            assert_eq!(AGENT_ICON_ORDER.iter().filter(|agent| agent.owns_target(target.target)).count(), 1);
        }
    }

    #[test]
    fn compatibility_badges_cover_shared_provider_specific_roots() {
        let cursor = AGENT_ICON_ORDER
            .iter()
            .find(|agent| agent.id == "cursor")
            .unwrap();
        assert!(cursor.supports_target(InstallTarget::Universal));
        assert!(cursor.supports_target(InstallTarget::Cursor));
        assert!(cursor.supports_target(InstallTarget::ClaudeCode));
        assert!(cursor.supports_target(InstallTarget::Codex));

        let openclaw = AGENT_ICON_ORDER
            .iter()
            .find(|agent| agent.id == "openclaw")
            .unwrap();
        assert!(!openclaw.supports_target(InstallTarget::Universal));
    }
}
