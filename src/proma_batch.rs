//! Explicit multi-workspace state changes, bound to the previewed installations.
use crate::proma::{self, Installation, Workspace};
use anyhow::{Context, Result};
use std::{collections::{BTreeMap, BTreeSet}, path::{Path, PathBuf}};

#[derive(Clone, Debug)]
pub struct Skill {
    pub key: String,
    pub name: String,
    pub source: PathBuf,
    pub provider: &'static str,
}

pub fn source_key(path: &Path) -> String {
    format!("file:{}", path.canonicalize().unwrap_or_else(|_| path.to_path_buf()).display())
}

pub fn installation_key(copy: &Installation) -> String {
    if copy.managed && let Some(source) = &copy.source {
        return source_key(&source.join("SKILL.md"));
    }
    if copy.bundled { return format!("proma:builtin:{}", copy.name); }
    if let Some(provider) = &copy.integration {
        return format!("proma:integration:{provider}:{}", copy.name);
    }
    // Untrusted names never join installations from other workspaces.
    source_key(&copy.path.join("SKILL.md"))
}

#[derive(Clone, Debug)]
pub struct WorkspaceSnapshot {
    pub workspace: Workspace,
    pub installations: Vec<Installation>,
    pub error: Option<String>,
    pub identities: BTreeMap<PathBuf, String>,
}

#[derive(Clone, Debug)]
pub struct Catalog {
    pub root: PathBuf,
    pub workspaces: Vec<WorkspaceSnapshot>,
    pub skills: Vec<Skill>,
}

impl Catalog {
    pub fn scan(root: &Path) -> Result<Self> {
        let mut workspaces = Vec::new();
        let mut skills = BTreeMap::new();
        for workspace in proma::workspaces(root)? {
            let (installations, error) = match proma::inspect_in(root, &workspace.path) {
                Ok(copies) => (copies, None),
                Err(error) => (Vec::new(), Some(format!("{error:#}"))),
            };
            for copy in &installations {
                let key = installation_key(copy);
                skills.entry(key.clone()).or_insert_with(|| Skill {
                    key,
                    name: copy.name.clone(),
                    source: copy.source.clone().unwrap_or_else(|| copy.path.clone()),
                    provider: if copy.managed { "managed" } else if copy.bundled { "builtin" }
                        else if copy.integration.is_some() { "integration" }
                        else if copy.status == "broken_link" { "broken-link" }
                        else if copy.status == "external_link" { "link" } else { "unmanaged" },
                });
            }
            let identities = installations.iter().map(|copy| (copy.path.clone(), installation_key(copy))).collect();
            workspaces.push(WorkspaceSnapshot { workspace, installations, error, identities });
        }
        workspaces.sort_by_key(|w| (w.workspace.name.to_lowercase(), w.workspace.id.clone()));
        let mut skills = skills.into_values().collect::<Vec<_>>();
        skills.sort_by_key(|s| (s.name.to_lowercase(), s.key.clone()));
        Ok(Self { root: root.canonicalize().unwrap_or_else(|_| root.to_path_buf()), workspaces, skills })
    }

    pub fn plan(&self, skills: &BTreeSet<String>, workspaces: &BTreeSet<String>, enabled: bool) -> Plan {
        let mut items = Vec::new();
        for entry in self.workspaces.iter().filter(|w| workspaces.contains(&w.workspace.id)) {
            for skill in self.skills.iter().filter(|s| skills.contains(&s.key)) {
                let copies = entry.installations.iter().filter(|copy| entry.identities.get(&copy.path) == Some(&skill.key)).collect::<Vec<_>>();
                let (disposition, reason) = if let Some(error) = &entry.error {
                    (Disposition::Unavailable, Some(error.clone()))
                } else if copies.is_empty() {
                    (Disposition::Missing, None)
                } else if copies.len() > 1 {
                    (Disposition::Protected, Some("同一工作区存在重复副本，请先检查启用和停用目录".into()))
                } else if !copies[0].can_toggle() {
                    (Disposition::Protected, Some(protection_reason(copies[0]).into()))
                } else if (copies[0].state == "active") == enabled {
                    (Disposition::Unchanged, None)
                } else {
                    (Disposition::Change, None)
                };
                items.push(PlanItem {
                    workspace: entry.workspace.clone(), skill: skill.clone(),
                    installation: copies.first().map(|copy| (*copy).clone()), disposition, reason,
                });
            }
        }
        Plan { root: self.root.clone(), enabled, items }
    }
}

fn protection_reason(copy: &Installation) -> &'static str {
    match copy.status.as_str() {
        "preset_variant" => "匹配本机预设模板，但与当前应用自带内容不同；保留副本，启停仍受保护",
        "modified" => "副本已修改，保留现有内容",
        "external_link" | "broken_link" => "旧软链接请在用户技能中移除或转换为副本",
        _ if copy.managed => "用户技能使用安装、卸载；启用、停用仅用于 Proma 内置预设",
        _ if copy.integration.is_some() => "由集成维护，不支持在此启停",
        "unmanaged" if !copy.bundled => "未托管副本，无法核验来源",
        _ => "副本受保护或工作区有待恢复操作，请检查后刷新",
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disposition { Change, Unchanged, Missing, Protected, Unavailable }

#[derive(Clone, Debug)]
pub struct PlanItem {
    pub workspace: Workspace,
    pub skill: Skill,
    pub installation: Option<Installation>,
    pub disposition: Disposition,
    pub reason: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Plan {
    pub root: PathBuf,
    pub enabled: bool,
    pub items: Vec<PlanItem>,
}

impl Plan {
    pub fn count(&self, disposition: Disposition) -> usize {
        self.items.iter().filter(|item| item.disposition == disposition).count()
    }

    pub fn execute(&self) -> BatchResult {
        let outcomes = self.items.iter().map(|item| {
            let result = if item.disposition == Disposition::Change {
                apply_item(&self.root, item, self.enabled)
            } else { Ok(false) };
            match result {
                Ok(changed) => Outcome { item: item.clone(), changed, error: None },
                Err(error) => Outcome { item: item.clone(), changed: false, error: Some(format!("{error:#}")) },
            }
        }).collect();
        BatchResult { root: self.root.clone(), enabled: self.enabled, outcomes }
    }
}

fn apply_item(root: &Path, item: &PlanItem, enabled: bool) -> Result<bool> {
    let expected = item.installation.as_ref().ok_or_else(|| anyhow::anyhow!("缺少预览安装位置"))?;
    let workspace = proma::workspace_for_path(root, &item.workspace.path)?;
    anyhow::ensure!(workspace.id == item.workspace.id && workspace.slug == item.workspace.slug,
        "工作区身份已变化，请刷新后重新选择");
    let copies = proma::inspect_in(root, &workspace.path)?;
    let matching = copies.iter().filter(|copy| installation_key(copy) == item.skill.key).collect::<Vec<_>>();
    anyhow::ensure!(matching.len() == 1, "副本缺失或存在重复副本，请刷新后检查");
    let copy = matching[0];
    anyhow::ensure!(copy.name == expected.name && copy.source == expected.source, "副本来源已变化，请刷新后重新选择");
    anyhow::ensure!(copy.can_toggle(), "{}", protection_reason(copy));
    if (copy.state == "active") == enabled { return Ok(false); }
    anyhow::ensure!(copy.path == expected.path && copy.state == expected.state,
        "副本安装位置或状态已变化，请刷新后重新选择");
    let source_file = if expected.managed {
        Some(Path::new(item.skill.key.strip_prefix("file:").context("受管技能源身份无效")?))
    } else { None };
    proma::set_enabled_previewed_in(root, &item.workspace, expected, source_file, enabled)?;
    Ok(true)
}

#[derive(Clone, Debug)]
pub struct Outcome {
    pub item: PlanItem,
    pub changed: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct BatchResult {
    pub root: PathBuf,
    pub enabled: bool,
    pub outcomes: Vec<Outcome>,
}

impl BatchResult {
    pub fn changed(&self) -> usize { self.outcomes.iter().filter(|o| o.changed).count() }
    pub fn failed(&self) -> usize { self.outcomes.iter().filter(|o| o.error.is_some()).count() }
    pub fn retry_plan(&self) -> Plan {
        Plan { root: self.root.clone(), enabled: self.enabled,
            items: self.outcomes.iter().filter(|o| o.error.is_some()).map(|o| o.item.clone()).collect() }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;

    struct Fixture { _temp: tempfile::TempDir, root: PathBuf, source: PathBuf, workspaces: Vec<PathBuf> }
    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let base = temp.path().canonicalize().unwrap();
            let root = base.join("proma");
            let workspaces = ["a", "b", "c"].iter().map(|name| root.join("agent-workspaces").join(name)).collect::<Vec<_>>();
            for w in &workspaces { fs::create_dir_all(w).unwrap(); }
            fs::write(root.join("agent-workspaces.json"), serde_json::to_vec(&serde_json::json!({"version":2,"workspaces":[
                {"id":"a", "slug":"a", "name":"A", "projectRootPath":base},
                {"id":"b", "slug":"b", "name":"B", "projectRootPath":base},
                {"id":"c", "slug":"c", "name":"C", "archived":true}
            ]})).unwrap()).unwrap();
            let source = base.join("library");
            for name in ["alpha", "beta"] {
                fs::create_dir_all(source.join(name).join("local-data")).unwrap();
                fs::write(source.join(name).join("SKILL.md"), format!("---\nname: {name}\ndescription: Test\n---\nTest body\n")).unwrap();
                fs::write(source.join(name).join("local-data/retained.txt"), "keep local data").unwrap();
            }
            Self { _temp: temp, root, source, workspaces }
        }
        fn install(&self, workspace: usize, name: &str) {
            let template = self.root.join("fixture-app/default-skills").join(name);
            let copy = self.workspaces[workspace].join("skills").join(name);
            fs::create_dir_all(&template).unwrap(); fs::create_dir_all(&copy).unwrap();
            fs::copy(self.source.join(name).join("SKILL.md"), template.join("SKILL.md")).unwrap();
            fs::copy(template.join("SKILL.md"), copy.join("SKILL.md")).unwrap();
            proma::register_test_bundled_templates(&self.root, &self.root.join("fixture-app/default-skills"));
        }
        fn skills(&self, names: &[&str]) -> BTreeSet<String> {
            names.iter().map(|name| format!("proma:builtin:{name}")).collect()
        }
        fn plan(&self, names: &[&str], enabled: bool) -> Plan {
            Catalog::scan(&self.root).unwrap().plan(&self.skills(names), &BTreeSet::from(["a".into(), "b".into()]), enabled)
        }
    }

    #[test]
    fn same_source_copies_merge_and_only_selected_workspaces_change() {
        let f = Fixture::new();
        for i in 0..3 { f.install(i, "alpha"); }
        f.install(0, "beta");
        proma::set_enabled_path_in(&f.root, &f.workspaces[1].join("skills/alpha"), false).unwrap();
        let original = fs::read(f.source.join("alpha/SKILL.md")).unwrap();
        let copy_original = fs::read(f.workspaces[0].join("skills/alpha/SKILL.md")).unwrap();
        let catalog = Catalog::scan(&f.root).unwrap();
        assert_eq!(catalog.skills.len(), 2);
        assert!(catalog.workspaces[2].workspace.archived);
        let plan = f.plan(&["alpha", "beta"], false);
        assert_eq!(plan.count(Disposition::Change), 2);
        assert_eq!(plan.count(Disposition::Unchanged), 1);
        assert_eq!(plan.count(Disposition::Missing), 1);
        let result = plan.execute();
        assert_eq!((result.changed(), result.failed()), (2, 0));
        assert!(f.workspaces[2].join("skills/alpha/SKILL.md").is_file());
        assert!(!f.workspaces[1].join("skills/beta").exists());
        assert_eq!(fs::read(f.source.join("alpha/SKILL.md")).unwrap(), original);
        assert_eq!(fs::read_to_string(f.source.join("alpha/local-data/retained.txt")).unwrap(), "keep local data");
        assert_eq!(f.plan(&["alpha", "beta"], true).execute().changed(), 3);
        assert_eq!(fs::read(f.workspaces[0].join("skills/alpha/SKILL.md")).unwrap(), copy_original);
    }

    #[test]
    fn same_name_other_sources_and_unmanaged_copies_stay_separate() {
        let f = Fixture::new();
        f.install(0, "alpha");
        let other = f.source.join("other-alpha");
        fs::create_dir(&other).unwrap();
        fs::write(other.join("SKILL.md"), "---\nname: alpha\ndescription: Other source\n---\nIndependent user skill.\n").unwrap();
        proma::apply_plan(&proma::plan_install(&f.root, &f.workspaces[1], &other, "alpha").unwrap()).unwrap();
        let unmanaged = f.workspaces[2].join("skills/alpha");
        fs::create_dir_all(&unmanaged).unwrap();
        fs::copy(other.join("SKILL.md"), unmanaged.join("SKILL.md")).unwrap();
        let catalog = Catalog::scan(&f.root).unwrap();
        assert_eq!(catalog.skills.len(), 3);
        let plan = catalog.plan(&f.skills(&["alpha"]), &BTreeSet::from(["a".into(), "b".into(), "c".into()]), false);
        assert_eq!(plan.count(Disposition::Change), 1);
        assert_eq!(plan.count(Disposition::Missing), 2);
        assert_eq!(plan.execute().changed(), 1);
        assert!(f.workspaces[1].join("skills/alpha").is_dir());
        assert!(unmanaged.is_dir());
    }

    #[test]
    fn partial_failure_retries_only_failed_installations() {
        let f = Fixture::new();
        for i in 0..2 { f.install(i, "alpha"); }
        let plan = f.plan(&["alpha"], false);
        let path = f.workspaces[1].join("skills/alpha/SKILL.md");
        let original = fs::read(&path).unwrap();
        fs::write(&path, "local changes after preview").unwrap();
        let result = plan.execute();
        assert_eq!((result.changed(), result.failed()), (1, 1));
        assert_eq!(fs::read_to_string(&path).unwrap(), "local changes after preview");
        let retry = result.retry_plan();
        assert_eq!(retry.items.len(), 1);
        assert_eq!(retry.items[0].workspace.id, "b");
        fs::write(&path, original).unwrap();
        let first = f.workspaces[0].join("skills-inactive/alpha/SKILL.md");
        fs::write(&first, "preserve successful workspace").unwrap();
        let result = retry.execute();
        assert_eq!((result.changed(), result.failed()), (1, 0));
        assert_eq!(fs::read_to_string(first).unwrap(), "preserve successful workspace");
    }

    #[test]
    fn planned_missing_copies_are_not_added_to_the_operation_later() {
        let f = Fixture::new();
        f.install(0, "alpha");
        let plan = f.plan(&["alpha"], false);
        f.install(1, "alpha");
        assert_eq!(plan.execute().changed(), 1);
        assert!(f.workspaces[1].join("skills/alpha").is_dir());
    }

    #[test]
    fn changed_workspace_identity_or_preset_content_is_rejected() {
        let f = Fixture::new();
        for i in 0..2 { f.install(i, "alpha"); }
        let plan = f.plan(&["alpha"], false);
        fs::write(f.workspaces[0].join("skills/alpha/SKILL.md"), "modified preset").unwrap();
        let index_path = f.root.join("agent-workspaces.json");
        let mut index: serde_json::Value = serde_json::from_slice(&fs::read(&index_path).unwrap()).unwrap();
        index["workspaces"][1]["id"] = serde_json::json!("replaced-b");
        fs::write(index_path, serde_json::to_vec(&index).unwrap()).unwrap();
        let result = plan.execute();
        assert_eq!((result.changed(), result.failed()), (0, 2));
        for w in &f.workspaces[..2] { assert!(w.join("skills/alpha").is_dir()); }
    }

    #[test]
    fn protected_and_unavailable_pairs_are_reported_without_writing() {
        let f = Fixture::new();
        f.install(0, "alpha");
        let path = f.workspaces[0].join("skills/alpha/SKILL.md");
        fs::write(&path, "keep modified copy").unwrap();
        fs::write(f.workspaces[1].join("skills"), "invalid directory").unwrap();
        let plan = Catalog::scan(&f.root).unwrap().plan(&BTreeSet::from([source_key(&path)]),
            &BTreeSet::from(["a".into(), "b".into()]), false);
        assert_eq!(plan.count(Disposition::Protected), 1);
        assert_eq!(plan.count(Disposition::Unavailable), 1);
        assert_eq!(plan.execute().changed(), 0);
        assert_eq!(fs::read_to_string(path).unwrap(), "keep modified copy");
    }

    #[test]
    fn duplicate_active_and_inactive_copies_are_protected() {
        let f = Fixture::new();
        f.install(0, "alpha");
        let copy = f.workspaces[0].join("skills-inactive/alpha");
        fs::create_dir_all(&copy).unwrap();
        for name in ["SKILL.md"] {
            fs::copy(f.workspaces[0].join("skills/alpha").join(name), copy.join(name)).unwrap();
        }
        let plan = f.plan(&["alpha"], false);
        assert_eq!(plan.count(Disposition::Protected), 1);
        assert_eq!(plan.execute().changed(), 0);
    }

    #[test]
    fn preset_modified_after_batch_preflight_is_rejected_inside_write_lock() {
        let f = Fixture::new();
        f.install(0, "alpha");
        let plan = f.plan(&["alpha"], false);
        let preset_path = f.workspaces[0].join("skills/alpha/SKILL.md");
        proma::BEFORE_TOGGLE_LOCK.with(|hook| *hook.borrow_mut() = Some(Box::new(move || {
            fs::write(preset_path, "keep concurrent edit").unwrap();
        })));
        let result = plan.execute();
        assert_eq!((result.changed(), result.failed()), (0, 1));
        assert!(f.workspaces[0].join("skills/alpha/SKILL.md").is_file());
        assert!(!f.workspaces[0].join("skills-inactive").exists());
    }

    #[test]
    fn workspace_replaced_after_batch_preflight_is_rejected_inside_write_lock() {
        let f = Fixture::new();
        f.install(0, "alpha");
        let plan = f.plan(&["alpha"], false);
        let index_path = f.root.join("agent-workspaces.json");
        proma::BEFORE_TOGGLE_LOCK.with(|hook| *hook.borrow_mut() = Some(Box::new(move || {
            let mut index: serde_json::Value = serde_json::from_slice(&fs::read(&index_path).unwrap()).unwrap();
            index["workspaces"][0]["id"] = serde_json::json!("replaced-a");
            fs::write(index_path, serde_json::to_vec(&index).unwrap()).unwrap();
        })));
        let result = plan.execute();
        assert_eq!((result.changed(), result.failed()), (0, 1));
        assert!(f.workspaces[0].join("skills/alpha/SKILL.md").is_file());
        assert!(!f.workspaces[0].join("skills-inactive").exists());
    }

    #[test]
    fn user_copies_cannot_be_toggled_or_moved_into_inactive() {
        let f = Fixture::new();
        for workspace in &f.workspaces[..2] {
            proma::apply_plan(&proma::plan_install(&f.root, workspace, &f.source.join("alpha"), "alpha").unwrap()).unwrap();
        }
        let catalog = Catalog::scan(&f.root).unwrap();
        let keys = BTreeSet::from([source_key(&f.source.join("alpha/SKILL.md"))]);
        let plan = catalog.plan(&keys, &BTreeSet::from(["a".into(), "b".into()]), false);
        assert_eq!(plan.count(Disposition::Protected), 2);
        assert_eq!(plan.execute().changed(), 0);
        for workspace in &f.workspaces[..2] {
            assert!(workspace.join("skills/alpha/SKILL.md").is_file());
            assert!(!workspace.join("skills-inactive").exists());
        }
    }

    #[test]
    fn lost_preset_template_after_preview_is_rejected_inside_write_lock() {
        let f = Fixture::new(); f.install(0, "alpha");
        let plan = f.plan(&["alpha"], false);
        let template = f.root.join("fixture-app/default-skills/alpha/SKILL.md");
        proma::BEFORE_TOGGLE_LOCK.with(|hook| *hook.borrow_mut() = Some(Box::new(move || { fs::remove_file(template).unwrap(); })));
        let result = plan.execute();
        assert_eq!((result.changed(), result.failed()), (0, 1));
        assert!(f.workspaces[0].join("skills/alpha/SKILL.md").is_file());
        assert!(!f.workspaces[0].join("skills-inactive").exists());
    }
}
