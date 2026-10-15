//! Read project catalogs without scanning conversations or changing Agent data.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

use anyhow::{Result, anyhow};
use serde::Serialize;
use serde_json::Value;

use crate::effective_skills::AgentKind;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProjectEntry {
    pub path: PathBuf,
    pub name: String,
    /// Agents whose project catalogs explicitly contain this directory.
    pub agents: Vec<AgentKind>,
    /// Native Skill roots remain separate from the Finder display identity.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub proma_workspaces: Vec<PathBuf>,
}

impl ProjectEntry {
    /// Project-local directories this Agent can read. Proma uses its native
    /// workspaces rather than the business directory shown in the project list.
    pub fn skill_directories(&self, agent: crate::agents::AgentIconInfo) -> Vec<PathBuf> {
        use crate::{InstallTarget, agents::installation_root};

        if agent.global_only {
            return Vec::new();
        }
        let mut roots = Vec::new();
        for target in agent.install_targets {
            if *target == InstallTarget::Proma {
                for workspace in &self.proma_workspaces {
                    roots.push(workspace.join("skills"));
                }
            } else {
                roots.push(installation_root(&self.path, *target));
            }
        }
        if agent.shared_agents {
            roots.push(installation_root(&self.path, InstallTarget::Universal));
        }
        let mut seen = BTreeSet::new();
        roots.retain(|root| seen.insert(path_identity(root)));
        roots
    }
}

pub fn path_identity(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

pub fn codex_home() -> Option<PathBuf> {
    // Only selects the read-only catalog; skill operations still use CODEX_HOME.
    std::env::var_os("KITTER_CODEX_PROJECTS_HOME")
        .or_else(|| std::env::var_os("CODEX_HOME"))
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".codex")))
}

pub fn discover_proma_projects(root: &Path) -> Result<Vec<ProjectEntry>> {
    let mut projects = crate::proma::workspaces(root)?
        .into_iter()
        .map(|workspace| ProjectEntry {
            path: workspace.project_root_path.unwrap_or_else(|| workspace.path.clone()),
            name: workspace.name,
            agents: vec![AgentKind::Proma],
            proma_workspaces: vec![workspace.path],
        })
        .collect::<Vec<_>>();
    let mut counts = BTreeMap::new();
    for project in &projects {
        *counts.entry(project.path.clone()).or_insert(0) += 1;
    }
    for project in &mut projects {
        if counts[&project.path] > 1 {
            project.name = project_name(&project.path);
        }
    }
    Ok(projects)
}

pub fn discover_agent_projects() -> Result<Vec<ProjectEntry>> {
    discover_catalogs(codex_home().as_deref(), crate::proma::home().as_deref())
}

fn discover_catalogs(codex: Option<&Path>, proma: Option<&Path>) -> Result<Vec<ProjectEntry>> {
    let mut entries = Vec::new();
    let mut failure = None;
    for catalog in [codex.map(discover_codex_projects), proma.map(discover_proma_projects)].into_iter().flatten() {
        match catalog {
            Ok(projects) => entries.extend(projects),
            Err(error) => failure = Some(error),
        }
    }
    if entries.is_empty() && let Some(error) = failure {
        return Err(error);
    }
    Ok(merge_projects(&[], &entries, &BTreeSet::new()))
}

pub fn discover_codex_projects(home: &Path) -> Result<Vec<ProjectEntry>> {
    let file = match File::open(home.join(".codex-global-state.json")) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err(anyhow!("无法读取 Codex 项目记录")),
    };
    let mut bytes = Vec::new();
    file.take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| anyhow!("无法读取 Codex 项目记录"))?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err(anyhow!("Codex 项目记录过大"));
    }
    let state: Value =
        serde_json::from_slice(&bytes).map_err(|_| anyhow!("Codex 项目记录格式无效"))?;
    let object = state
        .as_object()
        .ok_or_else(|| anyhow!("Codex 项目记录格式无效"))?;
    let mut entries = Vec::new();
    if let Some(current) = object.get("local-projects") {
        // The primary root identifies a project; remaining roots only extend
        // its workspace. An empty current catalog supersedes legacy history.
        let projects = current
            .as_object()
            .ok_or_else(|| anyhow!("Codex 项目列表格式无效"))?;
        for (id, project) in projects {
            // ChatGPT cloud projects share this catalog but are not local projects.
            if id.starts_with("g-p-") {
                continue;
            }
            let path = project
                .get("rootPaths")
                .and_then(Value::as_array)
                .and_then(|roots| roots.first())
                .and_then(Value::as_str);
            if let Some(path) = path {
                append_codex(
                    &mut entries,
                    path,
                    project.get("name").and_then(Value::as_str),
                );
            }
        }
    } else if let Some(roots) = object
        .get("electron-saved-workspace-roots")
        .and_then(Value::as_array)
    {
        for path in roots.iter().filter_map(Value::as_str) {
            append_codex(&mut entries, path, None);
        }
    }
    let cloud_cache = path_identity(&home.join(".chatgpt-projects"));
    entries.retain(|entry| !entry.path.starts_with(&cloud_cache));
    let mut merged = merge_projects(&[], &entries, &BTreeSet::new());
    merged.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.path.cmp(&b.path)));
    Ok(merged)
}

fn append_codex(entries: &mut Vec<ProjectEntry>, path: &str, name: Option<&str>) {
    let path = Path::new(path);
    if !path.is_absolute() || !path.is_dir() {
        return;
    }
    let path = path_identity(path);
    entries.push(ProjectEntry {
        name: name
            .filter(|name| !name.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| project_name(&path)),
        path,
        agents: vec![AgentKind::Codex],
        proma_workspaces: Vec::new(),
    });
}

fn project_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

pub fn project_identity(path: &Path, discovered: &[ProjectEntry]) -> PathBuf {
    let identity = path_identity(path);
    discovered.iter()
        .find(|entry| entry.proma_workspaces.iter().any(|native| path_identity(native) == identity))
        .map(|entry| path_identity(&entry.path))
        .unwrap_or(identity)
}

/// A Finder project is one row, even when an Agent stores Skills elsewhere.
pub fn merge_projects(
    manual: &[PathBuf],
    discovered: &[ProjectEntry],
    hidden: &BTreeSet<PathBuf>,
) -> Vec<ProjectEntry> {
    let hidden = hidden
        .iter()
        .map(|path| project_identity(path, discovered))
        .collect::<BTreeSet<_>>();
    let mut merged: Vec<ProjectEntry> = Vec::new();
    for path in manual {
        let path = project_identity(path, discovered);
        if hidden.contains(&path) {
            continue;
        }
        if !merged.iter().any(|entry| entry.path == path) {
            merged.push(ProjectEntry {
                name: project_name(&path),
                path,
                agents: Vec::new(),
                proma_workspaces: Vec::new(),
            });
        }
    }
    for candidate in discovered {
        let path = path_identity(&candidate.path);
        if hidden.contains(&path) || !path.is_dir() {
            continue;
        }
        if let Some(existing) = merged.iter_mut().find(|entry| entry.path == path) {
            if existing.agents.is_empty() {
                existing.name = candidate.name.clone();
            }
            for agent in &candidate.agents {
                if !existing.agents.contains(agent) {
                    existing.agents.push(*agent);
                }
            }
            for workspace in &candidate.proma_workspaces {
                if !existing.proma_workspaces.contains(workspace) {
                    existing.proma_workspaces.push(workspace.clone());
                }
            }
        } else {
            merged.push(ProjectEntry {
                path,
                ..candidate.clone()
            });
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn project_shortcuts_use_shared_and_native_roots_without_creating_them() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("business repo");
        let first = temp.path().join("proma/first");
        let second = temp.path().join("proma/second");
        let entry = ProjectEntry {
            path: project.clone(),
            name: "Project".into(),
            agents: vec![AgentKind::Codex, AgentKind::Proma],
            proma_workspaces: vec![first.clone(), second.clone(), first.clone()],
        };
        let agent = |id| *crate::agents::AGENT_ICON_ORDER.iter()
            .find(|agent| agent.id == id).unwrap();
        assert_eq!(entry.skill_directories(agent("codex")), [
            project.join(".codex/skills"), project.join(".agents/skills"),
        ]);
        assert_eq!(entry.skill_directories(agent("proma")), [
            first.join("skills"), second.join("skills"),
        ]);
        assert!(entry.skill_directories(agent("openclaw")).is_empty());
        assert!(!project.exists());
        assert!(!first.exists());
        assert!(!second.exists());
    }

    #[cfg(unix)]
    #[test]
    fn project_shortcuts_deduplicate_aliases_without_changing_the_display_path() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        let shared = project.join(".agents/skills");
        std::fs::create_dir_all(&shared).unwrap();
        std::fs::create_dir_all(project.join(".codex")).unwrap();
        std::os::unix::fs::symlink(&shared, project.join(".codex/skills")).unwrap();
        let entry = ProjectEntry {
            path: project.clone(), name: "Project".into(),
            agents: vec![AgentKind::Codex], proma_workspaces: Vec::new(),
        };
        let codex = *crate::agents::AGENT_ICON_ORDER.iter()
            .find(|agent| agent.id == "codex").unwrap();
        assert_eq!(entry.skill_directories(codex), [project.join(".codex/skills")]);
    }

    fn write_state(home: &Path, value: Value) {
        std::fs::write(
            home.join(".codex-global-state.json"),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn proma_discovery_groups_by_business_root() {
        let temp = tempfile::tempdir().unwrap();
        let native = temp.path().join("agent-workspaces/ai-cube-pilot");
        let business = temp.path().join("business-repo");
        std::fs::create_dir_all(&native).unwrap();
        std::fs::create_dir_all(&business).unwrap();
        std::fs::write(temp.path().join("agent-workspaces.json"), serde_json::to_vec(&json!({
            "version": 1, "workspaces": [{"id": "pilot-id", "name": "试点工作区",
                "slug": "ai-cube-pilot", "projectRootPath": business}]
        })).unwrap()).unwrap();
        let projects = discover_proma_projects(temp.path()).unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].path, path_identity(&business));
        assert_eq!(projects[0].name, "试点工作区");
        assert_eq!(projects[0].agents, [AgentKind::Proma]);
        assert_eq!(projects[0].proma_workspaces, [path_identity(&native)]);
        assert_ne!(projects[0].path, path_identity(&native));
    }

    #[cfg(unix)]
    #[test]
    fn multi_workspace_projects_use_business_name_and_keep_native_roots() {
        let temp = tempfile::tempdir().unwrap();
        let business = temp.path().join("shared-project");
        std::fs::create_dir(&business).unwrap();
        for slug in ["first", "second"] {
            std::fs::create_dir_all(temp.path().join("agent-workspaces").join(slug)).unwrap();
        }
        let first = json!({"id": "first", "slug": "first", "name": "演示工作区 A", "projectRootPath": business});
        let second = json!({"id": "second", "slug": "second", "name": "演示工作区 B", "projectRootPath": business});
        for workspaces in [vec![first.clone(), second.clone()], vec![second.clone(), first.clone()]] {
            std::fs::write(temp.path().join("agent-workspaces.json"),
                serde_json::to_vec(&json!({"version": 2, "workspaces": workspaces})).unwrap()).unwrap();
            let discovered = discover_proma_projects(temp.path()).unwrap();
            let projects = merge_projects(&[], &discovered, &BTreeSet::new());
            assert_eq!(projects.len(), 1);
            assert_eq!(projects[0].path, path_identity(&business));
            assert_eq!(projects[0].name, "shared-project");
            assert_eq!(projects[0].proma_workspaces.len(), 2);
            assert!(projects[0].proma_workspaces.contains(&path_identity(&temp.path().join("agent-workspaces/first"))));
            assert!(projects[0].proma_workspaces.contains(&path_identity(&temp.path().join("agent-workspaces/second"))));
        }
        std::fs::write(temp.path().join("agent-workspaces.json"),
            serde_json::to_vec(&json!({"version": 2, "workspaces": [first]})).unwrap()).unwrap();
        assert_eq!(discover_proma_projects(temp.path()).unwrap()[0].name, "演示工作区 A");
    }

    #[test]
    fn malformed_proma_catalog_does_not_hide_codex_projects() {
        let temp = tempfile::tempdir().unwrap();
        let codex = temp.path().join("codex");
        let proma = temp.path().join("proma");
        std::fs::create_dir(&codex).unwrap();
        std::fs::create_dir(&proma).unwrap();
        write_state(&codex, json!({"electron-saved-workspace-roots": [temp.path()]}));
        std::fs::write(proma.join("agent-workspaces.json"), "fixture-private").unwrap();
        let projects = discover_catalogs(Some(&codex), Some(&proma)).unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].agents, [AgentKind::Codex]);
    }

    #[cfg(unix)]
    #[test]
    fn linked_proma_and_codex_merge_manual_native_alias_and_hidden_identity() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("proma");
        let native = root.join("agent-workspaces/ai-agent");
        let business = temp.path().join("AI Agent");
        std::fs::create_dir_all(&native).unwrap();
        std::fs::create_dir(&business).unwrap();
        std::fs::write(root.join("agent-workspaces.json"), serde_json::to_vec(&json!({
            "version": 2, "workspaces": [{"id": "workspace", "slug": "ai-agent",
                "name": "AI Agent", "projectRootPath": business}]
        })).unwrap()).unwrap();
        write_state(temp.path(), json!({"local-projects": {
            "project": {"name": "AI Agent", "rootPaths": [business]}
        }}));
        let discovered = discover_catalogs(Some(temp.path()), Some(&root)).unwrap();
        let merged = merge_projects(&[native.clone(), business.clone()], &discovered, &BTreeSet::new());
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].path, path_identity(&business));
        assert_eq!(merged[0].agents, [AgentKind::Codex, AgentKind::Proma]);
        assert_eq!(merged[0].proma_workspaces, [path_identity(&native)]);
        assert_eq!(project_identity(&native, &discovered), path_identity(&business));
        for hidden in [native.clone(), business.clone()] {
            assert!(merge_projects(&[native.clone()], &discovered, &BTreeSet::from([hidden])).is_empty());
        }
    }

    #[cfg(unix)]
    #[test]
    fn unlinked_or_invalid_business_roots_keep_distinct_native_workspaces() {
        let temp = tempfile::tempdir().unwrap();
        let workspaces = [
            json!({"id": "standalone", "slug": "standalone", "name": "Same name"}),
            json!({"id": "missing", "slug": "missing", "name": "Same name",
                "projectRootPath": temp.path().join("deleted-project")}),
            json!({"id": "relative", "slug": "relative", "name": "Same name", "projectRootPath": "relative"}),
        ];
        for slug in ["standalone", "missing", "relative"] {
            std::fs::create_dir_all(temp.path().join("agent-workspaces").join(slug)).unwrap();
        }
        std::fs::write(temp.path().join("agent-workspaces.json"),
            serde_json::to_vec(&json!({"version": 2, "workspaces": workspaces})).unwrap()).unwrap();
        let discovered = discover_proma_projects(temp.path()).unwrap();
        let merged = merge_projects(&[], &discovered, &BTreeSet::new());
        assert_eq!(merged.len(), 3);
        assert!(merged.iter().all(|entry| entry.path == entry.proma_workspaces[0]));
    }

    #[cfg(windows)]
    #[test]
    fn proma_discovery_rejects_unsupported_platform_without_changing_catalog() {
        let temp = tempfile::tempdir().unwrap();
        let native = temp.path().join("agent-workspaces/pilot");
        std::fs::create_dir_all(&native).unwrap();
        let catalog = temp.path().join("agent-workspaces.json");
        let bytes = serde_json::to_vec(&json!({"version": 2, "workspaces": [
            {"id": "pilot", "slug": "pilot", "name": "Pilot"}
        ]})).unwrap();
        std::fs::write(&catalog, &bytes).unwrap();
        let error = discover_proma_projects(temp.path()).unwrap_err();
        assert!(error.to_string().contains("此平台尚未支持 Proma 受管写入"));
        assert_eq!(std::fs::read(&catalog).unwrap(), bytes);
        assert_eq!(std::fs::read_dir(native).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn linked_business_symlink_identity_merges_agents() {
        let temp = tempfile::tempdir().unwrap();
        let business = temp.path().join("actual");
        let alias = temp.path().join("alias");
        let native = temp.path().join("agent-workspaces/pilot");
        std::fs::create_dir_all(&native).unwrap();
        std::fs::create_dir(&business).unwrap();
        std::os::unix::fs::symlink(&business, &alias).unwrap();
        std::fs::write(temp.path().join("agent-workspaces.json"),
            serde_json::to_vec(&json!({"version": 1, "workspaces": [
                {"id": "pilot", "slug": "pilot", "name": "Proma title", "projectRootPath": alias}
            ]})).unwrap()).unwrap();
        write_state(temp.path(), json!({"electron-saved-workspace-roots": [business]}));
        let projects = discover_catalogs(Some(temp.path()), Some(temp.path())).unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].path, path_identity(&business));
        assert_eq!(projects[0].agents, [AgentKind::Codex, AgentKind::Proma]);
    }

    #[test]
    fn current_catalog_uses_primary_root_and_ignores_stale_legacy() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("primary");
        let extra = temp.path().join("extra");
        let cloud_cache = temp.path().join(".chatgpt-projects/g-p-cache");
        std::fs::create_dir(&project).unwrap();
        std::fs::create_dir(&extra).unwrap();
        std::fs::create_dir_all(&cloud_cache).unwrap();
        write_state(
            temp.path(),
            json!({"local-projects": {
            "valid": {"name": "项目名称", "rootPaths": [project, extra]},
            "g-p-cloud": {"rootPaths": [extra]},
            "cache": {"rootPaths": [cloud_cache]},
            "deleted": {"rootPaths": [temp.path().join("missing"), extra]},
            "remote": {"rootPaths": ["ssh://host/project"]}
        }, "electron-saved-workspace-roots": [extra]}),
        );
        let found = discover_codex_projects(temp.path()).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, path_identity(&project));
        assert_eq!(found[0].name, "项目名称");
    }

    #[test]
    fn empty_current_catalog_does_not_restore_removed_legacy_projects() {
        let temp = tempfile::tempdir().unwrap();
        write_state(
            temp.path(),
            json!({"local-projects": {}, "electron-saved-workspace-roots": [temp.path()]}),
        );
        assert!(discover_codex_projects(temp.path()).unwrap().is_empty());
    }

    #[test]
    fn legacy_catalog_filters_missing_and_relative_paths() {
        let temp = tempfile::tempdir().unwrap();
        let cloud_cache = temp.path().join(".chatgpt-projects/g-p-cache");
        std::fs::create_dir_all(&cloud_cache).unwrap();
        write_state(
            temp.path(),
            json!({"electron-saved-workspace-roots": [temp.path(), temp.path(), "relative", temp.path().join("missing"), cloud_cache]}),
        );
        let found = discover_codex_projects(temp.path()).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].agents, [AgentKind::Codex]);
    }

    #[test]
    fn missing_or_invalid_state_does_not_expose_contents() {
        let temp = tempfile::tempdir().unwrap();
        assert!(discover_codex_projects(temp.path()).unwrap().is_empty());
        std::fs::write(
            temp.path().join(".codex-global-state.json"),
            "private contents",
        )
        .unwrap();
        let error = discover_codex_projects(temp.path())
            .unwrap_err()
            .to_string();
        assert!(!error.contains("private contents"));
    }

    #[test]
    fn shared_project_merges_agents_and_respects_hidden_projects() {
        let temp = tempfile::tempdir().unwrap();
        let path = path_identity(temp.path());
        let codex = ProjectEntry {
            path: path.clone(),
            name: "Shared".into(),
            agents: vec![AgentKind::Codex],
            proma_workspaces: Vec::new(),
        };
        let other = ProjectEntry {
            agents: vec![AgentKind::ClaudeCode, AgentKind::Codex],
            ..codex.clone()
        };
        let merged = merge_projects(&[path.clone()], &[codex.clone(), other], &BTreeSet::new());
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].agents, [AgentKind::Codex, AgentKind::ClaudeCode]);
        assert!(merge_projects(&[], &[codex], &BTreeSet::from([path])).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_and_real_directory_are_one_project() {
        let temp = tempfile::tempdir().unwrap();
        let actual = temp.path().join("real");
        let alias = temp.path().join("alias");
        std::fs::create_dir(&actual).unwrap();
        std::os::unix::fs::symlink(&actual, &alias).unwrap();
        write_state(
            temp.path(),
            json!({"electron-saved-workspace-roots": [actual, alias]}),
        );
        assert_eq!(discover_codex_projects(temp.path()).unwrap().len(), 1);
    }
}
