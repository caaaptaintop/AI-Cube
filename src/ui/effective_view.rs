use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

use crate::{
    InstallTarget, ProjectSkill, ProjectSkillInstallation,
    effective_skills::{
        AgentContextEstimate, AgentKind, SkillScope, SkillSource, SkillVisibility, group_effective_skills,
    },
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
    pub key: String,
    pub name: String,
    pub duplicate_name: bool,
    pub description: String,
    pub locations: Vec<PathBuf>,
    pub built_in: bool,
    pub agents: Vec<AgentKind>,
    pub manual_only: bool,
    pub direct_installations: Vec<ProjectSkillInstallation>,
    pub origins: Vec<SkillOrigin>,
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct SkillRowsCacheKey {
    pub project: PathBuf,
    pub generation: u64,
    pub agent: Option<AgentKind>,
    pub scanned_at: Instant,
}

#[derive(Default)]
pub(super) struct SkillRowsCache {
    cached: Option<(SkillRowsCacheKey, Arc<Vec<EffectiveSkillRow>>)>,
}

impl SkillRowsCache {
    pub fn get_or_build(
        &mut self, key: SkillRowsCacheKey, estimates: &[AgentContextEstimate],
        project_skills: &[ProjectSkill],
    ) -> Arc<Vec<EffectiveSkillRow>> {
        if let Some((previous, rows)) = &self.cached
            && previous == &key
        {
            return Arc::clone(rows);
        }
        let rows = Arc::new(skill_rows(estimates, project_skills, key.agent, &key.project));
        self.cached = Some((key, Arc::clone(&rows)));
        rows
    }
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

#[derive(Clone, PartialEq, Eq)]
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

    pub fn batch_installations(&self, action: super::ProjectBatchAction) -> Vec<ProjectSkillInstallation> {
        match action {
            super::ProjectBatchAction::Remove => self.removable_installations(),
            super::ProjectBatchAction::DisableProma | super::ProjectBatchAction::EnableProma => {
                let state = if action == super::ProjectBatchAction::EnableProma { "inactive" } else { "active" };
                self.direct_installations.iter().filter(|item| {
                    item.target == InstallTarget::Proma
                        && item.state.as_deref() == Some(state)
                        && item.toggleable
                }).cloned().collect()
            }
        }
    }
}

pub(super) fn same_file(left: &Path, right: &Path) -> bool {
    let left = left.canonicalize().unwrap_or_else(|_| left.to_path_buf());
    let right = right.canonicalize().unwrap_or_else(|_| right.to_path_buf());
    left == right
}

pub(super) fn skill_file_key(path: &Path) -> String {
    format!("file:{}", path.canonicalize().unwrap_or_else(|_| path.to_path_buf()).display())
}

#[cfg(test)]
thread_local! {
    static PATH_RESOLUTIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(super) fn reset_path_resolution_count() {
    PATH_RESOLUTIONS.with(|count| count.set(0));
}

#[cfg(test)]
pub(super) fn path_resolution_count() -> usize {
    PATH_RESOLUTIONS.with(|count| count.get())
}

fn cached_path(path: &Path, paths: &mut BTreeMap<PathBuf, PathBuf>) -> PathBuf {
    paths.entry(path.to_path_buf()).or_insert_with(|| {
        #[cfg(test)]
        PATH_RESOLUTIONS.with(|count| count.set(count.get() + 1));
        path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
    }).clone()
}

struct IndexedInstallation<'a> {
    name: &'a str,
    has_proma: bool,
    installation: &'a ProjectSkillInstallation,
    key: PathBuf,
}

pub(super) fn skill_rows(
    estimates: &[AgentContextEstimate],
    project_skills: &[ProjectSkill],
    selected_agent: Option<AgentKind>,
    project_path: &Path,
) -> Vec<EffectiveSkillRow> {
    let mut paths = BTreeMap::new();
    let home = dirs::home_dir();
    let global_roots = home.as_ref().map(|home| {
        crate::agents::PROJECT_INSTALL_TARGETS.iter().copied()
            .filter(|target| *target != InstallTarget::Proma)
            .map(|target| cached_path(&crate::agents::global_target_root(home, target), &mut paths))
            .collect::<HashSet<_>>()
    }).unwrap_or_default();
    let mut installations = Vec::new();
    let mut by_skill_path = BTreeMap::<PathBuf, Vec<usize>>::new();
    for installed in project_skills {
        let has_proma = installed.installations.iter().any(|item| item.target == InstallTarget::Proma);
        for installation in &installed.installations {
            if selected_agent.is_some_and(|agent| {
                !crate::agents::AGENT_ICON_ORDER.iter().any(|info| info.id == agent.id()
                    && info.supports_target(installation.target))
            }) { continue; }
            let skill_path = cached_path(&installation.path.join("SKILL.md"), &mut paths);
            // Resolve the parent, not the skill link itself: distinct agent
            // entries pointing to one source must retain their own actions.
            let parent = cached_path(installation.path.parent().unwrap_or(Path::new("")), &mut paths);
            let key = installation.path.file_name().map(|name| parent.join(name)).unwrap_or(parent);
            by_skill_path.entry(skill_path).or_default().push(installations.len());
            installations.push(IndexedInstallation { name: &installed.name, has_proma, installation, key });
        }
    }
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
    let mut matched_installations = HashSet::new();
    for group in groups {
        let Some((first_agent, first)) = group.entries().first() else {
            continue;
        };
        let mut row = EffectiveSkillRow {
            key: if matches!(first.source, SkillSource::Filesystem) {
                format!("file:{}", cached_path(&first.path, &mut paths).display())
            } else {
                format!("builtin:{}:{}:{}", first_agent.id(), first.id, first.path.display())
            },
            name: first.name.clone(),
            duplicate_name: false,
            description: first.description.clone(),
            locations: Vec::new(),
            built_in: false,
            agents: Vec::new(),
            manual_only: false,
            direct_installations: Vec::new(),
            origins: Vec::new(),
        };
        let mut row_installations = HashSet::new();
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
            let skill_path = cached_path(&skill.path, &mut paths);
            for index in by_skill_path.get(&skill_path).into_iter().flatten() {
                let indexed = &installations[*index];
                let installation = indexed.installation;
                if crate::agents::AGENT_ICON_ORDER.iter().any(|info| info.id == agent.id()
                    && info.supports_target(installation.target))
                    && (indexed.name == skill.name || indexed.has_proma)
                {
                    direct = true;
                    bundled |= installation.bundled;
                    integrated |= installation.integration.is_some();
                    if row_installations.insert(indexed.key.clone()) {
                        row.direct_installations.push(installation.clone());
                        matched_installations.insert(indexed.key.clone());
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
                            global_roots.contains(&cached_path(root, &mut paths))
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
        for indexed in &installations {
            let installation = indexed.installation;
            if installation.target != InstallTarget::Proma
                || !matched_installations.insert(indexed.key.clone())
            {
                continue;
            }
            let key = format!("file:{}", cached_path(&installation.path.join("SKILL.md"), &mut paths).display());
            let row = if let Some(index) = rows.iter().position(|row| row.key == key) {
                &mut rows[index]
            } else {
                rows.push(EffectiveSkillRow {
                    key,
                    name: indexed.name.to_string(),
                    duplicate_name: false,
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
    let mut names = BTreeMap::<String, usize>::new();
    for row in &rows { *names.entry(row.name.clone()).or_default() += 1; }
    for row in &mut rows {
        row.duplicate_name = names[&row.name] > 1;
        row.locations.sort_by_key(|path| super::display_path(path));
        row.agents.sort_by_key(|agent| crate::agents::display_sort_key(agent.label(), true));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effective_skills::EffectiveSkill;

    #[test]
    fn same_named_active_and_inactive_proma_copies_do_not_attach_to_codex() {
        let skill = EffectiveSkill {
            id: "pdf".into(), name: "pdf".into(), description: "Codex version".into(), when_to_use: None,
            path: "/global/pdf/SKILL.md".into(), root_path: Some("/global".into()), prompt_path: None,
            scope: SkillScope::User, visibility: SkillVisibility::Automatic, source: SkillSource::Filesystem,
        };
        let estimates = [AgentContextEstimate {
            agent: AgentKind::Codex, discovered_count: 1, model_visible_count: 1,
            manual_only_count: 0, name_only_count: 0, conditional_count: 0, estimated_tokens: 1, skills: vec![skill],
        }];
        for state in ["active", "inactive"] {
            let direct = [ProjectSkill { name: "pdf".into(), installations: vec![ProjectSkillInstallation {
                target: InstallTarget::Proma, path: format!("/native/{state}/pdf").into(),
                managed: false, removable: false, toggleable: true, bundled: true, integration: None,
                state: Some(state.into()), status: Some("unmanaged".into()),
            }] }];
            let rows = skill_rows(&estimates, &direct, None, Path::new("/business"));
            assert_eq!(rows.len(), 2, "independent copies must remain distinct even when names match");
            assert_ne!(rows[0].key, rows[1].key);
            assert!(rows.iter().all(|row| row.duplicate_name));
            let codex = rows.iter().find(|row| row.agents == [AgentKind::Codex]).unwrap();
            assert!(codex.direct_installations.is_empty());
            assert_eq!(codex.origins, [SkillOrigin::User]);
            let proma = rows.iter().find(|row| row.agents == [AgentKind::Proma]).unwrap();
            assert_eq!(proma.direct_installations.len(), 1);
            assert_eq!(proma.direct_installations[0].state.as_deref(), Some(state));
        }
    }

    #[test]
    fn plugin_names_and_ids_stay_with_the_selected_agent() {
        let estimate = |agent| AgentContextEstimate {
            agent, discovered_count: 1, model_visible_count: 1, manual_only_count: 0,
            name_only_count: 0, conditional_count: 0, estimated_tokens: 1,
            skills: vec![EffectiveSkill {
                id: "pdf".into(), name: "pdf".into(), description: String::new(), when_to_use: None,
                path: "/virtual/plugin/pdf".into(), root_path: None, prompt_path: None,
                scope: SkillScope::User, visibility: SkillVisibility::Automatic,
                source: SkillSource::Plugin { id: "suite".into(), display_name: "Suite".into() },
            }],
        };
        let estimates = [estimate(AgentKind::Codex), estimate(AgentKind::Proma)];
        let all = plugin_groups(&estimates, None);
        assert_eq!(all.len(), 2);
        assert_ne!(all[0].key, all[1].key);
        let codex = plugin_groups(&estimates, Some(AgentKind::Codex));
        assert_eq!(codex.len(), 1);
        assert_eq!(codex[0].agent, AgentKind::Codex);
        assert!(skill_rows(&estimates, &[], None, Path::new("/project")).is_empty());
    }

    #[test]
    fn large_proma_catalog_resolves_paths_linearly_and_keeps_agent_entries() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let mut skills = Vec::new();
        let mut direct = Vec::new();
        for index in 0..160 {
            let name = format!("skill-{index}");
            let folder = root.join("skills").join(&name);
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(folder.join("SKILL.md"), "fixture").unwrap();
            skills.push(EffectiveSkill {
                id: name.clone(), name: name.clone(), description: String::new(), when_to_use: None,
                path: folder.join("SKILL.md"), root_path: Some(root.join("skills")), prompt_path: None,
                scope: SkillScope::Repository, visibility: SkillVisibility::Automatic, source: SkillSource::Filesystem,
            });
            direct.push(ProjectSkill { name, installations: vec![ProjectSkillInstallation {
                target: InstallTarget::Proma, path: folder, managed: false, removable: false, toggleable: false,
                bundled: false, integration: None, state: Some("active".into()), status: Some("unmanaged".into()),
            }] });
        }
        let estimates = [AgentContextEstimate {
            agent: AgentKind::Proma, discovered_count: skills.len(), model_visible_count: skills.len(),
            manual_only_count: 0, name_only_count: 0, conditional_count: 0, estimated_tokens: 0, skills,
        }];
        PATH_RESOLUTIONS.with(|count| count.set(0));
        let rows = skill_rows(&estimates, &direct, None, root);
        let resolutions = PATH_RESOLUTIONS.with(|count| count.get());
        assert_eq!(rows.len(), 160);
        assert!(rows.iter().all(|row| row.direct_installations.len() == 1 && row.origins == [SkillOrigin::Project]));
        assert!(resolutions < 4 * direct.len(), "path resolutions grew beyond linear: {resolutions}");
    }

    #[cfg(unix)]
    #[test]
    fn indexed_matching_preserves_alias_names_and_distinct_installation_entries() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let source = root.join("source");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("SKILL.md"), "fixture").unwrap();
        let codex = root.join(".codex/skills/alias");
        let proma = root.join("native/skills/alias");
        let pi = root.join(".pi/skills/alias");
        for path in [&codex, &proma, &pi] {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::os::unix::fs::symlink(&source, path).unwrap();
        }
        let effective = |path| EffectiveSkill {
            id: "actual-name".into(), name: "actual-name".into(), description: String::new(), when_to_use: None,
            path, root_path: None, prompt_path: None, scope: SkillScope::Local,
            visibility: SkillVisibility::Automatic, source: SkillSource::Filesystem,
        };
        let estimates = [AgentContextEstimate {
            agent: AgentKind::Codex, discovered_count: 1, model_visible_count: 1,
            manual_only_count: 0, name_only_count: 0, conditional_count: 0, estimated_tokens: 0,
            skills: vec![effective(codex.join("SKILL.md"))],
        }];
        let installation = |target, path| ProjectSkillInstallation {
            target, path, managed: false, removable: false, toggleable: false, bundled: false, integration: None,
            state: Some("active".into()), status: Some("external_link".into()),
        };
        let direct = [ProjectSkill { name: "alias".into(), installations: vec![
            installation(InstallTarget::Codex, codex.clone()), installation(InstallTarget::Proma, proma.clone()),
        ] }];
        let rows = skill_rows(&estimates, &direct, None, root);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "actual-name");
        assert_eq!(rows[0].direct_installations.iter().map(|item| &item.path).collect::<Vec<_>>(), [&codex, &proma]);
        let filtered = skill_rows(&estimates, &direct, Some(AgentKind::Codex), root);
        assert_eq!(filtered[0].direct_installations.iter().map(|item| &item.path).collect::<Vec<_>>(), [&codex]);
        let unrelated = [ProjectSkill { name: "actual-name".into(), installations: vec![installation(InstallTarget::Pi, pi)] }];
        let rows = skill_rows(&estimates, &unrelated, None, root);
        assert!(rows[0].direct_installations.is_empty(), "another Agent's link must not become a Codex action");
        assert_eq!(rows[0].agents, [AgentKind::Codex]);
    }

    #[test]
    fn row_cache_reuses_rows_and_invalidates_for_scans_refresh_project_and_agent() {
        let key = SkillRowsCacheKey { project: "/project".into(), generation: 1, agent: None, scanned_at: Instant::now() };
        let direct = [ProjectSkill { name: "preset".into(), installations: vec![ProjectSkillInstallation {
            target: InstallTarget::Proma, path: "/native/skills/preset".into(), managed: false, removable: false, toggleable: false,
            bundled: true, integration: None, state: Some("active".into()), status: Some("unmanaged".into()),
        }] }];
        let mut cache = SkillRowsCache::default();
        let rows = cache.get_or_build(key.clone(), &[], &direct);
        assert_eq!(rows.len(), 1);
        assert!(Arc::ptr_eq(&rows, &cache.get_or_build(key.clone(), &[], &direct)));
        for changed in [
            SkillRowsCacheKey { generation: 2, ..key.clone() },
            SkillRowsCacheKey { scanned_at: key.scanned_at + std::time::Duration::from_nanos(1), ..key.clone() },
            SkillRowsCacheKey { project: "/other".into(), ..key.clone() },
            SkillRowsCacheKey { agent: Some(AgentKind::Codex), ..key.clone() },
        ] {
            let old = cache.get_or_build(key.clone(), &[], &direct);
            let new = cache.get_or_build(changed.clone(), &[], &[]);
            assert!(!Arc::ptr_eq(&old, &new));
            assert!(new.is_empty());
            assert!(Arc::ptr_eq(&new, &cache.get_or_build(changed, &[], &[])));
        }
    }

    #[test]
    fn source_groups_count_each_skill_once_and_collapse_only_their_rows() {
        let row = |name: &str, origins| EffectiveSkillRow {
            key: name.into(), name: name.into(), duplicate_name: false, description: String::new(), locations: Vec::new(),
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
            managed: false, removable: false, toggleable: false, bundled: true, integration: None,
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
                managed: false, removable: false, toggleable: false, bundled: false, integration: None,
                state: Some("active".into()), status: Some("unmanaged".into()),
            }],
        }];
        let rows = skill_rows(&estimates, &direct, None, Path::new("/repo/project"));
        let row = |name| rows.iter().find(|row| row.name == name).unwrap();
        let mixed = rows.iter().filter(|row| row.name == "mixed").collect::<Vec<_>>();
        assert_eq!(mixed.len(), 2);
        assert!(mixed.iter().any(|row| row.origins == [SkillOrigin::User]));
        assert!(mixed.iter().any(|row| row.origins == [SkillOrigin::Project]));
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
            removable: true, toggleable: false,
            bundled: false,
            integration: None,
            state: Some(state.into()),
            status: Some("current".into()),
        }).collect();
        let direct = [ProjectSkill { name: "alpha".into(), installations }];
        let rows = skill_rows(&[], &direct, Some(AgentKind::Proma), Path::new("/pilot"));
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|row| row.agents == [AgentKind::Proma] && row.duplicate_name));
        assert_ne!(rows[0].key, rows[1].key);
        assert_eq!(rows[0].direct_installations[0].state.as_deref(), Some("active"));
        assert_eq!(rows[1].direct_installations[0].state.as_deref(), Some("inactive"));
        assert!(skill_rows(&[], &direct, Some(AgentKind::Codex), Path::new("/pilot")).is_empty());
    }
}
