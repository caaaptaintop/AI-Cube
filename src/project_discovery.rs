//! Read project catalogs without scanning conversations or changing Agent data.
use std::{
    collections::BTreeSet,
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
    });
}

fn project_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

/// A directory is one project, regardless of which catalogs contributed it.
pub fn merge_projects(
    manual: &[PathBuf],
    discovered: &[ProjectEntry],
    hidden: &BTreeSet<PathBuf>,
) -> Vec<ProjectEntry> {
    let hidden = hidden
        .iter()
        .map(|path| path_identity(path))
        .collect::<BTreeSet<_>>();
    let mut merged: Vec<ProjectEntry> = Vec::new();
    for path in manual {
        let path = path_identity(path);
        if !merged.iter().any(|entry| entry.path == path) {
            merged.push(ProjectEntry {
                name: project_name(&path),
                path,
                agents: Vec::new(),
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

    fn write_state(home: &Path, value: Value) {
        std::fs::write(
            home.join(".codex-global-state.json"),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
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
