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

pub(super) fn skill_source_groups(rows: &[EffectiveSkillRow]) -> Vec<(SkillSourceGroup, usize)> {
    SkillSourceGroup::ALL.into_iter().filter_map(|group| {
        let count = rows.iter().filter(|row| row.source_group() == group).count();
        (count > 0).then_some((group, count))
    }).collect()
}

impl EffectiveSkillRow {
    pub fn proma_is_inactive(&self) -> bool {
        proma_copies_inactive(&self.direct_installations)
    }

    pub fn is_inactive(&self) -> bool {
        self.agents == [AgentKind::Proma] && self.proma_is_inactive()
    }

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

pub(super) fn proma_copies_inactive(installations: &[ProjectSkillInstallation]) -> bool {
    let mut copies = installations.iter().filter(|copy| copy.target == InstallTarget::Proma).peekable();
    copies.peek().is_some() && copies.all(|copy| copy.state.as_deref() == Some("inactive"))
}

pub(super) fn same_file(left: &Path, right: &Path) -> bool {
    let left = left.canonicalize().unwrap_or_else(|_| left.to_path_buf());
    let right = right.canonicalize().unwrap_or_else(|_| right.to_path_buf());
    left == right
}

pub(super) fn skill_file_key(path: &Path) -> String {
    format!("file:{}", path.canonicalize().unwrap_or_else(|_| path.to_path_buf()).display())
}

fn installation_source_file(installation: &ProjectSkillInstallation) -> PathBuf {
    if installation.target == InstallTarget::Proma && installation.managed
        && let Some(source) = &installation.source
    {
        return source.join("SKILL.md");
    }
    installation.path.join("SKILL.md")
}

pub(super) fn installation_skill_key(installation: &ProjectSkillInstallation) -> String {
    skill_file_key(&installation_source_file(installation))
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

fn cached_installation_key(path: &Path, paths: &mut BTreeMap<PathBuf, PathBuf>) -> PathBuf {
    let parent = cached_path(path.parent().unwrap_or(Path::new("")), paths);
    path.file_name().map(|name| parent.join(name)).unwrap_or(parent)
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
    let mut copy_sources = BTreeMap::<PathBuf, PathBuf>::new();
    for installed in project_skills {
        let has_proma = installed.installations.iter().any(|item| item.target == InstallTarget::Proma);
        for installation in &installed.installations {
            if selected_agent.is_some_and(|agent| {
                !crate::agents::AGENT_ICON_ORDER.iter().any(|info| info.id == agent.id()
                    && info.supports_target(installation.target))
            }) { continue; }
            let skill_path = cached_path(&installation.path.join("SKILL.md"), &mut paths);
            // Keep the installation entry: a modified SKILL.md may link to a
            // different source and must not rewrite that source's identity.
            let key = cached_installation_key(&installation.path, &mut paths);
            if installation.target == InstallTarget::Proma && installation.managed
                && installation.source.is_some()
            {
                copy_sources.insert(key.clone(), cached_path(&installation_source_file(installation), &mut paths));
            }
            by_skill_path.entry(skill_path).or_default().push(installations.len());
            installations.push(IndexedInstallation { name: &installed.name, has_proma, installation, key });
        }
    }
    let mut groups = Vec::new();
    let mut ordinary = Vec::new();
    for (agent, skill) in estimates
            .iter()
            .filter(|estimate| selected_agent.is_none_or(|agent| agent == estimate.agent))
            .flat_map(|estimate| {
                estimate
                    .skills
                    .iter()
                    .filter(|skill| !skill.is_plugin())
                    .map(|skill| (estimate.agent, skill))
            })
    {
        let entry = cached_installation_key(skill.path.parent().unwrap_or(Path::new("")), &mut paths);
        if agent == AgentKind::Proma && matches!(skill.source, SkillSource::Filesystem)
            && copy_sources.contains_key(&entry)
        {
            groups.push(vec![(agent, skill)]);
        } else {
            ordinary.push((agent, skill));
        }
    }
    groups.extend(group_effective_skills(ordinary).into_iter().map(|group| group.entries().to_vec()));
    groups.sort_by(|left, right| left[0].1.name.cmp(&right[0].1.name));
    let mut rows = Vec::with_capacity(groups.len());
    let mut matched_installations = HashSet::new();
    for group in groups {
        let Some((first_agent, first)) = group.first() else {
            continue;
        };
        let mut row = EffectiveSkillRow {
            key: if matches!(first.source, SkillSource::Filesystem) {
                let path = cached_path(&first.path, &mut paths);
                let entry = cached_installation_key(first.path.parent().unwrap_or(Path::new("")), &mut paths);
                let source = (*first_agent == AgentKind::Proma).then(|| copy_sources.get(&entry)).flatten();
                format!("file:{}", source.unwrap_or(&path).display())
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
        for (agent, skill) in &group {
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
            let entry = cached_installation_key(skill.path.parent().unwrap_or(Path::new("")), &mut paths);
            for index in by_skill_path.get(&skill_path).into_iter().flatten() {
                let indexed = &installations[*index];
                let installation = indexed.installation;
                if crate::agents::AGENT_ICON_ORDER.iter().any(|info| info.id == agent.id()
                    && info.supports_target(installation.target))
                    && (indexed.name == skill.name || indexed.has_proma)
                    && (*agent != AgentKind::Proma
                        || (!copy_sources.contains_key(&entry) && !copy_sources.contains_key(&indexed.key))
                        || indexed.key == entry)
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
            let key = format!("file:{}", cached_path(&installation_source_file(installation), &mut paths).display());
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
    // Physical Proma packages retain their own operations and state, while a
    // verified managed origin joins them to the skill they were installed from.
    let mut merged = Vec::<EffectiveSkillRow>::new();
    let mut row_indices = BTreeMap::<String, usize>::new();
    for row in rows {
        if let Some(&index) = row_indices.get(&row.key) {
            let existing = &mut merged[index];
            if existing.description.is_empty() { existing.description = row.description; }
            existing.built_in |= row.built_in;
            existing.manual_only |= row.manual_only;
            existing.locations.extend(row.locations);
            existing.agents.extend(row.agents);
            for origin in row.origins {
                if !existing.origins.contains(&origin) { existing.origins.push(origin); }
            }
            for installation in row.direct_installations {
                if !existing.direct_installations.iter().any(|item| item.target == installation.target && item.path == installation.path) {
                    existing.direct_installations.push(installation);
                }
            }
        } else {
            row_indices.insert(row.key.clone(), merged.len());
            merged.push(row);
        }
    }
    let mut rows = merged;
    let mut names = BTreeMap::<String, usize>::new();
    for row in &rows { *names.entry(row.name.clone()).or_default() += 1; }
    for row in &mut rows {
        row.duplicate_name = names[&row.name] > 1;
        row.locations.sort_by_key(|path| super::display_path(path));
        row.locations.dedup();
        row.agents.sort_by_key(|agent| crate::agents::display_sort_key(agent.label(), true));
        row.agents.dedup();
    }
    rows.sort_by_key(|row| (row.is_inactive(), row.name.to_lowercase(), row.key.clone()));
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effective_skills::EffectiveSkill;

    #[test]
    fn managed_copies_share_the_original_skill_row_and_keep_agent_actions() {
        use super::super::ProjectBatchAction;
        let temp = tempfile::tempdir().unwrap();
        for name in ["humanizer-zh", "tabbit", "another-skill"] {
            let source = temp.path().join("library").join(name);
            std::fs::create_dir_all(&source).unwrap();
            std::fs::write(source.join("SKILL.md"), "Original skill").unwrap();
            for state in ["active", "inactive"] {
                let copy = temp.path().join(state).join(name);
                std::fs::create_dir_all(&copy).unwrap();
                std::fs::write(copy.join("SKILL.md"), "Skill with Proma adaptation").unwrap();
                for status in ["current", "update_pending", "modified"] {
                    let estimate = |agent, path: &Path| AgentContextEstimate {
                        agent, discovered_count: 1, model_visible_count: 1, manual_only_count: 0,
                        name_only_count: 0, conditional_count: 0, estimated_tokens: 1,
                        skills: vec![EffectiveSkill {
                            id: name.into(), name: name.into(), description: name.into(), when_to_use: None,
                            path: path.join("SKILL.md"), root_path: path.parent().map(Path::to_path_buf),
                            prompt_path: None, scope: SkillScope::Repository,
                            visibility: SkillVisibility::Automatic, source: SkillSource::Filesystem,
                        }],
                    };
                    let mut estimates = vec![estimate(AgentKind::Codex, &source)];
                    if state == "active" { estimates.push(estimate(AgentKind::Proma, &copy)); }
                    let original = ProjectSkillInstallation {
                        target: InstallTarget::Codex, path: source.clone(), managed: true, source: None,
                        removable: true, toggleable: false, bundled: false, integration: None,
                        state: None, status: None,
                    };
                    let mirror = ProjectSkillInstallation {
                        target: InstallTarget::Proma, path: copy.clone(), managed: true, source: Some(source.clone()),
                        removable: status != "modified", toggleable: status != "modified", bundled: false, integration: None,
                        state: Some(state.into()), status: Some(status.into()),
                    };
                    let direct = [ProjectSkill { name: name.into(), installations: vec![original, mirror.clone()] }];
                    let rows = skill_rows(&estimates, &direct, None, temp.path());
                    assert_eq!(rows.len(), 1, "{name}: {state}/{status} is one skill with two installations");
                    let row = &rows[0];
                    assert_eq!(row.agents, [AgentKind::Codex, AgentKind::Proma]);
                    assert_eq!(row.direct_installations.len(), 2);
                    assert!(!row.duplicate_name);
                    assert_eq!(row.key, installation_skill_key(&mirror));
                    let action = if state == "active" { ProjectBatchAction::DisableProma } else { ProjectBatchAction::EnableProma };
                    let targets = row.batch_installations(action);
                    assert_eq!(targets.len(), usize::from(status != "modified"));
                    if let Some(target) = targets.first() {
                        assert_eq!(target.target, InstallTarget::Proma);
                        assert_eq!(target.path, copy, "toggling must target the physical copy, never its origin");
                    }
                    for agent in [AgentKind::Codex, AgentKind::Proma] {
                        let filtered = skill_rows(&estimates, &direct, Some(agent), temp.path());
                        assert_eq!(filtered.len(), 1);
                        assert_eq!(filtered[0].agents, [agent]);
                        assert_eq!(filtered[0].direct_installations.len(), 1);
                        assert_eq!(filtered[0].key, row.key);
                    }
                }
            }
        }
    }

    #[test]
    fn copy_identity_requires_a_verified_matching_origin() {
        let skill = EffectiveSkill {
            id: "example".into(), name: "example".into(), description: String::new(), when_to_use: None,
            path: "/library/example/SKILL.md".into(), root_path: Some("/library".into()), prompt_path: None,
            scope: SkillScope::User, visibility: SkillVisibility::Automatic, source: SkillSource::Filesystem,
        };
        let estimates = [AgentContextEstimate {
            agent: AgentKind::Codex, discovered_count: 1, model_visible_count: 1, manual_only_count: 0,
            name_only_count: 0, conditional_count: 0, estimated_tokens: 1, skills: vec![skill],
        }];
        for (managed, source) in [(false, Some("/library/example")), (true, None), (true, Some("/other/example"))] {
            let direct = [ProjectSkill { name: "example".into(), installations: vec![ProjectSkillInstallation {
                target: InstallTarget::Proma, path: "/native/skills/example".into(), managed,
                source: source.map(PathBuf::from), removable: false, toggleable: false, bundled: false,
                integration: None, state: Some("inactive".into()), status: Some("modified".into()),
            }] }];
            let rows = skill_rows(&estimates, &direct, None, Path::new("/project"));
            assert_eq!(rows.len(), 2, "names or unverified source claims cannot establish identity");
            assert!(rows.iter().all(|row| row.duplicate_name));
            assert!(rows.iter().find(|row| row.agents == [AgentKind::Codex]).unwrap().direct_installations.is_empty());
        }
    }

    #[cfg(unix)]
    #[test]
    fn modified_copy_entry_links_cannot_merge_other_sources_or_their_actions() {
        use super::super::ProjectBatchAction;
        let temp = tempfile::tempdir().unwrap();
        let a = temp.path().join("source-a");
        let b = temp.path().join("source-b");
        for path in [&a, &b] {
            std::fs::create_dir_all(path).unwrap();
            std::fs::write(path.join("SKILL.md"), path.display().to_string()).unwrap();
        }
        for state in ["active", "inactive"] {
            let copy = temp.path().join(state).join("example");
            std::fs::create_dir_all(&copy).unwrap();
            std::os::unix::fs::symlink(b.join("SKILL.md"), copy.join("SKILL.md")).unwrap();
            let skill = |path: &Path| EffectiveSkill {
                id: "example".into(), name: "example".into(), description: String::new(), when_to_use: None,
                path: path.join("SKILL.md"), root_path: path.parent().map(Path::to_path_buf), prompt_path: None,
                scope: SkillScope::Repository, visibility: SkillVisibility::Automatic, source: SkillSource::Filesystem,
            };
            let estimate = |agent, skills: Vec<EffectiveSkill>| AgentContextEstimate {
                agent, discovered_count: skills.len(), model_visible_count: skills.len(), manual_only_count: 0,
                name_only_count: 0, conditional_count: 0, estimated_tokens: 1, skills,
            };
            let mut estimates = vec![estimate(AgentKind::Codex, vec![skill(&a), skill(&b)])];
            if state == "active" { estimates.push(estimate(AgentKind::Proma, vec![skill(&copy)])); }
            let installation = |path: &Path| ProjectSkillInstallation {
                target: InstallTarget::Codex, path: path.to_path_buf(), managed: true, source: None,
                removable: true, toggleable: false, bundled: false, integration: None, state: None, status: None,
            };
            let direct = [ProjectSkill { name: "example".into(), installations: vec![
                installation(&a), installation(&b), ProjectSkillInstallation {
                    target: InstallTarget::Proma, path: copy.clone(), managed: true, source: Some(a.clone()),
                    removable: false, toggleable: false, bundled: false, integration: None,
                    state: Some(state.into()), status: Some("modified".into()),
                },
            ] }];
            let rows = skill_rows(&estimates, &direct, None, temp.path());
            assert_eq!(rows.len(), 2);
            let row_a = rows.iter().find(|row| row.key == skill_file_key(&a.join("SKILL.md"))).unwrap();
            let row_b = rows.iter().find(|row| row.key == skill_file_key(&b.join("SKILL.md"))).unwrap();
            assert_eq!(row_a.agents, [AgentKind::Codex, AgentKind::Proma]);
            assert_eq!(row_b.agents, [AgentKind::Codex]);
            assert_eq!(row_a.removable_installations().iter().map(|item| &item.path).collect::<Vec<_>>(), [&a]);
            assert_eq!(row_b.removable_installations().iter().map(|item| &item.path).collect::<Vec<_>>(), [&b]);
            assert!(row_a.batch_installations(ProjectBatchAction::EnableProma).is_empty());
            assert!(row_a.batch_installations(ProjectBatchAction::DisableProma).is_empty());
        }
    }

    #[test]
    fn copy_source_metadata_is_optional_and_round_trips() {
        let old = serde_json::json!({"target":"proma","path":"/native/skills/example","managed":true});
        let mut installation: ProjectSkillInstallation = serde_json::from_value(old).unwrap();
        assert!(installation.source.is_none());
        assert!(serde_json::to_value(&installation).unwrap().get("source").is_none());
        installation.source = Some("/library/example".into());
        let encoded = serde_json::to_value(&installation).unwrap();
        let decoded: ProjectSkillInstallation = serde_json::from_value(encoded.clone()).unwrap();
        assert_eq!(decoded.source, installation.source);
        assert_eq!(serde_json::to_value(decoded).unwrap(), encoded);
    }

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
                managed: false, source: None, removable: false, toggleable: true, bundled: true, integration: None,
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
                target: InstallTarget::Proma, path: folder, managed: false, source: None, removable: false, toggleable: false,
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
            target, path, managed: false, source: None, removable: false, toggleable: false, bundled: false, integration: None,
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
            target: InstallTarget::Proma, path: "/native/skills/preset".into(), managed: false, source: None, removable: false, toggleable: false,
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
    fn source_groups_count_each_skill_once_and_omit_empty_categories() {
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
        let groups = skill_source_groups(&rows);
        assert_eq!(groups.iter().map(|(group, count)| (group.key(), *count)).collect::<Vec<_>>(),
            [("project", 1), ("user", 2), ("parent", 1), ("system", 1), ("builtin", 1), ("integration", 1), ("multiple", 1)]);
        assert_eq!(groups.iter().map(|(_, count)| count).sum::<usize>(), rows.len());
        assert_eq!(skill_source_groups(&rows[..1]), [(SkillSourceGroup::Origin(SkillOrigin::Project), 1)]);
        assert!(skill_source_groups(&[]).is_empty());
    }

    #[test]
    fn proma_presets_are_builtin_without_becoming_removable() {
        let direct = [ProjectSkill { name: "preset".into(), installations: vec![ProjectSkillInstallation {
            target: InstallTarget::Proma, path: "/native/skills-inactive/preset".into(),
            managed: false, source: None, removable: false, toggleable: false, bundled: true, integration: None,
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
                managed: false, source: None, removable: false, toggleable: false, bundled: false, integration: None,
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
            managed: true, source: None,
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
        assert!(!rows[0].is_inactive());
        assert!(rows[1].is_inactive());
        let mut shared = rows[1].clone();
        shared.agents.insert(0, AgentKind::Codex);
        assert!(shared.proma_is_inactive());
        assert!(!shared.is_inactive(), "Codex remains active when only its Proma copy is inactive");
        shared.direct_installations.push(rows[0].direct_installations[0].clone());
        assert!(!shared.proma_is_inactive(), "mixed Proma copies must not all appear inactive");
        assert!(skill_rows(&[], &direct, Some(AgentKind::Codex), Path::new("/pilot")).is_empty());
    }
}
