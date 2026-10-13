use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
};

use crate::{
    InstallTarget, ProjectSkill, ProjectSkillInstallation,
    effective_skills::{
        AgentContextEstimate, AgentKind, SkillScope, SkillSource, SkillVisibility, group_effective_skills,
    },
    project,
};

#[derive(Clone)]
pub(super) struct EffectivePluginGroup {
    pub key: String,
    pub agent: AgentKind,
    pub display_name: String,
    pub skills: Vec<String>,
}

pub(super) fn plugin_groups(
    estimates: &[AgentContextEstimate],
    selected_agent: Option<AgentKind>,
) -> Vec<EffectivePluginGroup> {
    let mut groups = BTreeMap::<String, EffectivePluginGroup>::new();
    for estimate in estimates
        .iter()
        .filter(|estimate| selected_agent.is_none_or(|agent| agent == estimate.agent))
    {
        for skill in estimate.plugin_skills() {
            let Some(id) = skill.source.plugin_id() else {
                continue;
            };
            let display_name = skill.source.plugin_display_name().unwrap_or(id);
            let key = format!("{}:{id}", estimate.agent.id());
            let group = groups
                .entry(key.clone())
                .or_insert_with(|| EffectivePluginGroup {
                    key,
                    agent: estimate.agent,
                    display_name: display_name.to_string(),
                    skills: Vec::new(),
                });
            if !group.skills.contains(&skill.name) {
                group.skills.push(skill.name.clone());
            }
        }
    }
    for group in groups.values_mut() {
        group.skills.sort();
    }
    groups.into_values().collect()
}

#[derive(Clone)]
pub(super) struct EffectiveSkillRow {
    pub name: String,
    pub description: String,
    pub locations: Vec<PathBuf>,
    pub built_in: bool,
    pub agents: Vec<AgentKind>,
    pub manual_only: bool,
    pub direct_installations: Vec<ProjectSkillInstallation>,
    pub origins: Vec<SkillOrigin>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SkillOrigin {
    Project,
    Parent,
    User,
    System,
    Builtin,
    Integration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SkillSourceGroup {
    Origin(SkillOrigin),
    Multiple,
}

impl SkillSourceGroup {
    pub const ALL: [Self; 7] = [
        Self::Origin(SkillOrigin::Project), Self::Origin(SkillOrigin::User),
        Self::Origin(SkillOrigin::Parent), Self::Origin(SkillOrigin::System),
        Self::Origin(SkillOrigin::Builtin), Self::Origin(SkillOrigin::Integration), Self::Multiple,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Origin(SkillOrigin::Project) => "project",
            Self::Origin(SkillOrigin::User) => "user",
            Self::Origin(SkillOrigin::Parent) => "parent",
            Self::Origin(SkillOrigin::System) => "system",
            Self::Origin(SkillOrigin::Builtin) => "builtin",
            Self::Origin(SkillOrigin::Integration) => "integration",
            Self::Multiple => "multiple",
        }
    }
}

pub(super) enum ProjectSkillListItem {
    Header { group: SkillSourceGroup, count: usize },
    Skill(usize),
}

pub(super) fn grouped_skill_items(
    rows: &[EffectiveSkillRow], collapsed: &HashSet<String>,
) -> Vec<ProjectSkillListItem> {
    let mut items = Vec::new();
    for group in SkillSourceGroup::ALL {
        let indices = rows.iter().enumerate().filter_map(|(index, row)| {
            (row.source_group() == group).then_some(index)
        }).collect::<Vec<_>>();
        if indices.is_empty() { continue; }
        items.push(ProjectSkillListItem::Header { group, count: indices.len() });
        if !collapsed.contains(group.key()) {
            items.extend(indices.into_iter().map(ProjectSkillListItem::Skill));
        }
    }
    items
}

impl EffectiveSkillRow {
    pub fn source_group(&self) -> SkillSourceGroup {
        match self.origins.as_slice() {
            [origin] => SkillSourceGroup::Origin(*origin),
            _ => SkillSourceGroup::Multiple,
        }
    }

    pub fn can_remove(&self) -> bool {
        self.direct_installations.iter().any(|item| item.removable)
    }

    pub fn removable_installations(&self) -> Vec<ProjectSkillInstallation> {
        self.direct_installations.iter().filter(|item| item.removable).cloned().collect()
    }
}

pub(super) fn same_file(left: &Path, right: &Path) -> bool {
    let left = left.canonicalize().unwrap_or_else(|_| left.to_path_buf());
    let right = right.canonicalize().unwrap_or_else(|_| right.to_path_buf());
    left == right
}

pub(super) fn skill_rows(
    estimates: &[AgentContextEstimate],
    project_skills: &[ProjectSkill],
    selected_agent: Option<AgentKind>,
    project_path: &Path,
) -> Vec<EffectiveSkillRow> {
    let home = dirs::home_dir();
    let global_roots = home.as_ref().map(|home| {
        crate::agents::PROJECT_INSTALL_TARGETS.iter().copied()
            .filter(|target| *target != InstallTarget::Proma)
            .map(|target| crate::agents::global_target_root(home, target))
            .collect::<Vec<_>>()
    }).unwrap_or_default();
    let groups = group_effective_skills(
        estimates
            .iter()
            .filter(|estimate| selected_agent.is_none_or(|agent| agent == estimate.agent))
            .flat_map(|estimate| {
                estimate
                    .skills
                    .iter()
                    .filter(|skill| !skill.is_plugin())
                    .map(|skill| (estimate.agent, skill))
            }),
    );
    let mut rows = Vec::with_capacity(groups.len());
    for group in groups {
        let Some((_, first)) = group.entries().first() else {
            continue;
        };
        let mut row = EffectiveSkillRow {
            name: first.name.clone(),
            description: first.description.clone(),
            locations: Vec::new(),
            built_in: false,
            agents: Vec::new(),
            manual_only: false,
            direct_installations: Vec::new(),
            origins: Vec::new(),
        };
        for (agent, skill) in group.entries() {
            if row.description.is_empty() && !skill.description.is_empty() {
                row.description = skill.description.clone();
            }
            if let Some(root) = &skill.root_path
                && !row.locations.iter().any(|location| location == root)
            {
                row.locations.push(root.clone());
            }
            row.built_in |= matches!(skill.source, SkillSource::Builtin);
            if !row.agents.contains(agent) {
                row.agents.push(*agent);
            }
            row.manual_only |= skill.visibility == SkillVisibility::ManualOnly;

            let mut direct = false;
            let mut bundled = false;
            let mut integrated = false;
            for installation in project_skills
                .iter()
                .filter(|installed| installed.name == skill.name
                    || installed.installations.iter().any(|item| item.target == InstallTarget::Proma))
                .flat_map(|installed| &installed.installations)
            {
                if same_file(&installation.path.join("SKILL.md"), &skill.path) {
                    direct = true;
                    bundled |= installation.bundled;
                    integrated |= installation.integration.is_some();
                    if !row.direct_installations.iter().any(|existing| {
                        project::installation_key(&existing.path)
                            == project::installation_key(&installation.path)
                    }) {
                        row.direct_installations.push(installation.clone());
                    }
                }
            }
            let origin = if bundled || matches!(skill.source, SkillSource::Builtin) {
                SkillOrigin::Builtin
            } else if integrated {
                SkillOrigin::Integration
            } else {
                match skill.scope {
                    SkillScope::User => SkillOrigin::User,
                    SkillScope::System => SkillOrigin::System,
                    SkillScope::Local | SkillScope::Repository => {
                        if direct {
                            if home.as_deref() == Some(project_path) { SkillOrigin::User } else { SkillOrigin::Project }
                        } else if skill.root_path.as_ref().is_some_and(|root| {
                            global_roots.iter().any(|global| same_file(root, global))
                        }) {
                            SkillOrigin::User
                        } else if skill.root_path.as_ref().is_some_and(|root| root.starts_with(project_path)) {
                            SkillOrigin::Project
                        } else {
                            SkillOrigin::Parent
                        }
                    }
                }
            };
            if !row.origins.contains(&origin) {
                row.origins.push(origin);
            }
        }
        rows.push(row);
    }
    // Inactive or invalid Proma packages belong in installation management,
    // but must never be added to the effective catalog or token estimate.
    if selected_agent.is_none_or(|agent| agent == AgentKind::Proma) {
        for installed in project_skills {
            for installation in &installed.installations {
                if installation.target != InstallTarget::Proma
                    || rows.iter().any(|row| row.direct_installations.iter().any(|item| {
                        project::installation_key(&item.path) == project::installation_key(&installation.path)
                    }))
                {
                    continue;
                }
                let row = if let Some(index) = rows.iter().position(|row| row.name == installed.name) {
                    &mut rows[index]
                } else {
                    rows.push(EffectiveSkillRow {
                        name: installed.name.clone(),
                        description: String::new(),
                        locations: Vec::new(),
                        built_in: false,
                        agents: Vec::new(),
                        manual_only: false,
                        direct_installations: Vec::new(),
                        origins: Vec::new(),
                    });
                    rows.last_mut().unwrap()
                };
                if !row.agents.contains(&AgentKind::Proma) {
                    row.agents.push(AgentKind::Proma);
                }
                if let Some(parent) = installation.path.parent()
                    && !row.locations.iter().any(|path| path == parent)
                {
                    row.locations.push(parent.to_path_buf());
                }
                row.direct_installations.push(installation.clone());
                let origin = if installation.bundled { SkillOrigin::Builtin }
                    else if installation.integration.is_some() { SkillOrigin::Integration }
                    else { SkillOrigin::Project };
                if !row.origins.contains(&origin) {
                    row.origins.push(origin);
                }
            }
        }
    }
    let agent_order = |agent: &AgentKind| {
        crate::agents::AGENT_ICON_ORDER
            .iter()
            .position(|item| item.id == agent.id())
            .unwrap_or(usize::MAX)
    };
    for row in &mut rows {
        row.locations.sort_by_key(|path| super::display_path(path));
        row.agents.sort_by_key(agent_order);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effective_skills::EffectiveSkill;

    #[test]
    fn source_groups_count_each_skill_once_and_collapse_only_their_rows() {
        let row = |name: &str, origins| EffectiveSkillRow {
            name: name.into(), description: String::new(), locations: Vec::new(),
            built_in: false, agents: Vec::new(), manual_only: false,
            direct_installations: Vec::new(), origins,
        };
        let rows = [
            row("project", vec![SkillOrigin::Project]),
            row("global-a", vec![SkillOrigin::User]),
            row("global-b", vec![SkillOrigin::User]),
            row("parent", vec![SkillOrigin::Parent]),
            row("system", vec![SkillOrigin::System]),
            row("builtin", vec![SkillOrigin::Builtin]),
            row("integration", vec![SkillOrigin::Integration]),
            row("mixed", vec![SkillOrigin::User, SkillOrigin::Builtin]),
        ];
        let items = grouped_skill_items(&rows, &HashSet::new());
        let headers = items.iter().filter_map(|item| match item {
            ProjectSkillListItem::Header { group, count } => Some((group.key(), *count)),
            _ => None,
        }).collect::<Vec<_>>();
        assert_eq!(headers, [("project", 1), ("user", 2), ("parent", 1), ("system", 1), ("builtin", 1), ("integration", 1), ("multiple", 1)]);
        let indices = items.iter().filter_map(|item| match item {
            ProjectSkillListItem::Skill(index) => Some(*index), _ => None,
        }).collect::<HashSet<_>>();
        assert_eq!(indices.len(), rows.len());
        let collapsed = grouped_skill_items(&rows, &HashSet::from(["builtin".into()]));
        assert_eq!(collapsed.len(), items.len() - 1);
        assert!(!collapsed.iter().any(|item| matches!(item, ProjectSkillListItem::Skill(5))));
        assert!(collapsed.iter().any(|item| matches!(item, ProjectSkillListItem::Header { group: SkillSourceGroup::Origin(SkillOrigin::Builtin), count: 1 })));
    }

    #[test]
    fn proma_presets_are_builtin_without_becoming_removable() {
        let direct = [ProjectSkill { name: "preset".into(), installations: vec![ProjectSkillInstallation {
            target: InstallTarget::Proma, path: "/native/skills-inactive/preset".into(),
            managed: false, removable: false, bundled: true, integration: None,
            state: Some("inactive".into()), status: Some("unmanaged".into()),
        }] }];
        let rows = skill_rows(&[], &direct, None, Path::new("/business"));
        assert_eq!(rows[0].origins, [SkillOrigin::Builtin]);
        assert!(!rows[0].can_remove());
        assert_eq!(rows[0].direct_installations[0].state.as_deref(), Some("inactive"));
        let mut integrated = direct[0].clone();
        integrated.installations[0].bundled = false;
        integrated.installations[0].integration = Some("Nowledge Mem".into());
        let rows = skill_rows(&[], &[integrated], None, Path::new("/business"));
        assert_eq!(rows[0].origins, [SkillOrigin::Integration]);
        assert_eq!(rows[0].direct_installations[0].integration.as_deref(), Some("Nowledge Mem"));
        assert!(!rows[0].can_remove());
    }

    #[test]
    fn origin_labels_preserve_mixed_global_and_native_project_sources() {
        let skill = |name: &str, root: &str, scope, source| EffectiveSkill {
            id: name.into(), name: name.into(), description: String::new(), when_to_use: None,
            path: PathBuf::from(root).join(name).join("SKILL.md"),
            root_path: Some(root.into()), prompt_path: None, scope,
            visibility: SkillVisibility::Automatic, source,
        };
        let estimates = [AgentContextEstimate {
            agent: AgentKind::Codex, discovered_count: 4, model_visible_count: 4,
            manual_only_count: 0, name_only_count: 0, conditional_count: 0, estimated_tokens: 0,
            skills: vec![
                skill("mixed", "/home/.codex/skills", SkillScope::User, SkillSource::Filesystem),
                skill("parent", "/repo/.agents/skills", SkillScope::Repository, SkillSource::Filesystem),
                skill("local", "/repo/project/.agents/skills", SkillScope::Repository, SkillSource::Filesystem),
                skill("builtin", "/system", SkillScope::System, SkillSource::Builtin),
            ],
        }];
        let direct = [ProjectSkill {
            name: "mixed".into(), installations: vec![ProjectSkillInstallation {
                target: InstallTarget::Proma, path: "/native/skills/mixed".into(),
                managed: false, removable: false, bundled: false, integration: None,
                state: Some("active".into()), status: Some("unmanaged".into()),
            }],
        }];
        let rows = skill_rows(&estimates, &direct, None, Path::new("/repo/project"));
        let row = |name| rows.iter().find(|row| row.name == name).unwrap();
        assert_eq!(row("mixed").origins, [SkillOrigin::User, SkillOrigin::Project]);
        assert!(!row("mixed").can_remove());
        assert_eq!(row("parent").origins, [SkillOrigin::Parent]);
        assert_eq!(row("local").origins, [SkillOrigin::Project]);
        assert_eq!(row("builtin").origins, [SkillOrigin::Builtin]);
    }

    #[test]
    fn configured_user_roots_are_global_even_with_repository_scope() {
        let home = dirs::home_dir().unwrap();
        let root = crate::agents::global_target_root(&home, InstallTarget::Codex);
        let estimates = [AgentContextEstimate {
            agent: AgentKind::Cursor, discovered_count: 1, model_visible_count: 1,
            manual_only_count: 0, name_only_count: 0, conditional_count: 0, estimated_tokens: 0,
            skills: vec![EffectiveSkill {
                id: "configured".into(), name: "configured".into(), description: String::new(),
                when_to_use: None, path: root.join("configured/SKILL.md"),
                root_path: Some(root), prompt_path: None, scope: SkillScope::Repository,
                visibility: SkillVisibility::Automatic, source: SkillSource::Filesystem,
            }],
        }];
        let rows = skill_rows(&estimates, &[], None, Path::new("/repo/project"));
        assert_eq!(rows[0].origins, [SkillOrigin::User]);
        assert!(!rows[0].can_remove());
    }

    #[test]
    fn proma_inactive_installations_are_visible_with_distinct_state() {
        let installations = ["active", "inactive"].into_iter().map(|state| ProjectSkillInstallation {
            target: InstallTarget::Proma,
            path: PathBuf::from(if state == "active" { "/pilot/skills/alpha" } else { "/pilot/skills-inactive/alpha" }),
            managed: true,
            removable: true,
            bundled: false,
            integration: None,
            state: Some(state.into()),
            status: Some("current".into()),
        }).collect();
        let direct = [ProjectSkill { name: "alpha".into(), installations }];
        let rows = skill_rows(&[], &direct, Some(AgentKind::Proma), Path::new("/pilot"));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].agents, [AgentKind::Proma]);
        assert_eq!(rows[0].direct_installations.len(), 2);
        assert_eq!(rows[0].direct_installations[0].state.as_deref(), Some("active"));
        assert_eq!(rows[0].direct_installations[1].state.as_deref(), Some("inactive"));
        assert!(skill_rows(&[], &direct, Some(AgentKind::Codex), Path::new("/pilot")).is_empty());
    }
}
