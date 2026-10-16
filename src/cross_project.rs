//! One explicit plan for project links and registered Proma workspace copies.
use crate::{InstallTarget, agents::{self, AgentIconInfo}, directory_link, proma, proma_batch, project_discovery::ProjectEntry};
use anyhow::{Context, Result, bail, ensure};
use std::{collections::{BTreeMap, BTreeSet}, fs, path::{Path, PathBuf}};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action { Install, Remove, Enable, Disable }

impl Action {
    pub fn is_state(self) -> bool { matches!(self, Self::Enable | Self::Disable) }
    pub fn supported(self, agents: &BTreeSet<String>) -> bool {
        !agents.is_empty() && (!self.is_state() || agents.iter().all(|id| id == "proma"))
    }
}

#[derive(Clone, Debug)]
pub struct Skill {
    pub key: String,
    pub name: String,
    pub source: PathBuf,
    pub provider: &'static str,
    /// Only library skills may be newly installed. Other entries are inspectable.
    pub install_name: Option<String>,
}

#[derive(Clone, Debug)]
struct LinkEntry { target: InstallTarget, path: PathBuf, key: String }

#[derive(Clone, Debug)]
pub struct Catalog {
    pub skills: Vec<Skill>,
    pub projects: Vec<ProjectEntry>,
    pub installed_agents: Vec<AgentIconInfo>,
    pub proma: Option<proma_batch::Catalog>,
    pub proma_error: Option<String>,
    links: BTreeMap<PathBuf, Vec<LinkEntry>>,
}

/// Resolve the existing prefix, including directory aliases, but not the entry.
fn physical_entry(path: &Path) -> Result<PathBuf> {
    let name = path.file_name().context("安装位置缺少名称")?;
    let mut parent = path.parent().context("安装位置缺少父目录")?;
    let mut missing = Vec::new();
    loop {
        match parent.symlink_metadata() {
            Ok(_) => break,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing.push(parent.file_name().context("缺少项目目录")?.to_os_string());
                parent = parent.parent().context("缺少项目目录")?;
            }
            Err(error) => return Err(error.into()),
        }
    }
    let mut result = parent.canonicalize().context("无法解析安装目录")?;
    ensure!(result.is_dir(), "安装父目录不是文件夹");
    for part in missing.into_iter().rev() { result.push(part); }
    Ok(result.join(name))
}

impl Catalog {
    pub fn scan(mut skills: Vec<Skill>, mut projects: Vec<ProjectEntry>, installed_agents: Vec<AgentIconInfo>, proma_root: Option<&Path>) -> Self {
        let (proma, proma_error) = match proma_root.map(proma_batch::Catalog::scan) {
            Some(Ok(catalog)) => (Some(catalog), None),
            Some(Err(error)) => (None, Some(format!("{error:#}"))),
            None => (None, None),
        };
        if let Some(catalog) = &proma {
            for skill in &catalog.skills {
                if !skills.iter().any(|s| s.key == skill.key) {
                    skills.push(Skill { key: skill.key.clone(), name: skill.name.clone(), source: skill.source.clone(),
                        provider: skill.provider, install_name: None });
                }
            }
            // Keep workspace-only projects reachable, including archived ones.
            for entry in &catalog.workspaces {
                let w = &entry.workspace;
                let path = w.project_root_path.as_ref().unwrap_or(&w.path);
                if let Some(project) = projects.iter_mut().find(|p| &p.path == path) {
                    if project.agents.is_empty() && project.proma_workspaces.is_empty() { project.archived = w.archived; }
                    else if project.agents.iter().all(|a| *a == crate::effective_skills::AgentKind::Proma) { project.archived &= w.archived; }
                    if !project.agents.contains(&crate::effective_skills::AgentKind::Proma) { project.agents.push(crate::effective_skills::AgentKind::Proma); }
                    if !project.proma_workspaces.contains(&w.path) { project.proma_workspaces.push(w.path.clone()); }
                } else {
                    projects.push(ProjectEntry { path: path.clone(), name: w.name.clone(), agents: vec![crate::effective_skills::AgentKind::Proma],
                        proma_workspaces: vec![w.path.clone()], archived: w.archived, hidden: false, retained: false });
                }
            }
        }
        projects.sort_by_key(|p| (!p.is_active(), p.name.to_lowercase(), p.path.clone()));
        projects.dedup_by(|a, b| a.path == b.path);
        let mut links = BTreeMap::new();
        for project in &projects {
            let mut entries = Vec::new();
            for target in agents::PROJECT_INSTALL_TARGETS {
                let root = agents::installation_root(&project.path, *target);
                let Ok(children) = fs::read_dir(&root) else { continue; };
                for entry in children.flatten() {
                    let path = entry.path();
                    if !path.join("SKILL.md").is_file() { continue; }
                    let key = proma_batch::source_key(&path.join("SKILL.md"));
                    if !skills.iter().any(|s| s.key == key) {
                        let name = crate::library::read_frontmatter(&path.join("SKILL.md")).ok()
                            .and_then(|(name, _)| name).unwrap_or_else(|| entry.file_name().to_string_lossy().into_owned());
                        skills.push(Skill { key: key.clone(), name, source: path.clone(), provider: "project", install_name: None });
                    }
                    entries.push(LinkEntry { target: *target, path, key });
                }
            }
            links.insert(project.path.clone(), entries);
        }
        skills.sort_by_key(|s| (s.name.to_lowercase(), s.key.clone()));
        Self { skills, projects, installed_agents, proma, proma_error, links }
    }

    pub fn workspace_ids(&self, project: &Path) -> Vec<String> {
        self.proma.as_ref().map(|c| c.workspaces.iter().filter(|w| w.workspace.project_root_path.as_deref().unwrap_or(&w.workspace.path) == project)
            .map(|w| w.workspace.id.clone()).collect()).unwrap_or_default()
    }

    pub fn plan(&self, action: Action, skills: &BTreeSet<String>, selected_agents: &BTreeSet<String>, projects: &BTreeSet<PathBuf>, workspaces: &BTreeSet<String>) -> Result<Plan> {
        ensure!(action.supported(selected_agents), "启用、停用目前仅支持 Proma；请选择支持的操作或仅选择 Proma");
        let selected = self.installed_agents.iter().filter(|a| selected_agents.contains(a.id)).copied().collect::<Vec<_>>();
        ensure!(selected.len() == selected_agents.len() && selected.iter().all(|a| !a.global_only), "Agent 未安装或不支持项目技能，请重新选择");
        let mut items = Vec::new();
        for project in self.projects.iter().filter(|p| projects.contains(&p.path)) {
            for agent in selected.iter().filter(|a| a.id != "proma") {
                for skill in self.skills.iter().filter(|s| skills.contains(&s.key)) {
                    if action == Action::Install {
                        let target = agents::INDEPENDENT_INSTALL_TARGETS.iter().find(|t| agent.owns_target(t.target))
                            .map(|t| t.target).unwrap_or(InstallTarget::Universal);
                        let name = skill.install_name.as_deref().unwrap_or(&skill.name);
                        let logical = agents::installation_root(&project.path, target).join(name);
                        let mut item = Item::new(skill, &project.name, &project.path, target, logical, self);
                        item.prepare_link(action, skill.install_name.is_some());
                        items.push(item);
                    } else {
                        let matching = self.links.get(&project.path).into_iter().flatten()
                            .filter(|e| e.key == skill.key && agent.supports_target(e.target)).collect::<Vec<_>>();
                        if matching.is_empty() {
                            let mut item = Item::new(skill, &project.name, &project.path, InstallTarget::Universal, project.path.clone(), self);
                            item.affected_agents = vec![*agent]; item.disposition = Disposition::Missing;
                            items.push(item);
                        }
                        for entry in matching {
                            let mut item = Item::new(skill, &project.name, &project.path, entry.target, entry.path.clone(), self);
                            item.prepare_link(action, skill.install_name.is_some());
                            items.push(item);
                        }
                    }
                }
            }
        }
        if selected_agents.contains("proma") {
            ensure!(self.proma_error.is_none(), "Proma 无法扫描：{}", self.proma_error.as_deref().unwrap_or_default());
            if let Some(catalog) = &self.proma {
                for entry in catalog.workspaces.iter().filter(|w| workspaces.contains(&w.workspace.id)) {
                    for skill in self.skills.iter().filter(|s| skills.contains(&s.key)) {
                        let w = &entry.workspace;
                        let mut item = Item::new(skill, &w.name, &w.path, InstallTarget::Proma, w.path.join("skills").join(skill.install_name.as_deref().unwrap_or(&skill.name)), self);
                        item.workspace = Some(w.clone());
                        item.operation = None;
                        let copies = entry.installations.iter().filter(|c| entry.identities.get(&c.path) == Some(&skill.key)).collect::<Vec<_>>();
                        if let Some(error) = &entry.error { item.block(Disposition::Unavailable, error); }
                        else if action.is_state() {
                            let plan = catalog.plan(&BTreeSet::from([skill.key.clone()]), &BTreeSet::from([w.id.clone()]), action == Action::Enable);
                            if let Some(first) = plan.items.first() {
                                item.disposition = match first.disposition {
                                    proma_batch::Disposition::Change => Disposition::Change,
                                    proma_batch::Disposition::Unchanged => Disposition::Unchanged,
                                    proma_batch::Disposition::Missing => Disposition::Missing,
                                    proma_batch::Disposition::Protected => Disposition::Protected,
                                    proma_batch::Disposition::Unavailable => Disposition::Unavailable,
                                };
                                item.reason = first.reason.clone();
                                if let Some(copy) = &first.installation { item.path = copy.path.clone(); }
                                item.operation = Some(Operation::Toggle(plan));
                            } else { item.disposition = Disposition::Missing; }
                        } else if copies.len() > 1 { item.block(Disposition::Protected, "工作区存在重复副本，请先检查"); }
                        else if let Some(copy) = copies.first() {
                            item.path = copy.path.clone();
                            if action == Action::Install { item.disposition = Disposition::Unchanged; }
                            else if copy.managed && copy.removable && skill.install_name.is_some() {
                                item.operation = Some(Operation::RemoveProma { root: catalog.root.clone(), workspace: w.clone(), copy: (*copy).clone() });
                                item.disposition = Disposition::Change;
                            } else { item.block(Disposition::Protected, "内置、集成、未托管或已修改的副本保留"); }
                        } else if action == Action::Remove { item.disposition = Disposition::Missing; }
                        else if skill.install_name.is_none() { item.block(Disposition::Protected, "请先将技能纳入技能库，再安装到其他项目"); }
                        else {
                            match proma::plan_install(&catalog.root, &w.path, &skill.source, skill.install_name.as_deref().unwrap()) {
                                Ok(plan) if plan.workspace.id != w.id || plan.workspace.slug != w.slug || plan.workspace.path != w.path =>
                                    item.block(Disposition::Protected, "工作区身份在扫描后发生变化，请刷新后重新选择"),
                                Ok(plan) if plan.action == "create" => { item.operation = Some(Operation::InstallProma(plan)); item.disposition = Disposition::Change; }
                                Ok(_) => item.block(Disposition::Protected, "安装位置已有其他来源，请刷新后检查"),
                                Err(error) => item.block(Disposition::Protected, &format!("{error:#}")),
                            }
                        }
                        items.push(item);
                    }
                }
            }
        }
        // Alias directories and shared targets produce one physical operation.
        let mut unique: Vec<Item> = Vec::new();
        for mut item in items {
            if matches!(item.operation, Some(Operation::Link { .. })) && self.proma.as_ref().is_some_and(|c| c.workspaces.iter().any(|w| {
                item.path.starts_with(w.workspace.path.join("skills")) || item.path.starts_with(w.workspace.path.join("skills-inactive"))
            })) {
                item.block(Disposition::Protected, "安装路径与 Proma 实体副本目录重叠，请通过 Proma 工作区管理");
            }
            if let Some(existing) = unique.iter_mut().find(|e| e.path == item.path && e.skill.key == item.skill.key) {
                if existing.operation_identity() != item.operation_identity() {
                    existing.block(Disposition::Protected, "同一安装路径涉及不同类型或工作区的操作，请分开选择");
                }
                for agent in item.affected_agents { if !existing.affected_agents.contains(&agent) { existing.affected_agents.push(agent); } }
                if !existing.location.contains(&item.location) { existing.location.push_str(&format!(" / {}", item.location)); }
            } else { unique.push(item); }
        }
        let mut conflicts = BTreeMap::<PathBuf, BTreeSet<String>>::new();
        for item in &unique { if item.disposition == Disposition::Change { conflicts.entry(item.path.clone()).or_default().insert(item.skill.key.clone()); } }
        for item in &mut unique {
            if conflicts.get(&item.path).is_some_and(|s| s.len() > 1) { item.block(Disposition::Protected, "所选同名技能来源不同，安装位置冲突，请分开选择"); }
            item.affected_agents.sort_by_key(|a| a.name.to_lowercase());
        }
        Ok(Plan { action, items: unique })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disposition { Change, Unchanged, Missing, Protected, Unavailable }

#[derive(Clone, Debug)]
enum Operation {
    Link { logical: PathBuf, expected: Option<directory_link::DirectoryLink>, identity: Option<String> },
    InstallProma(proma::InstallPlan),
    RemoveProma { root: PathBuf, workspace: proma::Workspace, copy: proma::Installation },
    Toggle(proma_batch::Plan),
}

#[derive(Clone, Debug)]
pub struct Item {
    pub skill: Skill,
    pub location: String,
    pub project: PathBuf,
    pub workspace: Option<proma::Workspace>,
    pub path: PathBuf,
    pub affected_agents: Vec<AgentIconInfo>,
    pub disposition: Disposition,
    pub reason: Option<String>,
    operation: Option<Operation>,
    anchor: Option<(PathBuf, String)>,
    project_identity: Option<String>,
}

impl Item {
    fn operation_identity(&self) -> (&'static str, Option<&str>) {
        match &self.operation {
            Some(Operation::Link { .. }) => ("link", None),
            Some(Operation::InstallProma(plan)) => ("proma-install", Some(&plan.workspace.id)),
            Some(Operation::RemoveProma { workspace, .. }) => ("proma-remove", Some(&workspace.id)),
            Some(Operation::Toggle(plan)) => ("proma-toggle", plan.items.first().map(|i| i.workspace.id.as_str())),
            None => ("protected", None),
        }
    }
    fn new(skill: &Skill, location: &str, project: &Path, target: InstallTarget, logical: PathBuf, catalog: &Catalog) -> Self {
        let physical = physical_entry(&logical).unwrap_or_else(|_| logical.clone());
        let mut affected_agents = catalog.installed_agents.iter().filter(|a| {
            if a.global_only { return false; }
            a.supports_target(target) || (target != InstallTarget::Proma && a.install_targets.iter().chain(a.shared_agents.then_some(&InstallTarget::Universal))
                .filter(|t| **t != InstallTarget::Proma).any(|t| physical_entry(&agents::installation_root(project, *t).join(logical.file_name().unwrap_or_default())).ok().as_ref() == Some(&physical)))
        }).copied().collect::<Vec<_>>();
        affected_agents.sort_by_key(|a| a.name.to_lowercase());
        Self { skill: skill.clone(), location: location.into(), project: project.to_path_buf(), workspace: None,
            path: physical, affected_agents, disposition: Disposition::Protected, reason: None,
            operation: Some(Operation::Link { logical, expected: None, identity: None }), anchor: None, project_identity: None }
    }

    fn block(&mut self, disposition: Disposition, reason: &str) {
        self.disposition = disposition; self.reason = Some(reason.into()); self.operation = None;
    }

    fn prepare_link(&mut self, action: Action, trusted: bool) {
        let result = (|| -> Result<()> {
            let logical = match &self.operation { Some(Operation::Link { logical, .. }) => logical.clone(), _ => unreachable!() };
            crate::library::validate_name(self.skill.install_name.as_deref().unwrap_or(&self.skill.name))?;
            ensure!(self.project.is_dir() && dirs::home_dir().is_none_or(|home| home.canonicalize().ok() != self.project.canonicalize().ok()), "请选择实际项目，全局安装不在此操作");
            self.path = physical_entry(&logical)?;
            self.project_identity = Some(entry_identity(&self.project.canonicalize()?)?);
            let mut anchor = self.path.parent().context("缺少安装父目录")?;
            while !anchor.exists() { anchor = anchor.parent().context("缺少项目目录")?; }
            self.anchor = Some((anchor.to_path_buf(), entry_identity(anchor)?));
            ensure!(trusted, "仅管理技能库中的安装链接；普通项目技能源文件保留");
            ensure!(self.skill.source.join("SKILL.md").canonicalize()? == Path::new(self.skill.key.strip_prefix("file:").context("技能源无效")?), "技能源与预览不一致");
            ensure!(!self.path.starts_with(self.skill.source.canonicalize()?), "安装位置位于技能源目录，已保护");
            let metadata = match self.path.symlink_metadata() {
                Ok(metadata) => Some(metadata),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(error.into()),
            };
            if metadata.is_none() {
                self.disposition = if action == Action::Install { Disposition::Change } else { Disposition::Missing };
                return Ok(());
            }
            let link = directory_link::inspect(&self.path)?.context("安装位置是实体目录，已保留现有文件")?;
            ensure!(self.path.canonicalize()? == self.skill.source.canonicalize()?, "同名安装指向其他来源，已保留");
            self.operation = Some(Operation::Link { logical, expected: Some(link), identity: Some(entry_identity(&self.path)?) });
            self.disposition = if action == Action::Install { Disposition::Unchanged } else { Disposition::Change };
            Ok(())
        })();
        if let Err(error) = result { self.block(Disposition::Protected, &format!("{error:#}")); }
    }

    fn apply(&self, action: Action) -> Result<bool> {
        if self.disposition != Disposition::Change { return Ok(false); }
        ensure!(self.skill.install_name.is_none() || self.skill.source.join("SKILL.md").canonicalize()? == Path::new(self.skill.key.strip_prefix("file:").context("技能源身份无效")?), "技能源实际位置已变化，请刷新后检查");
        match self.operation.as_ref().context("缺少预览操作")? {
            Operation::InstallProma(plan) => { proma::apply_plan(plan)?; }
            Operation::RemoveProma { root, workspace, copy } => {
                let file = Path::new(self.skill.key.strip_prefix("file:").context("缺少受管来源")?);
                proma::uninstall_previewed_in(root, workspace, copy, file)?;
            }
            Operation::Toggle(plan) => {
                let result = plan.execute();
                if let Some(error) = result.outcomes.iter().find_map(|o| o.error.as_ref()) { bail!("{error}"); }
                return Ok(result.changed() > 0);
            }
            Operation::Link { logical, expected, identity } => {
                ensure!(self.project_identity.as_deref() == Some(entry_identity(&self.project.canonicalize()?)?.as_str()), "项目目录身份已变化，请重新预览");
                let (anchor, anchor_identity) = self.anchor.as_ref().context("缺少安装目录预览身份")?;
                ensure!(&entry_identity(anchor)? == anchor_identity, "安装父目录身份已变化，请重新预览");
                ensure!(physical_entry(logical)? == self.path, "项目安装目录实际位置已变化，请重新预览");
                if action == Action::Install {
                    ensure!(expected.is_none() && self.path.symlink_metadata().is_err(), "安装位置已变化，请刷新后检查");
                    install_link(&self.path, &self.skill.source.canonicalize()?)?;
                } else {
                    ensure!(directory_link::inspect(&self.path)?.as_ref() == expected.as_ref()
                        && identity.as_deref() == Some(entry_identity(&self.path)?.as_str())
                        && self.path.canonicalize()? == self.skill.source.canonicalize()?, "安装链接与预览不一致，已保留");
                    remove_link(&self.path, expected.as_ref().context("缺少预览链接")?, identity.as_deref().context("缺少链接身份")?)?;
                }
            }
        }
        Ok(true)
    }
}

fn entry_identity(path: &Path) -> Result<String> {
    let metadata = path.symlink_metadata()?;
    #[cfg(unix)] {
        use std::os::unix::fs::MetadataExt;
        Ok(format!("{}:{}", metadata.dev(), metadata.ino()))
    }
    #[cfg(not(unix))] { Ok(format!("{:?}:{:?}:{}", metadata.created()?, metadata.modified()?, metadata.len())) }
}

fn install_link(path: &Path, source: &Path) -> Result<()> {
    #[cfg(unix)] {
        let parent = path.parent().context("缺少父目录")?;
        let mut prefix = parent;
        let mut missing = Vec::new();
        while !prefix.exists() {
            missing.push(prefix.file_name().context("缺少目录名称")?.to_owned());
            prefix = prefix.parent().context("缺少项目目录")?;
        }
        let mut directory = crate::package_fs::Directory::open_anchored(prefix)?;
        for part in missing.into_iter().rev() { directory.mkdir(Path::new(&part))?; directory = directory.child(Path::new(&part))?; }
        directory.verify_location(parent)?;
        directory.symlink(Path::new(path.file_name().context("缺少技能名称")?), source.to_str().context("技能源路径不是UTF-8")?, true)?;
    }
    #[cfg(not(unix))] { fs::create_dir_all(path.parent().context("缺少父目录")?)?; directory_link::create(source, path)?; }
    Ok(())
}

/// Move aside first; a replaced entry is restored or retained, never deleted.
fn remove_link(path: &Path, expected: &directory_link::DirectoryLink, identity: &str) -> Result<()> {
    let parent = path.parent().context("缺少父目录")?;
    let backup = tempfile::Builder::new().prefix(".kitter-removed-").tempdir_in(parent)?.keep();
    let moved = backup.join("installation");
    let raw_link = fs::read_link(path).ok();
    #[cfg(test)]
    BEFORE_RECOVERY_NOTE.with(|hook| { if let Some(callback) = hook.borrow_mut().take() { callback(&backup); } });
    // Prepare recovery metadata before the mutation. Failure leaves the install in place.
    fs::write(backup.join("restore.json"), serde_json::to_vec_pretty(&serde_json::json!({"original":path,"entry":"installation","raw_target":raw_link}))?)?;
    crate::package_fs::rename_exclusive(path, &moved)?;
    let same_link = match &raw_link {
        Some(raw) => fs::read_link(&moved).is_ok_and(|current| &current == raw)
            && directory_link::inspect(&moved)?.is_some_and(|link| link.kind == expected.kind),
        None => directory_link::inspect(&moved)?.as_ref() == Some(expected),
    };
    if !same_link || entry_identity(&moved)? != identity {
        let restored = crate::package_fs::rename_exclusive(&moved, path);
        bail!("安装对象在执行时变化；{}", if restored.is_ok() { "已恢复原位置".into() } else { format!("保留于 {}", moved.display()) });
    }
    // Keep the detached link and a recovery note; the source package is untouched.
    Ok(())
}

#[cfg(test)]
thread_local! {
    static BEFORE_RECOVERY_NOTE: std::cell::RefCell<Option<Box<dyn FnOnce(&Path)>>> = const { std::cell::RefCell::new(None) };
}

#[derive(Clone, Debug)]
pub struct Plan { pub action: Action, pub items: Vec<Item> }
impl Plan {
    pub fn count(&self, disposition: Disposition) -> usize { self.items.iter().filter(|i| i.disposition == disposition).count() }
    pub fn execute(&self) -> BatchResult {
        BatchResult { action: self.action, outcomes: self.items.iter().map(|item| {
            match item.apply(self.action) {
                Ok(changed) => Outcome { item: item.clone(), changed, error: None },
                Err(error) => Outcome { item: item.clone(), changed: false, error: Some(format!("{error:#}")) },
            }
        }).collect() }
    }
}

#[derive(Clone, Debug)]
pub struct Outcome { pub item: Item, pub changed: bool, pub error: Option<String> }
#[derive(Clone, Debug)]
pub struct BatchResult { pub action: Action, pub outcomes: Vec<Outcome> }
impl BatchResult {
    pub fn changed(&self) -> usize { self.outcomes.iter().filter(|o| o.changed).count() }
    pub fn failed(&self) -> usize { self.outcomes.iter().filter(|o| o.error.is_some()).count() }
    pub fn retry_plan(&self) -> Plan { Plan { action: self.action, items: self.outcomes.iter().filter(|o| o.error.is_some()).map(|o| o.item.clone()).collect() } }
    pub fn merge_retry(&mut self, retry: Self) {
        for outcome in retry.outcomes {
            if let Some(old) = self.outcomes.iter_mut().find(|o| o.item.path == outcome.item.path && o.item.skill.key == outcome.item.skill.key) { *old = outcome; }
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    struct Fixture { _temp: tempfile::TempDir, base: PathBuf, source: PathBuf, projects: Vec<ProjectEntry>, root: PathBuf }
    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap(); let base = temp.path().canonicalize().unwrap();
            let source = base.join("library/alpha"); fs::create_dir_all(&source).unwrap();
            fs::write(source.join("SKILL.md"), "---\nname: alpha\ndescription: Test\n---\nKeep source\n").unwrap();
            let projects = ["Project A", "Project B"].into_iter().map(|name| {
                let path = base.join(name); fs::create_dir(&path).unwrap();
                ProjectEntry { name: name.into(), path, agents: vec![], proma_workspaces: vec![], archived: false, hidden: false, retained: false }
            }).collect::<Vec<_>>();
            let root = base.join("proma");
            for slug in ["a", "b", "c"] { fs::create_dir_all(root.join("agent-workspaces").join(slug)).unwrap(); }
            fs::write(root.join("agent-workspaces.json"), serde_json::to_vec(&serde_json::json!({"version":2,"workspaces":[
                {"id":"a","slug":"a","name":"Workspace A","projectRootPath":projects[0].path},
                {"id":"b","slug":"b","name":"Workspace B","projectRootPath":projects[0].path},
                {"id":"c","slug":"c","name":"Workspace C","projectRootPath":projects[1].path}
            ]})).unwrap()).unwrap();
            Self { _temp: temp, base, source, projects, root }
        }
        fn skill(&self) -> Skill { Skill { key: proma_batch::source_key(&self.source.join("SKILL.md")), name:"alpha".into(), source:self.source.clone(), provider:"library", install_name:Some("alpha".into()) } }
        fn catalog(&self) -> Catalog {
            let installed = agents::AGENT_ICON_ORDER.iter().filter(|a| ["codex","cursor","proma","dimagent","trae"].contains(&a.id)).copied().collect();
            Catalog::scan(vec![self.skill()], self.projects.clone(), installed, Some(&self.root))
        }
        fn plan(&self, catalog: &Catalog, action: Action, ids: &[&str], workspaces: &[&str]) -> Plan {
            catalog.plan(action, &BTreeSet::from([self.skill().key]), &ids.iter().map(|id| id.to_string()).collect(),
                &self.projects.iter().map(|p| p.path.clone()).collect(), &workspaces.iter().map(|id| id.to_string()).collect()).unwrap()
        }
    }

    #[test]
    fn multi_agent_install_remove_preserves_source_and_only_explicit_workspaces() {
        let f = Fixture::new(); let catalog = f.catalog();
        let install = f.plan(&catalog, Action::Install, &["codex","proma"], &["b"]);
        assert_eq!(install.count(Disposition::Change), 3);
        let first = install.execute(); assert_eq!((first.changed(), first.failed()), (3, 0));
        let repeated = install.execute(); assert_eq!((repeated.changed(), repeated.failed()), (0, 3));
    }

    #[test]
    fn installation_roundtrip_counts_physical_entries_and_retains_sources() {
        let f = Fixture::new();
        let install = f.plan(&f.catalog(), Action::Install, &["codex","proma"], &["b"]);
        let result = install.execute(); assert_eq!((result.changed(), result.failed()), (3, 0));
        assert!(f.projects[0].path.join(".codex/skills/alpha").is_symlink());
        assert!(f.root.join("agent-workspaces/b/skills/alpha/SKILL.md").is_file());
        assert!(!f.root.join("agent-workspaces/a/skills/alpha").exists());
        let remove = f.plan(&f.catalog(), Action::Remove, &["codex","proma"], &["b"]);
        let result = remove.execute(); assert_eq!((result.changed(), result.failed()), (3, 0));
        assert!(f.source.join("SKILL.md").is_file());
        assert!(!f.projects[0].path.join(".codex/skills/alpha").exists());
        assert!(f.projects[0].path.join(".codex/skills").read_dir().unwrap().any(|p| p.unwrap().file_name().to_string_lossy().starts_with(".kitter-removed-")));
    }

    #[test]
    fn shared_targets_deduplicate_and_disclose_other_installed_readers() {
        let f = Fixture::new();
        let plan = f.plan(&f.catalog(), Action::Install, &["dimagent","trae"], &[]);
        assert_eq!(plan.count(Disposition::Change), 2);
        assert!(plan.items.iter().all(|i| i.affected_agents.iter().any(|a| a.id == "codex")));
        let result = plan.execute(); assert_eq!((result.changed(), result.failed()), (2, 0));
        let remove = f.plan(&f.catalog(), Action::Remove, &["codex","dimagent","trae"], &[]);
        assert_eq!(remove.count(Disposition::Change), 2);
        assert_eq!(remove.execute().changed(), 2);
    }

    #[test]
    fn mixed_agents_cannot_toggle_and_single_proma_does_not_install_missing() {
        let f = Fixture::new(); let catalog = f.catalog();
        assert!(!Action::Disable.supported(&BTreeSet::from(["codex".into(),"proma".into()])));
        assert!(catalog.plan(Action::Disable, &BTreeSet::new(), &BTreeSet::from(["codex".into(),"proma".into()]), &BTreeSet::new(), &BTreeSet::new()).is_err());
        let plan = f.plan(&catalog, Action::Disable, &["proma"], &["a","b"]);
        assert_eq!(plan.count(Disposition::Missing), 2);
        assert_eq!(plan.execute().changed(), 0);
    }

    #[test]
    fn entity_directories_and_same_name_other_sources_are_protected() {
        let f = Fixture::new();
        let path = f.projects[0].path.join(".codex/skills/alpha"); fs::create_dir_all(&path).unwrap();
        fs::write(path.join("SKILL.md"), "project source, keep").unwrap();
        let plan = f.plan(&f.catalog(), Action::Install, &["codex"], &[]);
        assert_eq!(plan.count(Disposition::Protected), 1);
        assert_eq!(plan.execute().changed(), 1);
        assert_eq!(fs::read_to_string(path.join("SKILL.md")).unwrap(), "project source, keep");
        let alias = f.base.join("other"); fs::create_dir(&alias).unwrap();
        let link = f.projects[0].path.join(".cursor/skills/alpha"); fs::create_dir_all(link.parent().unwrap()).unwrap(); symlink(alias, &link).unwrap();
        assert_eq!(f.plan(&f.catalog(), Action::Install, &["cursor"], &[]).count(Disposition::Protected), 1);
    }

    #[test]
    fn same_name_different_sources_are_not_merged_or_overwritten() {
        let f = Fixture::new(); let mut other = f.skill(); other.source = f.base.join("other/alpha");
        fs::create_dir_all(&other.source).unwrap(); fs::write(other.source.join("SKILL.md"), "other source").unwrap();
        other.key = proma_batch::source_key(&other.source.join("SKILL.md"));
        let mut catalog = f.catalog(); catalog.skills.push(other.clone());
        let plan = catalog.plan(Action::Install, &BTreeSet::from([f.skill().key, other.key]), &BTreeSet::from(["codex".into()]),
            &BTreeSet::from([f.projects[0].path.clone()]), &BTreeSet::new()).unwrap();
        assert_eq!(plan.count(Disposition::Protected), 2); assert_eq!(plan.execute().changed(), 0);
    }

    #[test]
    fn removal_refuses_replaced_link_and_retry_keeps_other_successes() {
        let f = Fixture::new(); f.plan(&f.catalog(), Action::Install, &["codex"], &[]).execute();
        let plan = f.plan(&f.catalog(), Action::Remove, &["codex"], &[]);
        let path = f.projects[0].path.join(".codex/skills/alpha");
        fs::remove_file(&path).unwrap(); fs::create_dir(&path).unwrap(); fs::write(path.join("keep"), "keep").unwrap();
        let mut result = plan.execute(); assert_eq!((result.changed(), result.failed()), (1, 1));
        assert_eq!(result.retry_plan().items.len(), 1); assert!(path.join("keep").is_file());
        result.merge_retry(result.retry_plan().execute());
        assert_eq!((result.changed(), result.failed()), (1, 1)); assert!(f.source.join("SKILL.md").is_file());
    }

    #[test]
    fn redirected_target_parent_and_source_are_rejected_after_preview() {
        let f = Fixture::new(); let plan = f.plan(&f.catalog(), Action::Install, &["codex"], &[]);
        let other = f.base.join("unrelated"); fs::create_dir(&other).unwrap();
        symlink(&other, f.projects[0].path.join(".codex")).unwrap();
        let result = plan.execute(); assert_eq!((result.changed(), result.failed()), (1, 1));
        assert!(!other.join("skills/alpha").exists());
        let second = Fixture::new(); let plan = second.plan(&second.catalog(), Action::Install, &["codex"], &[]);
        fs::rename(&second.source, second.base.join("original")).unwrap(); symlink(&other, &second.source).unwrap();
        let result = plan.execute(); assert_eq!(result.failed(), 2);
    }

    #[test]
    fn alias_directories_remove_once_and_disclose_compatibility_agents() {
        let f = Fixture::new(); let p = &f.projects[0].path;
        fs::create_dir_all(p.join(".agents/skills")).unwrap(); fs::create_dir(p.join(".codex")).unwrap();
        symlink(p.join(".agents/skills"), p.join(".codex/skills")).unwrap();
        symlink(&f.source, p.join(".agents/skills/alpha")).unwrap();
        let plan = f.plan(&f.catalog(), Action::Remove, &["codex","cursor","dimagent"], &[]);
        assert_eq!(plan.count(Disposition::Change), 1);
        let item = plan.items.iter().find(|i| i.disposition == Disposition::Change).unwrap();
        assert!(item.affected_agents.iter().any(|a| a.id == "dimagent"));
        assert_eq!(plan.execute().changed(), 1); assert!(f.source.join("SKILL.md").is_file());
    }

    #[test]
    fn proma_id_changed_after_catalog_is_not_adopted_by_install_plan() {
        let f = Fixture::new(); let catalog = f.catalog();
        let index = f.root.join("agent-workspaces.json");
        let mut json: serde_json::Value = serde_json::from_slice(&fs::read(&index).unwrap()).unwrap();
        json["workspaces"][0]["id"] = "changed-id".into();
        fs::write(index, serde_json::to_vec(&json).unwrap()).unwrap();
        let plan = f.plan(&catalog, Action::Install, &["proma"], &["a"]);
        assert_eq!(plan.count(Disposition::Protected), 1); assert_eq!(plan.execute().changed(), 0);
        assert!(!f.root.join("agent-workspaces/a/skills/alpha").exists());
    }

    #[test]
    fn regular_agent_alias_into_proma_cannot_bypass_entity_transaction() {
        let f = Fixture::new(); let project = &f.projects[0].path;
        let native = f.root.join("agent-workspaces/a/skills"); fs::create_dir(&native).unwrap();
        fs::create_dir(project.join(".codex")).unwrap(); symlink(&native, project.join(".codex/skills")).unwrap();
        for ids in [vec!["codex"], vec!["codex","proma"]] {
            let plan = f.catalog().plan(Action::Install, &BTreeSet::from([f.skill().key]), &ids.into_iter().map(str::to_string).collect(),
                &BTreeSet::from([project.clone()]), &BTreeSet::from(["a".into()])).unwrap();
            assert_eq!(plan.count(Disposition::Change), 0); assert_eq!(plan.execute().changed(), 0);
            assert!(!native.join("alpha").exists());
        }
    }

    #[test]
    fn relative_link_removal_preserves_raw_target_and_source() {
        let f = Fixture::new(); let p = &f.projects[0].path;
        fs::create_dir_all(p.join(".codex/skills")).unwrap();
        symlink("../../../library/alpha", p.join(".codex/skills/alpha")).unwrap();
        let plan = f.plan(&f.catalog(), Action::Remove, &["codex"], &[]);
        assert_eq!(plan.count(Disposition::Change), 1);
        let result = plan.execute(); assert_eq!((result.changed(), result.failed()), (1, 0));
        assert!(!p.join(".codex/skills/alpha").exists()); assert!(f.source.join("SKILL.md").is_file());
    }

    #[test]
    fn recovery_note_failure_leaves_link_for_safe_retry() {
        let f = Fixture::new(); f.plan(&f.catalog(), Action::Install, &["codex"], &[]).execute();
        let plan = f.plan(&f.catalog(), Action::Remove, &["codex"], &[]);
        BEFORE_RECOVERY_NOTE.with(|hook| *hook.borrow_mut() = Some(Box::new(|backup| {
            fs::create_dir(backup.join("restore.json")).unwrap();
        })));
        let mut result = plan.execute(); assert_eq!((result.changed(), result.failed()), (1, 1));
        assert!(f.projects[0].path.join(".codex/skills/alpha").is_symlink());
        result.merge_retry(result.retry_plan().execute()); assert_eq!((result.changed(), result.failed()), (2, 0));
        assert!(f.source.join("SKILL.md").is_file());
    }

    #[test]
    fn replaced_project_directory_at_same_path_invalidates_plan() {
        let f = Fixture::new(); let plan = f.plan(&f.catalog(), Action::Install, &["codex"], &[]);
        let p = &f.projects[0].path; fs::rename(p, f.base.join("retained-project")).unwrap(); fs::create_dir(p).unwrap();
        let result = plan.execute(); assert_eq!((result.changed(), result.failed()), (1, 1));
        assert!(!p.join(".codex/skills/alpha").exists());
    }

    #[test]
    fn proma_removal_refuses_changed_native_identity_or_marker_source() {
        for replace_id in [true, false] {
            let f = Fixture::new();
            let install = f.plan(&f.catalog(), Action::Install, &["proma"], &["a"]).execute(); assert_eq!(install.changed(), 1);
            let plan = f.plan(&f.catalog(), Action::Remove, &["proma"], &["a"]);
            let path = f.root.join("agent-workspaces/a/skills/alpha");
            let marker_path = path.join(".kitter-proma.json");
            let mut marker: serde_json::Value = serde_json::from_slice(&fs::read(&marker_path).unwrap()).unwrap();
            if replace_id {
                let index_path = f.root.join("agent-workspaces.json");
                let mut index: serde_json::Value = serde_json::from_slice(&fs::read(&index_path).unwrap()).unwrap();
                index["workspaces"][0]["id"] = "replacement".into(); marker["workspace_id"] = "replacement".into();
                fs::write(index_path, serde_json::to_vec(&index).unwrap()).unwrap();
            } else {
                let other = f.base.join("other-alpha"); fs::create_dir(&other).unwrap();
                fs::copy(f.source.join("SKILL.md"), other.join("SKILL.md")).unwrap(); marker["source"] = other.to_string_lossy().into_owned().into();
            }
            fs::write(marker_path, serde_json::to_vec(&marker).unwrap()).unwrap();
            let result = plan.execute(); assert_eq!((result.changed(), result.failed()), (0, 1));
            assert!(path.join("SKILL.md").is_file()); assert!(f.source.join("SKILL.md").is_file());
        }
    }

    #[test]
    fn manual_project_rows_keep_native_archive_state_and_explicit_restore() {
        let mut f = Fixture::new(); let path = f.root.join("agent-workspaces.json");
        let mut index: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        index["workspaces"][2]["archived"] = true.into(); fs::write(path, serde_json::to_vec(&index).unwrap()).unwrap();
        let catalog = f.catalog();
        assert!(!catalog.projects.iter().find(|p| p.path == f.projects[1].path).unwrap().is_active());
        assert!(catalog.projects.iter().find(|p| p.path == f.projects[0].path).unwrap().is_active());
        f.projects[1].retained = true;
        assert!(f.catalog().projects.iter().find(|p| p.path == f.projects[1].path).unwrap().is_active());
    }
}
