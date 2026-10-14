//! Proma workspace installations. Skill entries are real packages; local data
//! remains at its existing source and is never copied into a workspace.
use crate::{
    package_fs::Directory,
    update_protection::{Manifest, Node},
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
};

const MARKER: &str = ".kitter-proma.json";
const RECOVERY: &str = ".kitter-proma-recovery";
const LIMIT: u64 = 16 * 1024 * 1024;

#[derive(Clone, Debug, Eq, Serialize, Deserialize)]
pub struct Workspace {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub path: PathBuf,
    #[serde(default)]
    pub project_root_path: Option<PathBuf>,
}

impl PartialEq for Workspace {
    fn eq(&self, other: &Self) -> bool {
        // Finder associations are display metadata, not transaction identity.
        // Recovery records written before this field existed remain valid.
        self.id == other.id && self.slug == other.slug
            && self.name == other.name && self.path == other.path
    }
}

#[derive(Deserialize)]
struct Index {
    version: u64,
    workspaces: Vec<IndexedWorkspace>,
}
#[derive(Deserialize)]
struct IndexedWorkspace {
    id: String,
    slug: String,
    name: String,
    #[serde(default, rename = "projectRootPath")]
    project_root_path: Option<PathBuf>,
}

pub fn home() -> Option<PathBuf> {
    std::env::var_os("KITTER_PROMA_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|p| p.join(".proma")))
}

fn real_directory(path: &Path) -> Result<()> {
    let m = fs::symlink_metadata(path).context("无法检查 Proma 实体目录")?;
    if !m.is_dir() || m.file_type().is_symlink() {
        bail!("Proma 目录必须是实体目录：{}", path.display());
    }
    Ok(())
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let parent = Directory::open_anchored(path.parent().context("文件缺少父目录")?)?;
    let mut bytes = Vec::new();
    parent
        .read(Path::new(path.file_name().context("文件缺少名称")?))?
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > LIMIT {
        bail!("Proma 元数据过大");
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("Proma 元数据格式无效：{}", path.display()))
}

pub fn workspaces(root: &Path) -> Result<Vec<Workspace>> {
    if !root.is_absolute() {
        bail!("Proma 配置根必须是绝对路径");
    }
    if !root.exists() {
        return Ok(vec![]);
    }
    real_directory(root)?;
    let index_path = root.join("agent-workspaces.json");
    if !index_path.exists() {
        return Ok(vec![]);
    }
    let index: Index = read_json(&index_path)?;
    if !matches!(index.version, 1 | 2) || index.workspaces.len() > 1000 {
        bail!("不支持的 Proma 工作区索引");
    }
    let root = root.canonicalize()?;
    let parent = root.join("agent-workspaces");
    real_directory(&parent)?;
    let mut ids = BTreeSet::new();
    let mut slugs = BTreeSet::new();
    let mut out = vec![];
    for entry in index.workspaces {
        crate::library::validate_name(&entry.slug)?;
        if entry.id.trim().is_empty()
            || entry.name.trim().is_empty()
            || !ids.insert(entry.id.clone())
            || !slugs.insert(entry.slug.to_lowercase())
        {
            bail!("Proma 工作区身份无效或重复");
        }
        let path = parent.join(&entry.slug);
        // Discovery does not initialize missing workspaces.
        if !path.exists() {
            continue;
        }
        real_directory(&path)?;
        let project_root_path = entry.project_root_path
            .filter(|project| project.is_absolute() && project.is_dir())
            .and_then(|project| project.canonicalize().ok());
        out.push(Workspace {
            id: entry.id,
            slug: entry.slug,
            name: entry.name,
            path,
            project_root_path,
        });
    }
    Ok(out)
}

/// Display identity may be a Finder project; all Skill operations retain the
/// registered native workspace path returned here.
pub fn workspaces_for_project(root: &Path, project: &Path) -> Result<Vec<Workspace>> {
    if !project.is_absolute() || !project.is_dir() {
        return Ok(Vec::new());
    }
    let identity = project.canonicalize()?;
    let workspaces = workspaces(root)?;
    if let Some(native) = workspaces.iter().find(|workspace| workspace.path == identity) {
        return Ok(vec![native.clone()]);
    }
    Ok(workspaces.into_iter()
        .filter(|workspace| workspace.project_root_path.as_ref() == Some(&identity))
        .collect())
}

pub fn project_workspaces(project: &Path) -> Result<Vec<Workspace>> {
    match home() {
        Some(root) => workspaces_for_project(&root, project),
        None => Ok(Vec::new()),
    }
}

/// Mutations must select exactly one workspace. Never fan out an install or
/// target-specific uninstall when several workspaces share a Finder project.
pub fn workspace_for_project(root: &Path, project: &Path) -> Result<Workspace> {
    let mut matches = workspaces_for_project(root, project)?;
    match matches.len() {
        1 => Ok(matches.remove(0)),
        0 => bail!("项目没有关联已登记的 Proma 原生工作区"),
        _ => bail!("项目关联多个 Proma 工作区，请指定一个原生工作区目录"),
    }
}

pub fn installation_workspace(project: &Path) -> Result<Workspace> {
    let root = home().context("无法定位 Proma 配置根")?;
    workspace_for_project(&root, project)
}

pub fn workspace_for_path(root: &Path, project: &Path) -> Result<Workspace> {
    real_directory(project)?;
    let identity = project.canonicalize()?;
    workspaces(root)?
        .into_iter()
        .find(|w| w.path == identity)
        .context("目录不是已注册的 Proma 原生工作区")
}

pub fn is_workspace(project: &Path) -> bool {
    home().is_some_and(|root| workspace_for_path(&root, project).is_ok())
}

fn configured_workspace(project: &Path) -> Result<(PathBuf, Workspace)> {
    let root = home().context("无法定位 Proma 配置根")?;
    let w = workspace_for_path(&root, project)?;
    Ok((root.canonicalize()?, w))
}

fn path_workspace(path: &Path) -> Result<(PathBuf, Workspace)> {
    let parent = path.parent().context("安装位置缺少父目录")?;
    if !matches!(
        parent.file_name().and_then(|x| x.to_str()),
        Some("skills" | "skills-inactive")
    ) {
        bail!("不是 Proma 技能安装位置");
    }
    let project = parent.parent().context("安装位置缺少工作区")?;
    configured_workspace(project)
}

fn signature(path: &Path) -> Result<String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let m = fs::symlink_metadata(path)?;
        Ok(format!("{}:{}", m.dev(), m.ino()))
    }
    #[cfg(not(unix))]
    {
        Ok(format!("{:?}", fs::symlink_metadata(path)?.created()?))
    }
}

fn hash<T: Serialize>(value: &T) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}

fn excluded(path: &str) -> bool {
    path.split('/').any(|component| {
        let lower = component.to_ascii_lowercase();
        let s = lower.as_str();
        matches!(
            s,
            "local-data"
                | ".git"
                | ".ds_store"
                | "node_modules"
                | ".venv"
                | "venv"
                | "__pycache__"
                | ".cache"
                | ".pytest_cache"
                | MARKER
        ) || s == ".env"
            || (s.starts_with(".env.")
                && !matches!(s, ".env.example" | ".env.sample" | ".env.template"))
    })
}

fn public_manifest(source: &Path) -> Result<Manifest> {
    let mut m = Directory::open_anchored(source)?.installation_snapshot()?;
    m.retain(|p, _| !excluded(p));
    // Structure-only parent packages need explicit materialization; never copy
    // a package link that could import an unrelated resource or private tree.
    if m.values().any(|n| matches!(n, Node::Symlink { .. })) {
        bail!("技能包含链接，需要先明确其资源范围；未复制内容");
    }
    if !matches!(m.get("SKILL.md"), Some(Node::File { .. })) {
        bail!("技能缺少实体 SKILL.md");
    }
    Ok(m)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Marker {
    schema: u32,
    workspace_id: String,
    workspace_slug: String,
    name: String,
    source: PathBuf,
    source_hash: String,
    content_hash: String,
    local_data: Option<PathBuf>,
}

fn marker(path: &Path, w: &Workspace) -> Result<Marker> {
    real_directory(path)?;
    let m: Marker = read_json(&path.join(MARKER))?;
    if m.schema != 1
        || m.workspace_id != w.id
        || m.workspace_slug != w.slug
        || path.file_name().and_then(|p| p.to_str()) != Some(&m.name)
        || !m.source.is_absolute()
    {
        bail!("Proma 安装标记与工作区身份不符");
    }
    Ok(m)
}

fn content_hash(path: &Path) -> Result<String> {
    let mut m = Directory::open_anchored(path)?.installation_snapshot()?;
    m.remove(MARKER);
    hash(&m)
}

fn verify_local_data(path: &Path, m: &Marker) -> Result<()> {
    let reference = path.join("local-data");
    match &m.local_data {
        Some(expected)
            if fs::symlink_metadata(&reference).is_ok_and(|v| v.file_type().is_symlink())
                && fs::read_link(&reference)? == *expected =>
        {
            Ok(())
        }
        None if fs::symlink_metadata(&reference)
            .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
        {
            Ok(())
        }
        _ => bail!("本机数据引用发生变化，保留现场"),
    }
}

fn clean_marker(path: &Path, w: &Workspace) -> Result<Marker> {
    let m = marker(path, w)?;
    if content_hash(path)? != m.content_hash {
        bail!("Proma 工作区副本已被修改，保留修改并拒绝覆盖");
    }
    verify_local_data(path, &m)?;
    Ok(m)
}

fn observed(path: &Path) -> Result<String> {
    let parent = match Directory::open_anchored(path.parent().context("安装位置缺少父目录")?)
    {
        Ok(parent) => parent,
        Err(error) if crate::package_fs::is_not_found(&error) => return Ok("absent".into()),
        Err(error) => return Err(error),
    };
    parent.observation(Path::new(path.file_name().context("安装位置缺少名称")?))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstallPlan {
    pub workspace: Workspace,
    pub name: String,
    pub source: PathBuf,
    pub destination: PathBuf,
    pub action: String,
    pub source_hash: String,
    pub local_data: Option<PathBuf>,
    root: PathBuf,
    workspace_signature: String,
    expected: String,
}

fn no_pending(w: &Workspace) -> Result<()> {
    let directory = w.path.join(RECOVERY);
    if !directory.exists() {
        return Ok(());
    }
    real_directory(&directory)?;
    for entry in fs::read_dir(directory)? {
        let path = entry?.path().join("transaction.json");
        if !path.exists() {
            continue;
        }
        let t: Transaction = read_json(&path)?;
        if !matches!(t.phase.as_str(), "committed" | "rolled_back") {
            bail!(
                "存在未完成 Proma 操作，请先执行 proma recover；恢复材料：{}",
                path.display()
            );
        }
    }
    Ok(())
}

pub fn plan_install(root: &Path, project: &Path, source: &Path, name: &str) -> Result<InstallPlan> {
    plan_install_at(root, project, source, name, "skills", false)
}

fn plan_install_at(
    root: &Path, project: &Path, source: &Path, name: &str,
    directory: &str, existing_only: bool,
) -> Result<InstallPlan> {
    crate::library::validate_name(name)?;
    if !matches!(directory, "skills" | "skills-inactive")
        || (directory == "skills-inactive" && !existing_only)
    {
        bail!("自动同步只允许更新原安装目录");
    }
    let workspace = workspace_for_path(root, project)?;
    no_pending(&workspace)?;
    let source = source.canonicalize().context("技能不存在")?;
    let source_hash = hash(&public_manifest(&source)?)?;
    let destination = workspace.path.join(directory).join(name);
    let other = if directory == "skills" { "skills-inactive" } else { "skills" };
    if fs::symlink_metadata(workspace.path.join(other).join(name)).is_ok() {
        bail!("技能启停位置已变化或重复；未改变工作区选择");
    }
    let action = match fs::symlink_metadata(&destination) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && !existing_only => "create",
        Err(e) => return Err(e.into()),
        Ok(m) if m.file_type().is_symlink() => {
            if existing_only {
                bail!("已有安装变成外部链接，未迁移或覆盖");
            }
            if destination.canonicalize().ok().as_ref() != Some(&source) {
                bail!("同名链接指向其他来源，未覆盖");
            }
            "migrate"
        }
        Ok(_) => {
            let m = clean_marker(&destination, &workspace).context("同名目录不能安全更新")?;
            if m.source != source {
                bail!("同名技能来源不同，未覆盖");
            }
            if m.source_hash == source_hash {
                "unchanged"
            } else {
                "update"
            }
        }
    }
    .to_string();
    let local_data_path = source.join("local-data");
    let local_data = match fs::symlink_metadata(&local_data_path) {
        Ok(m) if m.is_dir() && !m.file_type().is_symlink() => Some(local_data_path),
        Ok(_) => bail!("来源 local-data 必须是实体目录"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    #[cfg(test)]
    BEFORE_PLAN_OBSERVATION.with(|hook| {
        if let Some(callback) = hook.borrow_mut().take() { callback(); }
    });
    let expected = observed(&destination)?;
    if existing_only && expected == "absent" {
        bail!("已有安装已被移除，未重新创建");
    }
    Ok(InstallPlan {
        root: root.canonicalize()?,
        workspace_signature: signature(&workspace.path)?,
        expected,
        workspace,
        name: name.into(),
        source,
        destination,
        action,
        source_hash,
        local_data,
    })
}

fn adaptation(content: &str, source: &Path, local_data: bool) -> String {
    if !local_data {
        return content.into();
    }
    let bridge = format!(
        "\n\n## 本机适配与已配置资源\n\n执行前读取 `{}/local-data/ADAPTATION.md`（若存在），并应用其中与任务相关的本机约定。本机适配中的相对路径以该文件所在目录为准。当前技能的本机数据和已安装环境继续位于 `{}/local-data/`；本目录的 `local-data/` 引用同一位置。复用原配置与运行入口，不因技能安装副本重新初始化环境或重新要求用户配置凭据。不得展示凭据正文。\n\n",
        source.display(),
        source.display()
    );
    let text = content.trim_start_matches('\u{feff}');
    if text.starts_with("---\n") || text.starts_with("---\r\n") {
        let mut offset = 0;
        for (i, line) in text.split_inclusive('\n').enumerate() {
            offset += line.len();
            if i > 0 && line.trim() == "---" {
                return format!("{}{}{}", &text[..offset], bridge, &text[offset..]);
            }
        }
    }
    format!("{bridge}{text}")
}

fn copy_package(plan: &InstallPlan, output: &Directory) -> Result<Marker> {
    let incoming = public_manifest(&plan.source)?;
    if hash(&incoming)? != plan.source_hash {
        bail!("技能源已变化，请重新预览");
    }
    let input = Directory::open_anchored(&plan.source)?;
    for (path, node) in &incoming {
        match node {
            Node::Directory { .. } => output.mkdir(Path::new(path))?,
            Node::File { mode, .. } => {
                std::io::copy(
                    &mut input.read(Path::new(path))?,
                    &mut output.create_file(Path::new(path), *mode)?,
                )?;
            }
            Node::Symlink { .. } => bail!("技能包链接未被授权复制"),
        }
    }
    for (path, node) in incoming.iter().rev() {
        if let Node::Directory { mode } = node {
            output.chmod_directory(Path::new(path), *mode)?;
        }
    }
    if hash(&public_manifest(&plan.source)?)? != plan.source_hash
        || output.installation_snapshot()? != incoming
    {
        bail!("复制期间技能源变化，未安装");
    }
    let mut content = String::new();
    output
        .read(Path::new("SKILL.md"))?
        .take(LIMIT + 1)
        .read_to_string(&mut content)?;
    if content.len() as u64 > LIMIT {
        bail!("SKILL.md 过大");
    }
    output.write(
        Path::new("SKILL.md"),
        adaptation(&content, &plan.source, plan.local_data.is_some()).as_bytes(),
    )?;
    if let Some(local_data) = &plan.local_data {
        output.symlink(
            Path::new("local-data"),
            local_data.to_str().context("本机数据路径不是 UTF-8")?,
            true,
        )?;
    }
    let m = Marker {
        schema: 1,
        workspace_id: plan.workspace.id.clone(),
        workspace_slug: plan.workspace.slug.clone(),
        name: plan.name.clone(),
        source: plan.source.clone(),
        source_hash: plan.source_hash.clone(),
        content_hash: {
            let mut m = output.installation_snapshot()?;
            m.remove(MARKER);
            hash(&m)?
        },
        local_data: plan.local_data.clone(),
    };
    output.write(Path::new(MARKER), &serde_json::to_vec_pretty(&m)?)?;
    Ok(m)
}

#[derive(Serialize, Deserialize)]
struct Transaction {
    workspace: Workspace,
    workspace_signature: String,
    target: PathBuf,
    backup: Option<PathBuf>,
    before: String,
    after: String,
    phase: String,
}

fn recheck(plan: &InstallPlan) -> Result<()> {
    crate::library::validate_name(&plan.name)?;
    let inactive = plan.destination == plan.workspace.path.join("skills-inactive").join(&plan.name);
    if (matches!(plan.action.as_str(), "update" | "unchanged") && plan.expected == "absent")
        || (plan.destination != plan.workspace.path.join("skills").join(&plan.name)
        && !(inactive && plan.expected != "absent"
            && matches!(plan.action.as_str(), "update" | "unchanged")))
        || plan.source.canonicalize()? != plan.source
        || !matches!(
            plan.action.as_str(),
            "create" | "migrate" | "update" | "unchanged"
        )
        || plan
            .local_data
            .as_ref()
            .is_some_and(|p| *p != plan.source.join("local-data"))
    {
        bail!("Proma 安装计划路径或动作无效");
    }
    if let Some(path) = &plan.local_data {
        real_directory(path)?;
    }
    let other = if inactive { "skills" } else { "skills-inactive" };
    if fs::symlink_metadata(plan.workspace.path.join(other).join(&plan.name)).is_ok() {
        bail!("技能启停位置已变化，请重新预览");
    }
    if workspace_for_path(&plan.root, &plan.workspace.path)? != plan.workspace
        || signature(&plan.workspace.path)? != plan.workspace_signature
        || observed(&plan.destination)? != plan.expected
        || hash(&public_manifest(&plan.source)?)? != plan.source_hash
    {
        bail!("工作区、来源或安装位置已变化，请重新预览");
    }
    if plan.expected != "absent" {
        if fs::symlink_metadata(&plan.destination)?
            .file_type()
            .is_symlink()
        {
            if plan.action != "migrate" || plan.destination.canonicalize()? != plan.source {
                bail!("安装链接不属于计划来源");
            }
        } else if clean_marker(&plan.destination, &plan.workspace)?.source != plan.source {
            bail!("安装目录不属于计划来源");
        }
    }
    Ok(())
}

#[cfg(test)]
thread_local! {
    static BEFORE_PUBLISH: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = std::cell::RefCell::new(None);
    static BEFORE_STAGING: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = std::cell::RefCell::new(None);
    static BEFORE_ROLLBACK_MOVE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = std::cell::RefCell::new(None);
    static BEFORE_RECOVERY_MUTATION: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = std::cell::RefCell::new(None);
    static BEFORE_FOLLOWER_REFRESH: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = std::cell::RefCell::new(None);
    static BEFORE_PLAN_OBSERVATION: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = std::cell::RefCell::new(None);
    static BEFORE_TOGGLE_MOVE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = std::cell::RefCell::new(None);
    static AFTER_TOGGLE_MOVE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = std::cell::RefCell::new(None);
}

fn new_operation(
    workspace: &Directory,
    project: &Path,
    prefix: &str,
) -> Result<(PathBuf, Directory)> {
    workspace.mkdir(Path::new(RECOVERY))?;
    let recovery = workspace.child(Path::new(RECOVERY))?;
    #[cfg(test)]
    BEFORE_STAGING.with(|hook| {
        if let Some(callback) = hook.borrow_mut().take() {
            callback();
        }
    });
    workspace.verify_location(project)?;
    recovery.verify_location(&project.join(RECOVERY))?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let name = format!("{prefix}-{}-{stamp}", std::process::id());
    recovery.mkdir(Path::new(&name))?;
    let directory = recovery.child(Path::new(&name))?;
    let path = project.join(RECOVERY).join(name);
    directory.verify_location(&path)?;
    Ok((path, directory))
}

pub fn apply_plan(plan: &InstallPlan) -> Result<()> {
    let workspace = Directory::open_anchored(&plan.workspace.path)?;
    let _lock = workspace.lock(Path::new(".kitter-proma.lock"))?;
    recheck(plan)?;
    if workspace.identity()? != plan.workspace_signature {
        bail!("工作区目录身份已变化");
    }
    if plan.action == "unchanged" {
        return Ok(());
    }
    no_pending(&plan.workspace)?;
    let directory = plan.destination.parent().and_then(Path::file_name)
        .context("安装目录缺少名称")?;
    if directory == "skills" {
        workspace.mkdir(Path::new(directory))?;
    }
    let parent = workspace.child(Path::new(directory))?;
    let (operation, output) = new_operation(&workspace, &plan.workspace.path, "install")?;
    let mut t = Transaction {
        workspace: plan.workspace.clone(),
        workspace_signature: plan.workspace_signature.clone(),
        target: plan.destination.clone(),
        backup: (plan.expected != "absent").then(|| operation.join("previous")),
        before: plan.expected.clone(),
        after: "unpublished".into(),
        phase: "preparing".into(),
    };
    output.write(
        Path::new("transaction.json"),
        &serde_json::to_vec_pretty(&t)?,
    )?;
    output.mkdir(Path::new("staging"))?;
    let staged = output.child(Path::new("staging"))?;
    if let Err(error) = copy_package(plan, &staged) {
        t.phase = "rolled_back".into();
        output.write(
            Path::new("transaction.json"),
            &serde_json::to_vec_pretty(&t)?,
        )?;
        return Err(error);
    }
    t.after = observed(&operation.join("staging"))?;
    t.phase = "prepared".into();
    output.write(
        Path::new("transaction.json"),
        &serde_json::to_vec_pretty(&t)?,
    )?;
    recheck(plan)?;
    workspace.verify_location(&plan.workspace.path)?;
    parent.verify_location(&plan.workspace.path.join(directory))?;
    output.verify_location(&operation)?;
    #[cfg(test)]
    BEFORE_PUBLISH.with(|hook| {
        if let Some(callback) = hook.borrow_mut().take() {
            callback();
        }
    });
    let result = (|| -> Result<()> {
        if t.backup.is_some() {
            parent.rename_to(Path::new(&plan.name), &output, Path::new("previous"))?;
            if !output
                .observation(Path::new("previous"))
                .is_ok_and(|value| value == t.before)
            {
                // Preserve the actual concurrent object at its original location.
                output.rename_to(Path::new("previous"), &parent, Path::new(&plan.name))?;
                t.phase = "rolled_back".into();
                output.write(
                    Path::new("transaction.json"),
                    &serde_json::to_vec_pretty(&t)?,
                )?;
                bail!("被移走的原件已变化，已恢复原位置并停止安装");
            }
        }
        t.phase = "replacing".into();
        output.write(
            Path::new("transaction.json"),
            &serde_json::to_vec_pretty(&t)?,
        )?;
        output.rename_to(Path::new("staging"), &parent, Path::new(&plan.name))?;
        if observed(&plan.destination)? != t.after {
            bail!("发布对象身份或内容变化，保留现场");
        }
        clean_marker(&plan.destination, &plan.workspace)?;
        t.phase = "committed".into();
        output.write(
            Path::new("transaction.json"),
            &serde_json::to_vec_pretty(&t)?,
        )?;
        Ok(())
    })();
    if let Err(error) = result {
        if t.phase != "rolled_back" && restore_with_handles(&t, &output, &parent).is_ok() {
            t.phase = "rolled_back".into();
            output.write(
                Path::new("transaction.json"),
                &serde_json::to_vec_pretty(&t)?,
            )?;
        }
        bail!(
            "Proma 安装未完成：{error}；恢复材料：{}",
            operation.display()
        );
    }
    Ok(())
}

fn restore_with_handles(t: &Transaction, output: &Directory, parent: &Directory) -> Result<()> {
    let name = Path::new(t.target.file_name().context("目标缺少名称")?);
    let current = parent.observation(name)?;
    if current == t.before {
        return Ok(());
    }
    if t.backup.is_some() && output.observation(Path::new("previous"))? != t.before {
        bail!("恢复原件已变化，保留现场");
    }
    if current != "absent" && current != t.after {
        bail!("目标不属于本事务，保留现场");
    }
    #[cfg(test)]
    BEFORE_ROLLBACK_MOVE.with(|hook| {
        if let Some(callback) = hook.borrow_mut().take() {
            callback();
        }
    });
    if current != "absent" {
        parent.rename_to(name, output, Path::new("failed"))?;
        if !output
            .observation(Path::new("failed"))
            .is_ok_and(|value| value == t.after)
        {
            output.rename_to(Path::new("failed"), parent, name)?;
            bail!("回滚时目标已替换，已恢复原位置");
        }
    }
    if t.backup.is_some() {
        output.rename_to(Path::new("previous"), parent, name)?;
        if !parent
            .observation(name)
            .is_ok_and(|value| value == t.before)
        {
            parent.rename_to(name, output, Path::new("previous"))?;
            if current != "absent" {
                output.rename_to(Path::new("failed"), parent, name)?;
            }
            bail!("恢复时原件已替换，保留恢复前状态");
        }
    }
    Ok(())
}

#[cfg(test)]
fn restore_transaction(t: &Transaction, operation: &Path) -> Result<()> {
    restore_with_handles(
        t,
        &Directory::open_anchored(operation)?,
        &Directory::open_anchored(t.target.parent().context("目标缺少父目录")?)?,
    )
}

pub fn install(project: &Path, source: &Path, name: &str) -> Result<()> {
    let (root, _) = configured_workspace(project)?;
    apply_plan(&plan_install(&root, project, source, name)?)
}

/// Refresh existing active followers after the maintained source has committed.
/// No new installation is created and inactive workspace choices are retained.
pub(crate) fn sync_existing_source(source: &Path) -> Vec<String> {
    home()
        .map(|root| sync_existing_source_in(&root, source))
        .unwrap_or_default()
}

fn sync_existing_source_in(root: &Path, source: &Path) -> Vec<String> {
    let result = (|| -> Result<Vec<String>> {
        let source = source.canonicalize()?;
        let mut warnings = Vec::new();
        for workspace in workspaces(root)? {
            for installation in inspect_in(root, &workspace.path)? {
                if installation.state != "active"
                    || !installation.managed
                    || installation.source.as_ref() != Some(&source)
                {
                    continue;
                }
                #[cfg(test)]
                BEFORE_FOLLOWER_REFRESH.with(|hook| {
                    if let Some(callback) = hook.borrow_mut().take() {
                        callback();
                    }
                });
                if let Err(error) = plan_install(root, &workspace.path, &source, &installation.name)
                    .and_then(|plan| {
                        if !matches!(plan.action.as_str(), "update" | "unchanged") {
                            bail!("已有安装已移除或改变，未重新安装或迁移");
                        }
                        apply_plan(&plan)
                    })
                {
                    warnings.push(format!(
                        "Proma {} / {} 待同步：{error}",
                        workspace.name, installation.name
                    ));
                }
            }
        }
        Ok(warnings)
    })();
    result.unwrap_or_else(|error| vec![format!("Proma 已有安装待核验：{error}")])
}

#[derive(Default)]
pub(crate) struct LocalMirrorSync {
    observed: BTreeMap<PathBuf, String>,
}

#[derive(Default, Debug)]
pub(crate) struct LocalSyncReport {
    pub updated: usize,
    pub inactive_updated: usize,
    pub warnings: Vec<String>,
}

impl LocalMirrorSync {
    /// Two identical observations debounce edits. Only existing marked copies
    /// of registered local sources are eligible, in their current directory.
    pub(crate) fn poll(&mut self, root: &Path, sources: &BTreeSet<PathBuf>) -> Result<LocalSyncReport> {
        let sources = sources.iter().filter_map(|path| path.canonicalize().ok())
            .collect::<BTreeSet<_>>();
        self.observed.retain(|source, _| sources.contains(source));
        let mut followers: BTreeMap<PathBuf, Vec<(Workspace, String, Marker)>> = BTreeMap::new();
        let mut report = LocalSyncReport::default();
        if sources.is_empty() {
            return Ok(report);
        }
        for workspace in workspaces(root)? {
            for directory in ["skills", "skills-inactive"] {
                let parent = workspace.path.join(directory);
                if fs::symlink_metadata(&parent).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound) {
                    continue;
                }
                real_directory(&parent)?;
                for entry in fs::read_dir(parent)? {
                    let path = entry?.path();
                    // Unmanaged packages, built-ins and external links are never adopted.
                    if let Ok(m) = marker(&path, &workspace)
                        && sources.contains(&m.source)
                    {
                        followers.entry(m.source.clone()).or_default()
                            .push((workspace.clone(), directory.into(), m));
                    }
                }
            }
        }
        for (source, copies) in followers {
            let source_hash = match public_manifest(&source).and_then(|m| hash(&m)) {
                Ok(value) => value,
                Err(error) => {
                    self.observed.remove(&source);
                    report.warnings.push(format!("{}：{error}", source.display()));
                    continue;
                }
            };
            let stable = self.observed.insert(source.clone(), source_hash.clone())
                .as_ref() == Some(&source_hash);
            if !stable {
                continue;
            }
            for (workspace, directory, m) in copies {
                if m.source_hash == source_hash {
                    continue;
                }
                #[cfg(test)]
                BEFORE_FOLLOWER_REFRESH.with(|hook| {
                    if let Some(callback) = hook.borrow_mut().take() { callback(); }
                });
                let result = plan_install_at(root, &workspace.path, &source, &m.name, &directory, true)
                    .and_then(|plan| {
                        if plan.source_hash != source_hash {
                            bail!("技能源仍在修改，等待下一次稳定检查");
                        }
                        apply_plan(&plan)
                    });
                match result {
                    Ok(()) => {
                        report.updated += 1;
                        report.inactive_updated += usize::from(directory == "skills-inactive");
                    }
                    Err(error) => report.warnings.push(format!("{} / {}：{error}", workspace.name, m.name)),
                }
            }
        }
        report.warnings.sort();
        report.warnings.dedup();
        Ok(report)
    }
}

pub fn is_managed(path: &Path, source: &Path) -> bool {
    let Ok((_, w)) = path_workspace(path) else {
        return false;
    };
    marker(path, &w).is_ok_and(|m| source.canonicalize().ok().as_ref() == Some(&m.source))
}

#[derive(Clone, Debug, Serialize)]
pub struct Installation {
    pub name: String,
    pub path: PathBuf,
    pub state: String,
    pub status: String,
    pub source: Option<PathBuf>,
    pub managed: bool,
    pub removable: bool,
    pub bundled: bool,
    pub integration: Option<String>,
    toggleable: bool,
}

impl Installation {
    /// Snapshot capability only; set_enabled_path rechecks the live package.
    pub fn can_toggle(&self) -> bool {
        self.toggleable
    }
}

fn skill_entry_bytes(path: &Path) -> Option<Vec<u8>> {
    let file = Directory::open_anchored(path.parent()?).ok()?
        .read(Path::new(path.file_name()?)).ok()?;
    let mut bytes = Vec::new();
    file.take(LIMIT + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= LIMIT).then_some(bytes)
}

fn skill_entry_fingerprint(path: &Path) -> Option<[u8; 32]> {
    skill_entry_bytes(path).map(|bytes| Sha256::digest(&bytes).into())
}

fn integration_entry_fingerprints(root: &Path) -> BTreeMap<String, [u8; 32]> {
    let defaults = root.join("default-skills");
    let mut entries = BTreeMap::new();
    if real_directory(&defaults).is_err() { return entries; }
    for name in ["distill-memory", "read-working-memory", "save-thread", "search-memory", "status"] {
        let template = defaults.join(name);
        if real_directory(&template).is_err() { continue; }
        let Some(bytes) = skill_entry_bytes(&template.join("SKILL.md")) else { continue; };
        // Recognize the configured Proma integration templates, not arbitrary
        // same-name project skills. The workspace entry must also match below.
        if String::from_utf8_lossy(&bytes).contains("Nowledge Mem") {
            entries.insert(name.into(), Sha256::digest(&bytes).into());
        }
    }
    entries
}

fn bundled_entry_fingerprints(root: &Path) -> BTreeMap<String, BTreeSet<[u8; 32]>> {
    #[cfg(target_os = "macos")]
    return bundled_entry_fingerprints_in(&root.join("default-skills"), Path::new("/Applications/Proma.app/Contents/Resources/default-skills"));
    #[cfg(not(target_os = "macos"))]
    { let _ = root; BTreeMap::new() }
}

fn bundled_entry_fingerprints_in(
    defaults: &Path, app_templates: &Path,
) -> BTreeMap<String, BTreeSet<[u8; 32]>> {
    let mut entries = BTreeMap::<String, BTreeSet<[u8; 32]>>::new();
    if real_directory(app_templates).is_err() { return entries; }
    // The user template directory may also contain later additions. Only
    // names supplied by the installed application qualify as bundled.
    for entry in fs::read_dir(app_templates).into_iter().flatten().flatten() {
        let path = entry.path();
        if real_directory(&path).is_err() { continue; }
        let Some(fingerprint) = skill_entry_fingerprint(&path.join("SKILL.md")) else { continue; };
        let name = entry.file_name().to_string_lossy().into_owned();
        let fingerprints = entries.entry(name.clone()).or_default();
        fingerprints.insert(fingerprint);
        let cached = defaults.join(name);
        if real_directory(&cached).is_ok()
            && let Some(fingerprint) = skill_entry_fingerprint(&cached.join("SKILL.md"))
        {
            fingerprints.insert(fingerprint);
        }
    }
    entries
}

pub fn inspect(project: &Path) -> Result<Vec<Installation>> {
    let (root, _) = configured_workspace(project)?;
    inspect_in(&root, project)
}

pub fn inspect_in(root: &Path, project: &Path) -> Result<Vec<Installation>> {
    inspect_with_bundles(root, project, &bundled_entry_fingerprints(root))
}

fn inspect_with_bundles(
    root: &Path, project: &Path, bundled_entries: &BTreeMap<String, BTreeSet<[u8; 32]>>,
) -> Result<Vec<Installation>> {
    let w = workspace_for_path(root, project)?;
    let recovery_clear = no_pending(&w).is_ok();
    let integration_entries = integration_entry_fingerprints(root);
    let mut out = vec![];
    for (directory, state) in [("skills", "active"), ("skills-inactive", "inactive")] {
        let parent = w.path.join(directory);
        if !parent.exists() {
            continue;
        }
        real_directory(&parent)?;
        for entry in fs::read_dir(parent)? {
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let linked = fs::symlink_metadata(&path)?.file_type().is_symlink();
            let found = marker(&path, &w).ok();
            if !path.join("SKILL.md").is_file() && !linked && found.is_none() {
                continue;
            }
            let status = if linked {
                "external_link"
            } else if let Some(m) = &found {
                if clean_marker(&path, &w).is_err() {
                    "modified"
                } else if public_manifest(&m.source)
                    .and_then(|v| hash(&v))
                    .is_ok_and(|h| h == m.source_hash)
                {
                    "current"
                } else {
                    "update_pending"
                }
            } else {
                "unmanaged"
            };
            // An invalid managed marker must not fall back to preset rights.
            let unmarked = fs::symlink_metadata(path.join(MARKER))
                .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound);
            let preset = unmarked && real_directory(&path).is_ok();
            let integration = (preset
                && integration_entries.get(&name).is_some_and(|expected| {
                    skill_entry_fingerprint(&path.join("SKILL.md")).as_ref() == Some(expected)
                })).then(|| "Nowledge Mem".to_string());
            // Match template content, never just a well-known name. Bundled
            // packages can move intact, but do not gain removal rights.
            let bundled = preset && integration.is_none()
                && bundled_entries.get(&name).is_some_and(|fingerprints| {
                    skill_entry_fingerprint(&path.join("SKILL.md"))
                        .is_some_and(|fingerprint| fingerprints.contains(&fingerprint))
                });
            out.push(Installation {
                name,
                path,
                state: state.into(),
                status: status.into(),
                source: found.as_ref().map(|m| m.source.clone()),
                managed: found.is_some(),
                removable: recovery_clear && matches!(status, "current" | "update_pending"),
                bundled,
                integration,
                toggleable: cfg!(unix) && recovery_clear
                    && (matches!(status, "current" | "update_pending") || bundled),
            });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name).then(a.state.cmp(&b.state)));
    Ok(out)
}

pub fn set_enabled(project: &Path, name: &str, enabled: bool) -> Result<()> {
    crate::library::validate_name(name)?;
    let (_, w) = configured_workspace(project)?;
    let from = if enabled { "skills-inactive" } else { "skills" };
    set_enabled_path(&w.path.join(from).join(name), enabled)
}

/// Move one installation in its registered native workspace. A Finder project
/// association is never used to select or fan out this operation.
pub fn set_enabled_path(path: &Path, enabled: bool) -> Result<()> {
    let root = home().context("无法定位 Proma 配置根")?;
    set_enabled_path_with_bundles(&root, path, enabled, &bundled_entry_fingerprints(&root))
}

fn set_enabled_path_with_bundles(
    root: &Path, path: &Path, enabled: bool,
    bundled_entries: &BTreeMap<String, BTreeSet<[u8; 32]>>,
) -> Result<()> {
    if !path.is_absolute() {
        bail!("Proma 安装位置必须是绝对路径");
    }
    let name = path.file_name().and_then(|p| p.to_str()).context("缺少技能名称")?;
    crate::library::validate_name(name)?;
    let (from, to) = if enabled {
        ("skills-inactive", "skills")
    } else {
        ("skills", "skills-inactive")
    };
    let parent = path.parent().context("安装位置缺少父目录")?;
    if parent.file_name().and_then(|p| p.to_str()) != Some(from) {
        bail!("启停源目录应为 {from}，请刷新安装状态");
    }
    let w = workspace_for_path(root, parent.parent().context("安装位置缺少工作区")?)?;
    let workspace = Directory::open_anchored(&w.path)?;
    let _lock = workspace.lock(Path::new(".kitter-proma.lock"))?;
    no_pending(&w)?;
    let source = w.path.join(from).join(name);
    // Open the supplied path as well, rejecting linked or traversing aliases.
    let input = Directory::open_anchored(parent)?;
    input.verify_location(source.parent().context("缺少源目录")?)?;
    real_directory(&source)?;
    let expected = input.observation(Path::new(name))?;
    let installation = inspect_with_bundles(root, &w.path, bundled_entries)?
        .into_iter().find(|item| item.path == source)
        .context("未找到可启停的 Proma 安装")?;
    if !installation.can_toggle() {
        bail!("只允许启停未修改的受管安装或经核验的内置预设");
    }
    if installation.managed {
        clean_marker(&source, &w)?;
    }
    workspace.mkdir(Path::new(to))?;
    let output = workspace.child(Path::new(to))?;
    if workspace_for_path(root, &w.path)? != w
        || input.observation(Path::new(name))? != expected
    {
        bail!("Proma 工作区或安装内容已变化");
    }
    no_pending(&w)?;
    workspace.verify_location(&w.path)?;
    input.verify_location(&w.path.join(from))?;
    output.verify_location(&w.path.join(to))?;
    #[cfg(test)]
    BEFORE_TOGGLE_MOVE.with(|hook| {
        if let Some(callback) = hook.borrow_mut().take() {
            callback();
        }
    });
    input.rename_to(Path::new(name), &output, Path::new(name))?;
    #[cfg(test)]
    AFTER_TOGGLE_MOVE.with(|hook| {
        if let Some(callback) = hook.borrow_mut().take() {
            callback();
        }
    });
    if !output
        .observation(Path::new(name))
        .is_ok_and(|value| value == expected)
        || workspace.verify_location(&w.path).is_err()
        || input.verify_location(&w.path.join(from)).is_err()
        || output.verify_location(&w.path.join(to)).is_err()
    {
        output.rename_to(Path::new(name), &input, Path::new(name))
            .context("启停校验失败，无法恢复原位置；保留两个目录中的现场")?;
        bail!("技能在启停期间变化，已恢复原位置");
    }
    Ok(())
}

pub fn uninstall_path(path: &Path) -> Result<()> {
    let (root, w) = path_workspace(path)?;
    let workspace = Directory::open_anchored(&w.path)?;
    let _lock = workspace.lock(Path::new(".kitter-proma.lock"))?;
    no_pending(&w)?;
    clean_marker(path, &w)?;
    let expected = observed(path)?;
    let input = workspace.child(Path::new(
        path.parent()
            .and_then(|p| p.file_name())
            .context("缺少安装目录")?,
    ))?;
    let (operation, output) = new_operation(&workspace, &w.path, "uninstall")?;
    let mut t = Transaction {
        workspace: w.clone(),
        workspace_signature: workspace.identity()?,
        target: path.to_path_buf(),
        backup: Some(operation.join("previous")),
        before: expected.clone(),
        after: "absent".into(),
        phase: "prepared".into(),
    };
    output.write(
        Path::new("transaction.json"),
        &serde_json::to_vec_pretty(&t)?,
    )?;
    if workspace_for_path(&root, &w.path)? != w || observed(path)? != expected {
        bail!("Proma 工作区或安装内容已变化");
    }
    workspace.verify_location(&w.path)?;
    input.rename_to(
        Path::new(path.file_name().context("缺少技能名称")?),
        &output,
        Path::new("previous"),
    )?;
    if !output
        .observation(Path::new("previous"))
        .is_ok_and(|value| value == expected)
    {
        output.rename_to(
            Path::new("previous"),
            &input,
            Path::new(path.file_name().context("缺少技能名称")?),
        )?;
        t.phase = "rolled_back".into();
        output.write(
            Path::new("transaction.json"),
            &serde_json::to_vec_pretty(&t)?,
        )?;
        bail!("移除对象已变化，已恢复原位置");
    }
    t.phase = "committed".into();
    output.write(
        Path::new("transaction.json"),
        &serde_json::to_vec_pretty(&t)?,
    )?;
    Ok(())
}

pub fn recover(project: &Path) -> Result<()> {
    let (_, w) = configured_workspace(project)?;
    recover_in(&w)
}

fn recover_in(w: &Workspace) -> Result<()> {
    let workspace = Directory::open_anchored(&w.path)?;
    let _lock = workspace.lock(Path::new(".kitter-proma.lock"))?;
    let recovery = match workspace.child(Path::new(RECOVERY)) {
        Ok(recovery) => recovery,
        Err(error) if crate::package_fs::is_not_found(&error) => return Ok(()),
        Err(error) => return Err(error),
    };
    for name in recovery.directory_names()? {
        let operation = w.path.join(RECOVERY).join(&name);
        let output = recovery.child(Path::new(&name))?;
        let mut file = match output.read(Path::new("transaction.json")) {
            Ok(file) => file,
            Err(error) if crate::package_fs::is_not_found(&error) => continue,
            Err(error) => return Err(error),
        };
        let mut bytes = Vec::new();
        file.by_ref().take(LIMIT + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > LIMIT {
            bail!("恢复记录过大");
        }
        let mut t: Transaction =
            serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("恢复记录格式无效"))?;
        if matches!(t.phase.as_str(), "committed" | "rolled_back") {
            continue;
        }
        if t.workspace != *w
            || t.workspace_signature != workspace.identity()?
            || ![w.path.join("skills"), w.path.join("skills-inactive")]
                .iter()
                .any(|p| t.target.parent() == Some(p.as_path()))
            || t.backup
                .as_ref()
                .is_some_and(|p| *p != operation.join("previous"))
        {
            bail!("恢复记录身份或路径不符，保留现场");
        }
        crate::library::validate_name(
            t.target
                .file_name()
                .and_then(|p| p.to_str())
                .context("恢复记录名称无效")?,
        )?;
        let parent = workspace.child(Path::new(
            t.target
                .parent()
                .and_then(|p| p.file_name())
                .context("恢复记录目录无效")?,
        ))?;
        #[cfg(test)]
        BEFORE_RECOVERY_MUTATION.with(|hook| {
            if let Some(callback) = hook.borrow_mut().take() {
                callback();
            }
        });
        workspace.verify_location(&w.path)?;
        recovery.verify_location(&w.path.join(RECOVERY))?;
        output.verify_location(&operation)?;
        restore_with_handles(&t, &output, &parent)?;
        t.phase = "rolled_back".into();
        output.write(
            Path::new("transaction.json"),
            &serde_json::to_vec_pretty(&t)?,
        )?;
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::{package_fs, update_protection};
    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("proma");
        let project = root.join("agent-workspaces/pilot");
        let source = temp.path().join("library/example");
        fs::create_dir_all(project.join("skills")).unwrap();
        fs::create_dir_all(source.join("local-data")).unwrap();
        fs::write(
            root.join("agent-workspaces.json"),
            r#"{"version":1,"workspaces":[{"id":"workspace-1","slug":"pilot","name":"Pilot"}]}"#,
        )
        .unwrap();
        fs::write(
            source.join("SKILL.md"),
            "---\nname: example\ndescription: Example\n---\nRun scripts/example.py.\n",
        )
        .unwrap();
        fs::write(
            source.join("local-data/ADAPTATION.md"),
            "Use the already installed local-data runtime.",
        )
        .unwrap();
        fs::write(source.join("local-data/private.env"), "fixture-secret").unwrap();
        (temp, root, project, source)
    }

    fn builtin_fixture() -> (
        tempfile::TempDir, PathBuf, PathBuf, BTreeMap<String, BTreeSet<[u8; 32]>>,
    ) {
        let (temp, root, project, source) = fixture();
        let templates = root.join("fixture-app/default-skills");
        let template = templates.join("example");
        let copy = project.join("skills/example");
        fs::create_dir_all(&template).unwrap();
        fs::create_dir_all(copy.join("scripts")).unwrap();
        fs::create_dir_all(copy.join("local-data/empty")).unwrap();
        fs::copy(source.join("SKILL.md"), template.join("SKILL.md")).unwrap();
        fs::copy(template.join("SKILL.md"), copy.join("SKILL.md")).unwrap();
        fs::write(copy.join("scripts/run.sh"), "#!/bin/sh\nexit 0\n").unwrap();
        fs::write(copy.join(".extra.bin"), [0, 1, 255]).unwrap();
        fs::write(copy.join("local-data/notes.txt"), "keep local notes").unwrap();
        let bundles = bundled_entry_fingerprints_in(&root.join("default-skills"), &templates);
        (temp, root, project, bundles)
    }

    #[test]
    fn builtin_toggle_round_trip_preserves_the_entire_directory() {
        use std::os::unix::fs::PermissionsExt;
        let (_temp, root, project, bundles) = builtin_fixture();
        let active = project.join("skills/example");
        let inactive = project.join("skills-inactive/example");
        fs::set_permissions(active.join("scripts/run.sh"), fs::Permissions::from_mode(0o755)).unwrap();
        let body = fs::read(active.join("SKILL.md")).unwrap();
        let before = observed(&active).unwrap();
        let scan = inspect_with_bundles(&root, &project, &bundles).unwrap();
        assert!(scan[0].bundled && scan[0].can_toggle());
        assert!(!scan[0].managed && !scan[0].removable);
        for (path, enabled, destination) in [(&active, false, &inactive), (&inactive, true, &active)] {
            set_enabled_path_with_bundles(&root, path, enabled, &bundles).unwrap();
            assert!(!path.exists());
            assert_eq!(observed(destination).unwrap(), before);
            assert_eq!(fs::read(destination.join("SKILL.md")).unwrap(), body);
            assert_eq!(fs::read(destination.join(".extra.bin")).unwrap(), [0, 1, 255]);
            assert_eq!(fs::read_to_string(destination.join("local-data/notes.txt")).unwrap(), "keep local notes");
            assert!(destination.join("local-data/empty").is_dir());
            assert!(!destination.join(MARKER).exists());
            assert!(inspect_with_bundles(&root, &project, &bundles).unwrap()[0].can_toggle());
        }
    }

    #[test]
    fn toggle_never_replaces_same_name_targets_in_either_direction() {
        for enabled in [false, true] {
            for dangling in [false, true] {
                let (_temp, root, project, bundles) = builtin_fixture();
                let active = project.join("skills/example");
                let inactive = project.join("skills-inactive/example");
                fs::create_dir_all(inactive.parent().unwrap()).unwrap();
                let (source, destination) = if enabled {
                    fs::rename(&active, &inactive).unwrap();
                    (&inactive, &active)
                } else { (&active, &inactive) };
                if dangling {
                    std::os::unix::fs::symlink(project.join("absent"), destination).unwrap();
                } else {
                    fs::create_dir(destination).unwrap();
                }
                let before = observed(source).unwrap();
                let target_before = observed(destination).unwrap();
                assert!(set_enabled_path_with_bundles(&root, source, enabled, &bundles).is_err());
                assert_eq!(observed(source).unwrap(), before);
                assert_eq!(observed(destination).unwrap(), target_before);
            }
        }
    }

    #[test]
    fn managed_toggle_keeps_clean_marker_guards_and_allows_pending_updates() {
        let (_temp, root, project, source) = fixture();
        let plan = plan_install(&root, &project, &source, "example").unwrap();
        apply_plan(&plan).unwrap();
        fs::write(source.join("SKILL.md"), "new upstream version").unwrap();
        let scan = inspect_in(&root, &project).unwrap();
        assert_eq!(scan[0].status, "update_pending");
        assert!(scan[0].can_toggle());
        let before = observed(&plan.destination).unwrap();
        let inactive = project.join("skills-inactive/example");
        set_enabled_path_with_bundles(&root, &plan.destination, false, &BTreeMap::new()).unwrap();
        assert_eq!(observed(&inactive).unwrap(), before);
        set_enabled_path_with_bundles(&root, &inactive, true, &BTreeMap::new()).unwrap();
        assert_eq!(observed(&plan.destination).unwrap(), before);
        let original = fs::read(plan.destination.join("SKILL.md")).unwrap();
        for edit_entry in [true, false] {
            if edit_entry {
                fs::write(plan.destination.join("SKILL.md"), "local edit").unwrap();
            } else {
                fs::write(plan.destination.join("SKILL.md"), &original).unwrap();
                fs::write(plan.destination.join("new.txt"), "local addition").unwrap();
            }
            // Even an exact application template cannot override a dirty marker.
            let fingerprint = skill_entry_fingerprint(&plan.destination.join("SKILL.md")).unwrap();
            let bundles = BTreeMap::from([("example".into(), BTreeSet::from([fingerprint]))]);
            assert!(!inspect_with_bundles(&root, &project, &bundles).unwrap()[0].can_toggle());
            let before = observed(&plan.destination).unwrap();
            assert!(set_enabled_path_with_bundles(&root, &plan.destination, false, &bundles).is_err());
            assert_eq!(observed(&plan.destination).unwrap(), before);
        }
    }

    #[test]
    fn toggle_rechecks_presets_and_rejects_unmanaged_or_linked_entries() {
        for kind in ["mismatch", "unmanaged", "external", "entry_link", "invalid_marker", "marker_link", "integration"] {
            let (_temp, root, project, mut bundles) = builtin_fixture();
            let copy = project.join("skills/example");
            assert!(inspect_with_bundles(&root, &project, &bundles).unwrap()[0].can_toggle());
            match kind {
                "mismatch" => fs::write(copy.join("SKILL.md"), "changed after UI snapshot").unwrap(),
                "unmanaged" => bundles.clear(),
                "external" => {
                    fs::rename(&copy, project.join("external")).unwrap();
                    std::os::unix::fs::symlink(project.join("external"), &copy).unwrap();
                }
                "entry_link" => {
                    fs::rename(copy.join("SKILL.md"), project.join("external.md")).unwrap();
                    std::os::unix::fs::symlink(project.join("external.md"), copy.join("SKILL.md")).unwrap();
                }
                "invalid_marker" => fs::write(copy.join(MARKER), "invalid").unwrap(),
                "marker_link" => std::os::unix::fs::symlink(project.join("missing-marker"), copy.join(MARKER)).unwrap(),
                "integration" => {
                    let body = "Nowledge Mem integration";
                    let entry = project.join("skills/status");
                    fs::rename(&copy, &entry).unwrap();
                    let template = root.join("default-skills/status");
                    fs::create_dir_all(&template).unwrap();
                    fs::write(template.join("SKILL.md"), body).unwrap();
                    fs::write(entry.join("SKILL.md"), body).unwrap();
                    // Integration identity still wins over a matching bundle.
                    bundles.insert("status".into(), BTreeSet::from([skill_entry_fingerprint(&entry.join("SKILL.md")).unwrap()]));
                }
                _ => unreachable!(),
            }
            let path = if kind == "integration" { project.join("skills/status") } else { copy };
            let before = observed(&path).unwrap();
            let scan = inspect_with_bundles(&root, &project, &bundles).unwrap();
            assert!(!scan[0].can_toggle(), "{kind}");
            assert!(set_enabled_path_with_bundles(&root, &path, false, &bundles).is_err(), "{kind}");
            assert_eq!(observed(&path).unwrap(), before, "{kind}");
            assert!(!project.join("skills-inactive").exists());
        }
    }

    #[test]
    fn toggle_requires_registered_native_path_and_expected_source_directory() {
        let (temp, root, project, bundles) = builtin_fixture();
        let active = project.join("skills/example");
        let before = observed(&active).unwrap();
        assert!(set_enabled_path_with_bundles(&root, &active, true, &bundles).is_err());
        assert!(set_enabled_path_with_bundles(&root, Path::new("skills/example"), false, &bundles).is_err());
        let unrelated = root.join("agent-workspaces/unregistered");
        let business = temp.path().join("business");
        for workspace in [&unrelated, &business] {
            fs::create_dir_all(workspace.join("skills/example")).unwrap();
            fs::copy(active.join("SKILL.md"), workspace.join("skills/example/SKILL.md")).unwrap();
            let path = workspace.join("skills/example");
            let original = observed(&path).unwrap();
            assert!(set_enabled_path_with_bundles(&root, &path, false, &bundles).is_err());
            assert_eq!(observed(&path).unwrap(), original);
            assert!(!workspace.join("skills-inactive").exists());
        }
        // A Finder association does not make its Skill directory native.
        fs::write(root.join("agent-workspaces.json"), serde_json::to_vec(&serde_json::json!({
            "version": 2, "workspaces": [{"id": "workspace-1", "slug": "pilot", "name": "Pilot", "projectRootPath": business}]
        })).unwrap()).unwrap();
        assert!(set_enabled_path_with_bundles(&root, &business.join("skills/example"), false, &bundles).is_err());
        fs::create_dir_all(project.join("other/example")).unwrap();
        assert!(set_enabled_path_with_bundles(&root, &project.join("other/example"), false, &bundles).is_err());
        let alias = temp.path().join("native-alias");
        std::os::unix::fs::symlink(&project, &alias).unwrap();
        assert!(set_enabled_path_with_bundles(&root, &alias.join("skills/example"), false, &bundles).is_err());
        assert_eq!(observed(&active).unwrap(), before);
        assert!(!project.join("skills-inactive").exists());
    }

    #[test]
    fn toggle_rejects_pending_recovery_and_a_concurrent_writer() {
        let (_temp, root, project, bundles) = builtin_fixture();
        let active = project.join("skills/example");
        let before = observed(&active).unwrap();
        let workspace = Directory::open_anchored(&project).unwrap();
        let lock = workspace.lock(Path::new(".kitter-proma.lock")).unwrap();
        assert!(set_enabled_path_with_bundles(&root, &active, false, &bundles).is_err());
        drop(lock);
        let pending = project.join(RECOVERY).join("pending");
        fs::create_dir_all(&pending).unwrap();
        let transaction = Transaction {
            workspace: workspace_for_path(&root, &project).unwrap(), workspace_signature: workspace.identity().unwrap(),
            target: active.clone(), backup: None, before: before.clone(), after: String::new(), phase: "prepared".into(),
        };
        fs::write(pending.join("transaction.json"), serde_json::to_vec(&transaction).unwrap()).unwrap();
        let scan = inspect_with_bundles(&root, &project, &bundles).unwrap();
        assert!(scan[0].bundled && !scan[0].can_toggle());
        assert!(set_enabled_path_with_bundles(&root, &active, false, &bundles).is_err());
        assert_eq!(observed(&active).unwrap(), before);
        assert!(!project.join("skills-inactive").exists());
    }

    #[test]
    fn toggle_restores_a_package_changed_in_the_last_rename_gap() {
        for reoccupy in [false, true] {
            let (_temp, root, project, bundles) = builtin_fixture();
            let active = project.join("skills/example");
            let changed = active.clone();
            let inactive = project.join("skills-inactive/example");
            if reoccupy {
                AFTER_TOGGLE_MOVE.with(|hook| {
                    *hook.borrow_mut() = Some(Box::new(move || {
                        fs::write(inactive.join("extra.txt"), "concurrent edit").unwrap();
                        fs::create_dir(&changed).unwrap();
                        fs::write(changed.join("keep.txt"), "new occupant").unwrap();
                    }));
                });
            } else {
                BEFORE_TOGGLE_MOVE.with(|hook| {
                    *hook.borrow_mut() = Some(Box::new(move || {
                        fs::write(changed.join("extra.txt"), "concurrent edit").unwrap();
                    }));
                });
            }
            assert!(set_enabled_path_with_bundles(&root, &active, false, &bundles).is_err());
            if reoccupy {
                assert_eq!(fs::read_to_string(active.join("keep.txt")).unwrap(), "new occupant");
                assert!(project.join("skills-inactive/example/extra.txt").is_file());
            } else {
                assert_eq!(fs::read_to_string(active.join("extra.txt")).unwrap(), "concurrent edit");
                assert!(active.join("SKILL.md").is_file());
                assert!(!project.join("skills-inactive/example").exists());
            }
        }
    }

    #[test]
    fn removal_availability_matches_mirror_and_recovery_guards() {
        let (_temp, root, project, source) = fixture();
        let plan = plan_install(&root, &project, &source, "example").unwrap();
        apply_plan(&plan).unwrap();
        let original = fs::read(plan.destination.join("SKILL.md")).unwrap();
        assert!(inspect_in(&root, &project).unwrap()[0].removable);

        fs::write(source.join("SKILL.md"), "upstream changed").unwrap();
        let scan = inspect_in(&root, &project).unwrap();
        assert_eq!(scan[0].status, "update_pending");
        assert!(scan[0].removable);

        fs::create_dir_all(project.join("skills-inactive")).unwrap();
        let inactive = project.join("skills-inactive/example");
        fs::rename(&plan.destination, &inactive).unwrap();
        assert!(inspect_in(&root, &project).unwrap()[0].removable);
        fs::write(inactive.join("SKILL.md"), "local edit").unwrap();
        let scan = inspect_in(&root, &project).unwrap();
        assert_eq!(scan[0].status, "modified");
        assert!(!scan[0].removable);

        let external = project.join("skills/external");
        std::os::unix::fs::symlink(&source, &external).unwrap();
        let unmanaged = project.join("skills/unmanaged");
        fs::create_dir_all(&unmanaged).unwrap();
        fs::write(unmanaged.join("SKILL.md"), "unmanaged").unwrap();
        let scan = inspect_in(&root, &project).unwrap();
        assert!(scan.iter().all(|item| !item.removable));
        assert!(scan.iter().any(|item| item.status == "external_link"));
        assert!(scan.iter().any(|item| item.status == "unmanaged"));

        fs::write(inactive.join("SKILL.md"), original).unwrap();
        assert!(inspect_in(&root, &project).unwrap().iter().find(|item| item.name == "example").unwrap().removable);
        let pending = project.join(RECOVERY).join("pending");
        fs::create_dir_all(&pending).unwrap();
        let transaction = Transaction {
            workspace: plan.workspace.clone(), workspace_signature: plan.workspace_signature,
            target: inactive, backup: None, before: String::new(), after: String::new(),
            phase: "prepared".into(),
        };
        fs::write(pending.join("transaction.json"), serde_json::to_vec(&transaction).unwrap()).unwrap();
        assert!(inspect_in(&root, &project).unwrap().iter().all(|item| !item.removable));
    }

    #[test]
    fn integration_presets_require_template_and_provider_match_without_removal_rights() {
        let (_temp, root, project, _source) = fixture();
        let template = root.join("default-skills/status");
        let entry = project.join("skills/status");
        fs::create_dir_all(&template).unwrap();
        fs::create_dir_all(&entry).unwrap();
        let body = "---\nname: status\ndescription: Check Nowledge Mem from Proma\n---\nNowledge Mem integration.\n";
        fs::write(template.join("SKILL.md"), body).unwrap();
        fs::write(entry.join("SKILL.md"), body).unwrap();
        let scan = inspect_in(&root, &project).unwrap();
        assert_eq!(scan[0].integration.as_deref(), Some("Nowledge Mem"));
        assert!(!scan[0].bundled);
        assert!(!scan[0].removable);
        fs::write(entry.join("SKILL.md"), "custom status skill").unwrap();
        assert!(inspect_in(&root, &project).unwrap()[0].integration.is_none());
        fs::write(template.join("SKILL.md"), "custom status skill").unwrap();
        assert!(inspect_in(&root, &project).unwrap()[0].integration.is_none());
        fs::write(template.join("SKILL.md"), body).unwrap();
        fs::write(entry.join("SKILL.md"), body).unwrap();
        fs::create_dir_all(project.join("skills-inactive")).unwrap();
        fs::rename(&entry, project.join("skills-inactive/status")).unwrap();
        assert_eq!(inspect_in(&root, &project).unwrap()[0].integration.as_deref(), Some("Nowledge Mem"));
        std::os::unix::fs::symlink(&template, &entry).unwrap();
        assert!(inspect_in(&root, &project).unwrap().iter().find(|item| item.state == "active").unwrap().integration.is_none());
    }

    #[test]
    fn bundled_copies_require_matching_template_content_and_no_managed_marker() {
        let (_temp, root, project, source) = fixture();
        let app_templates = root.join("fixture-app/default-skills");
        let defaults = root.join("default-skills");
        let template = app_templates.join("example");
        let copy = project.join("skills/example");
        fs::create_dir_all(&template).unwrap();
        fs::create_dir_all(&copy).unwrap();
        fs::copy(source.join("SKILL.md"), template.join("SKILL.md")).unwrap();
        fs::copy(template.join("SKILL.md"), copy.join("SKILL.md")).unwrap();
        let inspect = || inspect_with_bundles(&root, &project, &bundled_entry_fingerprints_in(&defaults, &app_templates)).unwrap();
        let scan = inspect();
        assert!(scan[0].bundled);
        assert!(!scan[0].removable);
        let cached = defaults.join("example");
        fs::create_dir_all(&cached).unwrap();
        fs::write(cached.join("SKILL.md"), "previous app version").unwrap();
        fs::copy(cached.join("SKILL.md"), copy.join("SKILL.md")).unwrap();
        assert!(inspect()[0].bundled);
        let extra = defaults.join("user-added");
        fs::create_dir_all(&extra).unwrap();
        fs::write(extra.join("SKILL.md"), "added later").unwrap();
        let extra_copy = project.join("skills/user-added");
        fs::create_dir_all(&extra_copy).unwrap();
        fs::copy(extra.join("SKILL.md"), extra_copy.join("SKILL.md")).unwrap();
        assert!(!inspect().iter().find(|item| item.name == "user-added").unwrap().bundled);
        assert!(bundled_entry_fingerprints_in(&defaults, &root.join("missing-app")).is_empty());
        fs::create_dir_all(project.join("skills-inactive")).unwrap();
        let inactive = project.join("skills-inactive/example");
        fs::rename(&copy, &inactive).unwrap();
        assert!(inspect()[0].bundled);
        fs::write(inactive.join("SKILL.md"), "same name, custom content").unwrap();
        assert!(!inspect()[0].bundled);

        let linked_template = app_templates.join("linked");
        fs::create_dir_all(&linked_template).unwrap();
        fs::copy(template.join("SKILL.md"), linked_template.join("SKILL.md")).unwrap();
        std::os::unix::fs::symlink(&linked_template, project.join("skills/linked")).unwrap();
        assert!(!inspect().iter().find(|item| item.name == "linked").unwrap().bundled);

        let plan = plan_install(&root, &project, &source, "managed").unwrap();
        apply_plan(&plan).unwrap();
        let managed_template = app_templates.join("managed");
        fs::create_dir_all(&managed_template).unwrap();
        fs::copy(plan.destination.join("SKILL.md"), managed_template.join("SKILL.md")).unwrap();
        let scan = inspect();
        let managed = scan.iter().find(|item| item.name == "managed").unwrap();
        assert!(managed.removable);
        assert!(!managed.bundled);
    }

    #[test]
    fn business_project_resolves_only_registered_native_destination() {
        let (temp, root, native, source) = fixture();
        let business = temp.path().join("business");
        fs::create_dir_all(business.join("skills/example")).unwrap();
        fs::write(business.join("skills/example/SKILL.md"), "business data").unwrap();
        let original = workspace_for_path(&root, &native).unwrap();
        fs::write(root.join("agent-workspaces.json"), serde_json::to_vec(&serde_json::json!({
            "version": 2, "workspaces": [{"id": "workspace-1", "slug": "pilot", "name": "Pilot",
                "projectRootPath": business}]
        })).unwrap()).unwrap();
        let resolved = workspace_for_project(&root, &business).unwrap();
        assert_eq!(resolved.path, native.canonicalize().unwrap());
        assert_eq!(resolved.project_root_path, Some(business.canonicalize().unwrap()));
        assert_eq!(resolved, original, "Finder binding cannot invalidate existing transaction identity");
        let plan = plan_install(&root, &resolved.path, &source, "example").unwrap();
        apply_plan(&plan).unwrap();
        assert!(native.join("skills/example/.kitter-proma.json").is_file());
        assert_eq!(fs::read_to_string(business.join("skills/example/SKILL.md")).unwrap(), "business data");
        assert!(plan_install(&root, &business, &source, "example").is_err());
    }

    #[test]
    fn multiple_associated_workspaces_require_explicit_mutation_target() {
        let (temp, root, native, _source) = fixture();
        let business = temp.path().join("business");
        let other = root.join("agent-workspaces/other");
        fs::create_dir_all(&business).unwrap();
        fs::create_dir_all(&other).unwrap();
        fs::write(root.join("agent-workspaces.json"), serde_json::to_vec(&serde_json::json!({
            "version": 2, "workspaces": [
                {"id": "workspace-1", "slug": "pilot", "name": "Pilot", "projectRootPath": business},
                {"id": "workspace-2", "slug": "other", "name": "Other", "projectRootPath": business}
            ]
        })).unwrap()).unwrap();
        assert_eq!(workspaces_for_project(&root, &business).unwrap().len(), 2);
        assert!(workspace_for_project(&root, &business).is_err());
        assert_eq!(workspace_for_project(&root, &native).unwrap().path, native.canonicalize().unwrap());
        assert_eq!(workspace_for_project(&root, &other).unwrap().path, other.canonicalize().unwrap());
        assert!(workspaces_for_project(&root, temp.path()).unwrap().is_empty());
    }
    #[test]
    fn physical_entry_reuses_data_without_copying_credentials_and_updates_safely() {
        let (_t, root, project, source) = fixture();
        let plan = plan_install(&root, &project, &source, "example").unwrap();
        assert_eq!(plan.action, "create");
        assert!(!project.join("skills/example").exists());
        apply_plan(&plan).unwrap();
        let target = project.join("skills/example");
        assert!(
            !fs::symlink_metadata(&target)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read_link(target.join("local-data")).unwrap(),
            source.canonicalize().unwrap().join("local-data")
        );
        assert!(
            fs::read_to_string(target.join("SKILL.md"))
                .unwrap()
                .contains("ADAPTATION.md")
        );
        let m = update_protection::snapshot(&target).unwrap();
        assert!(!m.keys().any(|p| p.contains("private.env")));
        assert_eq!(
            fs::read_to_string(source.join("local-data/private.env")).unwrap(),
            "fixture-secret"
        );
        assert_eq!(
            plan_install(&root, &project, &source, "example")
                .unwrap()
                .action,
            "unchanged"
        );
        fs::write(source.join("new.txt"), "new version").unwrap();
        let update = plan_install(&root, &project, &source, "example").unwrap();
        assert_eq!(update.action, "update");
        apply_plan(&update).unwrap();
        assert!(target.join("new.txt").is_file());
        fs::write(target.join("new.txt"), "local edit").unwrap();
        assert!(plan_install(&root, &project, &source, "example").is_err());
        assert_eq!(inspect_in(&root, &project).unwrap()[0].status, "modified");
    }
    #[test]
    fn forged_plan_changed_identity_and_destination_are_rejected() {
        let (_t, root, project, source) = fixture();
        let mut p = plan_install(&root, &project, &source, "example").unwrap();
        p.destination = source.join("overwrite");
        assert!(apply_plan(&p).is_err());
        let p = plan_install(&root, &project, &source, "example").unwrap();
        fs::write(
            root.join("agent-workspaces.json"),
            r#"{"version":1,"workspaces":[{"id":"changed","slug":"pilot","name":"Pilot"}]}"#,
        )
        .unwrap();
        assert!(apply_plan(&p).is_err());
        assert!(!project.join("skills/example").exists());
    }
    #[test]
    fn source_and_target_changes_after_preview_are_rejected() {
        let (_t, root, project, source) = fixture();
        let p = plan_install(&root, &project, &source, "example").unwrap();
        fs::write(source.join("changed.txt"), "changed").unwrap();
        assert!(apply_plan(&p).is_err());
        let p = plan_install(&root, &project, &source, "example").unwrap();
        fs::create_dir_all(&p.destination).unwrap();
        fs::write(p.destination.join("unknown"), "keep").unwrap();
        assert!(apply_plan(&p).is_err());
        assert!(p.destination.join("unknown").is_file());
    }
    #[test]
    fn discovery_does_not_initialize_and_invalid_index_does_not_echo_content() {
        let t = tempfile::tempdir().unwrap();
        assert!(workspaces(&t.path().join("missing")).unwrap().is_empty());
        assert!(!t.path().join("missing").exists());
        fs::write(t.path().join("agent-workspaces.json"), "fixture-secret").unwrap();
        assert!(
            !workspaces(t.path())
                .unwrap_err()
                .to_string()
                .contains("fixture-secret")
        );
    }
    #[cfg(unix)]
    #[test]
    fn exact_legacy_link_is_preserved_in_recovery_and_data_reference_changes_block_updates() {
        let (_t, root, project, source) = fixture();
        let target = project.join("skills/example");
        std::os::unix::fs::symlink(&source, &target).unwrap();
        let p = plan_install(&root, &project, &source, "example").unwrap();
        assert_eq!(p.action, "migrate");
        apply_plan(&p).unwrap();
        let recovered = fs::read_dir(project.join(RECOVERY))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path()
            .join("previous");
        assert_eq!(fs::read_link(recovered).unwrap(), source);
        fs::remove_file(target.join("local-data")).unwrap();
        fs::create_dir(target.join("local-data")).unwrap();
        assert!(plan_install(&root, &project, &source, "example").is_err());
    }
    #[test]
    fn backup_gap_can_be_restored_without_touching_unknown_target_content() {
        let (_t, root, project, source) = fixture();
        let p = plan_install(&root, &project, &source, "example").unwrap();
        apply_plan(&p).unwrap();
        let target = p.destination;
        let before = observed(&target).unwrap();
        let op = project.join(RECOVERY).join("interrupted");
        fs::create_dir(&op).unwrap();
        let backup = op.join("previous");
        package_fs::rename_exclusive(&target, &backup).unwrap();
        let t = Transaction {
            workspace: p.workspace,
            workspace_signature: p.workspace_signature,
            target: target.clone(),
            backup: Some(backup.clone()),
            before,
            after: "unused".into(),
            phase: "replacing".into(),
        };
        restore_transaction(&t, &op).unwrap();
        assert!(target.join("SKILL.md").is_file());
        package_fs::rename_exclusive(&target, &backup).unwrap();
        fs::create_dir(&target).unwrap();
        fs::write(target.join("unknown"), "keep").unwrap();
        assert!(restore_transaction(&t, &op).is_err());
        assert!(target.join("unknown").is_file());
        assert!(backup.join("SKILL.md").is_file());
    }
    #[test]
    fn edit_in_last_publish_gap_is_restored_at_original_location() {
        let (_t, root, project, source) = fixture();
        apply_plan(&plan_install(&root, &project, &source, "example").unwrap()).unwrap();
        fs::write(source.join("new.txt"), "new version").unwrap();
        let p = plan_install(&root, &project, &source, "example").unwrap();
        let target = p.destination.clone();
        BEFORE_PUBLISH.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                fs::write(target.join("SKILL.md"), "concurrent edit").unwrap();
            }))
        });
        assert!(apply_plan(&p).is_err());
        assert_eq!(
            fs::read_to_string(p.destination.join("SKILL.md")).unwrap(),
            "concurrent edit"
        );
        assert!(!p.destination.join("new.txt").exists());
    }

    #[test]
    fn same_content_concurrent_winner_is_not_moved_by_failed_create() {
        let (_t, root, project, source) = fixture();
        let p = plan_install(&root, &project, &source, "example").unwrap();
        let target = p.destination.clone();
        let native = p.workspace.path.clone();
        let data = p.local_data.clone().unwrap();
        BEFORE_PUBLISH.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                let staging = fs::read_dir(native.join(RECOVERY))
                    .unwrap()
                    .next()
                    .unwrap()
                    .unwrap()
                    .path()
                    .join("staging");
                crate::package_fs::copy_tree(&staging, &target).unwrap();
                std::os::unix::fs::symlink(data, target.join("local-data")).unwrap();
            }))
        });
        assert!(apply_plan(&p).is_err());
        assert!(p.destination.join("SKILL.md").is_file());
        assert!(clean_marker(&p.destination, &p.workspace).is_ok());
    }

    #[test]
    fn recovery_parent_link_substitution_never_writes_to_outside_directory() {
        let (temp, root, project, source) = fixture();
        let p = plan_install(&root, &project, &source, "example").unwrap();
        let external = temp.path().join("external");
        fs::create_dir(&external).unwrap();
        let recovery = p.workspace.path.join(RECOVERY);
        let moved = p.workspace.path.join("old-recovery");
        let outside = external.clone();
        BEFORE_STAGING.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                fs::rename(&recovery, moved).unwrap();
                std::os::unix::fs::symlink(outside, recovery).unwrap();
            }))
        });
        assert!(apply_plan(&p).is_err());
        assert_eq!(fs::read_dir(external).unwrap().count(), 0);
        assert!(!p.destination.exists());
    }

    #[test]
    fn private_directory_case_alias_is_never_copied() {
        let (_t, root, project, source) = fixture();
        fs::rename(source.join("local-data"), source.join("LOCAL-DATA")).unwrap();
        let p = plan_install(&root, &project, &source, "example").unwrap();
        apply_plan(&p).unwrap();
        assert!(
            !Directory::open_anchored(&p.destination)
                .unwrap()
                .installation_snapshot()
                .unwrap()
                .keys()
                .any(|p| p.contains("private.env"))
        );
        for entry in fs::read_dir(p.workspace.path.join(RECOVERY)).unwrap() {
            assert!(
                !Directory::open_anchored(&entry.unwrap().path())
                    .unwrap()
                    .installation_snapshot()
                    .unwrap()
                    .keys()
                    .any(|p| p.contains("private.env"))
            );
        }
    }

    #[test]
    fn corrupted_backup_blocks_recovery_before_moving_current_target() {
        let (_t, root, project, source) = fixture();
        let p = plan_install(&root, &project, &source, "example").unwrap();
        apply_plan(&p).unwrap();
        let operation = p.workspace.path.join(RECOVERY).join("corrupt-backup");
        fs::create_dir(&operation).unwrap();
        let backup = operation.join("previous");
        crate::package_fs::copy_tree(&p.destination, &backup).unwrap();
        let before = observed(&backup).unwrap();
        let after = observed(&p.destination).unwrap();
        fs::write(backup.join("SKILL.md"), "modified backup").unwrap();
        let t = Transaction {
            workspace: p.workspace,
            workspace_signature: p.workspace_signature,
            target: p.destination.clone(),
            backup: Some(backup),
            before,
            after,
            phase: "replacing".into(),
        };
        assert!(restore_transaction(&t, &operation).is_err());
        assert!(p.destination.join("SKILL.md").is_file());
        assert!(!operation.join("failed").exists());
    }
    #[test]
    fn committed_source_refresh_only_updates_existing_clean_active_followers() {
        let (_t, root, project, source) = fixture();
        apply_plan(&plan_install(&root, &project, &source, "example").unwrap()).unwrap();
        fs::write(source.join("version.txt"), "v2").unwrap();
        assert!(sync_existing_source_in(&root, &source).is_empty());
        let target = project.join("skills/example");
        assert_eq!(
            fs::read_to_string(target.join("version.txt")).unwrap(),
            "v2"
        );
        fs::write(target.join("version.txt"), "local edit").unwrap();
        fs::write(source.join("version.txt"), "v3").unwrap();
        assert_eq!(sync_existing_source_in(&root, &source).len(), 1);
        assert_eq!(
            fs::read_to_string(target.join("version.txt")).unwrap(),
            "local edit"
        );
        fs::create_dir(project.join("skills-inactive")).unwrap();
        fs::rename(&target, project.join("skills-inactive/example")).unwrap();
        assert!(sync_existing_source_in(&root, &source).is_empty());
        assert!(!target.exists());
    }

    #[test]
    fn local_auto_sync_debounces_edits_and_updates_inactive_without_enabling() {
        let (_t, root, project, source) = fixture();
        apply_plan(&plan_install(&root, &project, &source, "example").unwrap()).unwrap();
        let second = root.join("agent-workspaces/second");
        let empty = root.join("agent-workspaces/empty");
        fs::create_dir_all(second.join("skills")).unwrap();
        fs::create_dir_all(&empty).unwrap();
        fs::write(root.join("agent-workspaces.json"), serde_json::to_vec(&serde_json::json!({
            "version":1,"workspaces":[
                {"id":"workspace-1","slug":"pilot","name":"Pilot"},
                {"id":"workspace-2","slug":"second","name":"Second"},
                {"id":"workspace-3","slug":"empty","name":"Empty"}
            ]
        })).unwrap()).unwrap();
        apply_plan(&plan_install(&root, &second, &source, "example").unwrap()).unwrap();
        fs::create_dir(second.join("skills-inactive")).unwrap();
        fs::rename(second.join("skills/example"), second.join("skills-inactive/example")).unwrap();
        let sources = BTreeSet::from([source.clone()]);
        let mut sync = LocalMirrorSync::default();
        assert_eq!(sync.poll(&root, &sources).unwrap().updated, 0);
        fs::write(source.join("version.txt"), "v2").unwrap();
        assert_eq!(sync.poll(&root, &sources).unwrap().updated, 0);
        fs::write(source.join("version.txt"), "v3").unwrap();
        assert_eq!(sync.poll(&root, &sources).unwrap().updated, 0);
        let report = sync.poll(&root, &sources).unwrap();
        assert!(report.warnings.is_empty());
        assert_eq!((report.updated, report.inactive_updated), (2, 1));
        for path in [project.join("skills/example"), second.join("skills-inactive/example")] {
            assert_eq!(fs::read_to_string(path.join("version.txt")).unwrap(), "v3");
        }
        assert!(!second.join("skills/example").exists());
        assert!(!empty.join("skills").exists());
        assert_eq!(sync.poll(&root, &sources).unwrap().updated, 0);
    }

    #[test]
    fn local_auto_sync_preserves_modified_unmanaged_and_unregistered_copies() {
        let (_t, root, project, source) = fixture();
        apply_plan(&plan_install(&root, &project, &source, "example").unwrap()).unwrap();
        apply_plan(&plan_install(&root, &project, &source, "disabled").unwrap()).unwrap();
        fs::create_dir(project.join("skills-inactive")).unwrap();
        fs::rename(project.join("skills/disabled"), project.join("skills-inactive/disabled")).unwrap();
        for path in [project.join("skills/example"), project.join("skills-inactive/disabled")] {
            fs::write(path.join("version.txt"), "workspace edit").unwrap();
        }
        let unmanaged = project.join("skills/unmanaged");
        fs::create_dir(&unmanaged).unwrap();
        fs::write(unmanaged.join("SKILL.md"), "unmanaged content").unwrap();
        let other = source.parent().unwrap().join("other");
        fs::create_dir(&other).unwrap();
        fs::write(other.join("SKILL.md"), "other content").unwrap();
        apply_plan(&plan_install(&root, &project, &other, "other").unwrap()).unwrap();
        let sources = BTreeSet::from([source.clone()]);
        let mut sync = LocalMirrorSync::default();
        sync.poll(&root, &sources).unwrap();
        fs::write(source.join("version.txt"), "source edit").unwrap();
        fs::write(other.join("SKILL.md"), "other edit").unwrap();
        sync.poll(&root, &sources).unwrap();
        let report = sync.poll(&root, &sources).unwrap();
        assert_eq!(report.updated, 0);
        assert_eq!(report.warnings.len(), 2);
        for path in [project.join("skills/example"), project.join("skills-inactive/disabled")] {
            assert_eq!(fs::read_to_string(path.join("version.txt")).unwrap(), "workspace edit");
        }
        assert_eq!(fs::read_to_string(unmanaged.join("SKILL.md")).unwrap(), "unmanaged content");
        assert_eq!(fs::read_to_string(project.join("skills/other/SKILL.md")).unwrap(), "other content");
        assert!(!project.join("skills/disabled").exists());
    }

    #[test]
    fn local_auto_sync_does_not_recreate_removed_or_replaced_copies() {
        for inactive in [false, true] {
            for replace_with_link in [false, true] {
                let (_t, root, project, source) = fixture();
                apply_plan(&plan_install(&root, &project, &source, "example").unwrap()).unwrap();
                let target = if inactive {
                    fs::create_dir(project.join("skills-inactive")).unwrap();
                    let path = project.join("skills-inactive/example");
                    fs::rename(project.join("skills/example"), &path).unwrap();
                    path
                } else { project.join("skills/example") };
                let sources = BTreeSet::from([source.clone()]);
                let mut sync = LocalMirrorSync::default();
                fs::write(source.join("version.txt"), "v2").unwrap();
                sync.poll(&root, &sources).unwrap();
                let saved = project.join("removed");
                let changed_target = target.clone();
                let origin = source.clone();
                BEFORE_FOLLOWER_REFRESH.with(|hook| {
                    *hook.borrow_mut() = Some(Box::new(move || {
                        fs::rename(&changed_target, saved).unwrap();
                        if replace_with_link { std::os::unix::fs::symlink(origin, changed_target).unwrap(); }
                    }));
                });
                let report = sync.poll(&root, &sources).unwrap();
                assert_eq!((report.updated, report.warnings.len()), (0, 1));
                if replace_with_link {
                    assert!(fs::symlink_metadata(&target).unwrap().file_type().is_symlink());
                } else { assert!(!target.exists()); }
            }
        }
    }

    #[test]
    fn local_auto_sync_rechecks_source_stability_before_publish() {
        let (_t, root, project, source) = fixture();
        apply_plan(&plan_install(&root, &project, &source, "example").unwrap()).unwrap();
        fs::write(source.join("version.txt"), "v2").unwrap();
        let sources = BTreeSet::from([source.clone()]);
        let mut sync = LocalMirrorSync::default();
        sync.poll(&root, &sources).unwrap();
        let changed_source = source.clone();
        BEFORE_FOLLOWER_REFRESH.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || { fs::write(changed_source.join("version.txt"), "v3").unwrap(); }));
        });
        let report = sync.poll(&root, &sources).unwrap();
        assert_eq!((report.updated, report.warnings.len()), (0, 1));
        assert!(!project.join("skills/example/version.txt").exists());
        assert_eq!(sync.poll(&root, &sources).unwrap().updated, 0);
        assert_eq!(sync.poll(&root, &sources).unwrap().updated, 1);
        assert_eq!(fs::read_to_string(project.join("skills/example/version.txt")).unwrap(), "v3");
    }

    #[test]
    fn local_auto_sync_respects_removal_during_plan_observation() {
        for inactive in [false, true] {
            let (_t, root, project, source) = fixture();
            apply_plan(&plan_install(&root, &project, &source, "example").unwrap()).unwrap();
            let directory = if inactive { "skills-inactive" } else { "skills" };
            if inactive {
                fs::create_dir(project.join(directory)).unwrap();
                fs::rename(project.join("skills/example"), project.join(directory).join("example")).unwrap();
            }
            fs::write(source.join("version.txt"), "v2").unwrap();
            let mut invalid = plan_install_at(&root, &project, &source, "example", directory, true).unwrap();
            invalid.expected = "absent".into();
            assert!(apply_plan(&invalid).is_err());
            let sources = BTreeSet::from([source.clone()]);
            let mut sync = LocalMirrorSync::default();
            sync.poll(&root, &sources).unwrap();
            let target = project.join(directory).join("example");
            let removed = project.join("removed-during-plan");
            let moved = target.clone();
            BEFORE_PLAN_OBSERVATION.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move || { fs::rename(moved, removed).unwrap(); }));
            });
            let report = sync.poll(&root, &sources).unwrap();
            assert_eq!((report.updated, report.warnings.len()), (0, 1));
            assert!(!target.exists());
        }
    }
    #[test]
    fn source_refresh_does_not_recreate_a_removed_or_replaced_follower() {
        for replace_with_link in [false, true] {
            let (_t, root, project, source) = fixture();
            let p = plan_install(&root, &project, &source, "example").unwrap();
            apply_plan(&p).unwrap();
            fs::write(source.join("version.txt"), "v2").unwrap();
            let target = p.destination.clone();
            let saved = p.workspace.path.join("removed");
            let origin = p.source.clone();
            BEFORE_FOLLOWER_REFRESH.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move || {
                    fs::rename(&target, saved).unwrap();
                    if replace_with_link {
                        std::os::unix::fs::symlink(origin, target).unwrap();
                    }
                }))
            });
            assert_eq!(sync_existing_source_in(&root, &source).len(), 1);
            if replace_with_link {
                assert!(
                    fs::symlink_metadata(&p.destination)
                        .unwrap()
                        .file_type()
                        .is_symlink()
                );
            } else {
                assert!(!p.destination.exists());
            }
        }
    }

    #[test]
    fn missing_managed_entry_reports_modified_and_preserves_user_deletion() {
        let (_t, root, project, source) = fixture();
        let p = plan_install(&root, &project, &source, "example").unwrap();
        apply_plan(&p).unwrap();
        fs::remove_file(p.destination.join("SKILL.md")).unwrap();
        assert_eq!(inspect_in(&root, &project).unwrap()[0].status, "modified");
        assert_eq!(sync_existing_source_in(&root, &source).len(), 1);
        assert!(!p.destination.join("SKILL.md").exists());
    }

    #[test]
    fn workspace_lock_rejects_a_second_writer_without_installing() {
        let (_t, root, project, source) = fixture();
        let p = plan_install(&root, &project, &source, "example").unwrap();
        let workspace = Directory::open_anchored(&p.workspace.path).unwrap();
        let _lock = workspace.lock(Path::new(".kitter-proma.lock")).unwrap();
        assert!(apply_plan(&p).is_err());
        assert!(!p.destination.exists());
    }
    #[test]
    fn unobservable_original_is_restored_after_last_publish_gap() {
        use std::os::unix::net::UnixListener;
        let (_t, root, project, source) = fixture();
        apply_plan(&plan_install(&root, &project, &source, "example").unwrap()).unwrap();
        fs::write(source.join("version.txt"), "v2").unwrap();
        let p = plan_install(&root, &project, &source, "example").unwrap();
        let target = p.destination.clone();
        let short = tempfile::tempdir().unwrap();
        let socket = short.path().join("s");
        let listener = UnixListener::bind(&socket).unwrap();
        BEFORE_PUBLISH.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                fs::rename(socket, target.join("concurrent.sock")).unwrap();
                drop(listener);
            }))
        });
        assert!(apply_plan(&p).is_err());
        assert!(p.destination.join("concurrent.sock").exists());
        assert!(p.destination.join("SKILL.md").is_file());
        assert!(!p.destination.join("version.txt").exists());
    }

    #[test]
    fn rollback_rechecks_the_actual_objects_moved_after_its_preflight() {
        for replace_backup in [false, true] {
            let (_t, root, project, source) = fixture();
            let p = plan_install(&root, &project, &source, "example").unwrap();
            apply_plan(&p).unwrap();
            let operation = p.workspace.path.join(RECOVERY).join("rollback-gap");
            fs::create_dir(&operation).unwrap();
            let backup = operation.join("previous");
            crate::package_fs::copy_tree(&p.destination, &backup).unwrap();
            let t = Transaction {
                workspace: p.workspace,
                workspace_signature: p.workspace_signature,
                target: p.destination.clone(),
                backup: Some(backup.clone()),
                before: observed(&backup).unwrap(),
                after: observed(&p.destination).unwrap(),
                phase: "replacing".into(),
            };
            let selected = if replace_backup {
                backup.clone()
            } else {
                p.destination.clone()
            };
            let moved = operation.join("concurrent-original");
            BEFORE_ROLLBACK_MOVE.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move || {
                    fs::rename(&selected, moved).unwrap();
                    fs::create_dir(&selected).unwrap();
                    fs::write(selected.join("unknown"), "keep").unwrap();
                }))
            });
            assert!(restore_transaction(&t, &operation).is_err());
            if replace_backup {
                assert!(backup.join("unknown").is_file());
                assert!(p.destination.join("SKILL.md").is_file());
            } else {
                assert!(p.destination.join("unknown").is_file());
                assert!(backup.join("SKILL.md").is_file());
            }
        }
    }

    #[test]
    fn recovery_retains_operation_descriptor_and_rejects_replaced_directory() {
        let (_t, root, project, source) = fixture();
        let p = plan_install(&root, &project, &source, "example").unwrap();
        apply_plan(&p).unwrap();
        let operation = p.workspace.path.join(RECOVERY).join("install-interrupted");
        fs::create_dir(&operation).unwrap();
        let backup = operation.join("previous");
        let before = observed(&p.destination).unwrap();
        crate::package_fs::rename_exclusive(&p.destination, &backup).unwrap();
        let t = Transaction {
            workspace: p.workspace.clone(),
            workspace_signature: p.workspace_signature,
            target: p.destination.clone(),
            backup: Some(backup.clone()),
            before,
            after: "unpublished".into(),
            phase: "replacing".into(),
        };
        Directory::open_anchored(&operation)
            .unwrap()
            .write(
                Path::new("transaction.json"),
                &serde_json::to_vec(&t).unwrap(),
            )
            .unwrap();
        let saved = operation.with_file_name("saved-original");
        let replaced = operation.clone();
        BEFORE_RECOVERY_MUTATION.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                fs::rename(&replaced, saved).unwrap();
                fs::create_dir(&replaced).unwrap();
                fs::write(replaced.join("transaction.json"), "keep unknown record").unwrap();
            }))
        });
        assert!(recover_in(&p.workspace).is_err());
        assert_eq!(
            fs::read_to_string(operation.join("transaction.json")).unwrap(),
            "keep unknown record"
        );
        assert!(!p.destination.exists());
        assert!(
            operation
                .with_file_name("saved-original")
                .join("previous/SKILL.md")
                .is_file()
        );
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn toggle_rejects_unsupported_writes_without_changing_directories() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("proma");
        let project = root.join("agent-workspaces/pilot");
        let active = project.join("skills/example");
        fs::create_dir_all(&active).unwrap();
        fs::write(root.join("agent-workspaces.json"),
            r#"{"version":1,"workspaces":[{"id":"workspace-1","slug":"pilot","name":"Pilot"}]}"#).unwrap();
        fs::write(active.join("SKILL.md"), "builtin fixture").unwrap();
        let fingerprint = Sha256::digest(b"builtin fixture").into();
        let bundles = BTreeMap::from([("example".into(), BTreeSet::from([fingerprint]))]);
        assert!(set_enabled_path_with_bundles(&root, &active, false, &bundles).is_err());
        assert_eq!(fs::read_to_string(active.join("SKILL.md")).unwrap(), "builtin fixture");
        assert!(!project.join("skills-inactive").exists());
        assert!(!project.join(".kitter-proma.lock").exists());
    }
}
