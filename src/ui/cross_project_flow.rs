use super::*;
use gpui_base::Disableable;
use crate::cross_project::{Action, BatchResult, Catalog, Disposition, Plan, Skill};

pub(super) struct CrossProjectState {
    catalog: Option<Arc<Catalog>>,
    plan: Option<Plan>,
    result: Option<BatchResult>,
    skills: BTreeSet<String>,
    agents: BTreeSet<String>,
    projects: BTreeSet<PathBuf>,
    workspaces: BTreeSet<String>,
    collapsed: BTreeSet<PathBuf>,
    skill_search: Entity<InputState>,
    project_search: Entity<InputState>,
    skill_scroll: ScrollHandle,
    project_scroll: ScrollHandle,
    action: Action,
    loading: bool,
    planning: bool,
    busy: bool,
    reviewed: bool,
    generation: u64,
    plan_generation: u64,
    error: Option<String>,
}

fn toggle_visible<T: Ord + Clone>(selected: &mut BTreeSet<T>, visible: &[T]) {
    if !visible.is_empty() && visible.iter().all(|key| selected.contains(key)) {
        selected.retain(|key| !visible.contains(key));
    } else { selected.extend(visible.iter().cloned()); }
}

impl CrossProjectState {
    pub(super) fn new(english: bool, window: &mut Window, cx: &mut Context<KitterApp>) -> Self {
        let skill_search = cx.new(|cx| InputState::new(window, cx).placeholder(if english { "Search skill names" } else { "搜索技能名称" }));
        let project_search = cx.new(|cx| InputState::new(window, cx).placeholder(if english { "Search projects or workspaces" } else { "搜索项目或工作区" }));
        for search in [&skill_search, &project_search] {
            cx.subscribe_in(search, window, |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.cross_project.skill_scroll.set_offset(point(px(0.), px(0.)));
                    this.cross_project.project_scroll.set_offset(point(px(0.), px(0.)));
                    this.notify_dialog(cx);
                }
            }).detach();
        }
        Self { catalog: None, plan: None, result: None, skills: BTreeSet::new(), agents: BTreeSet::new(), projects: BTreeSet::new(),
            workspaces: BTreeSet::new(), collapsed: BTreeSet::new(), skill_search, project_search,
            skill_scroll: ScrollHandle::new(), project_scroll: ScrollHandle::new(), action: Action::Install,
            loading: false, planning: false, busy: false, reviewed: false, generation: 0, plan_generation: 0, error: None }
    }
}

impl KitterApp {
    pub(super) fn cross_project_busy(&self) -> bool { self.cross_project.busy }

    pub(super) fn cross_project_agents_changed(&mut self) {
        if self.cross_project.busy { return; }
        self.cross_project.agents.retain(|id| self.model.installed_agents.iter().any(|a| a.id == id && !a.global_only));
        if let Some(catalog) = &mut self.cross_project.catalog { Arc::make_mut(catalog).installed_agents = self.model.installed_agents.clone(); }
        self.cross_project.plan_generation = self.cross_project.plan_generation.wrapping_add(1);
        self.cross_project.plan = None;
        self.cross_project.planning = false;
        self.cross_project.reviewed = false;
        self.cross_project.error = Some(self.tr("Agent 安装状态已变化，请刷新后重新预览", "Agent availability changed; refresh to review again").into());
    }

    pub(super) fn cross_project_explanation(&self) -> String {
        self.tr("将所选技能批量安装到多个项目，或移除项目安装。Proma 还支持跨工作区启用、停用已有副本，停用后保留文件。",
            "Install selected skills across projects or remove project installations. Proma also supports enabling and disabling existing workspace copies; inactive files are retained.").to_string()
    }

    pub(super) fn open_library_cross_project(&mut self, cx: &mut Context<Self>) {
        let selected = self.selected_skill_keys();
        let keys = self.model.skills.iter().filter(|s| selected.iter().any(|key| key == skill_storage_name(s)))
            .map(|s| crate::proma_batch::source_key(&s.path.join("SKILL.md"))).collect();
        self.open_cross_project(keys, None, Action::Install, None, cx);
    }

    pub(super) fn open_proma_batch(&mut self, keys: Vec<String>, project: Option<PathBuf>, enabled: bool, cx: &mut Context<Self>) {
        self.open_cross_project(keys, project, if enabled { Action::Enable } else { Action::Disable }, Some("proma".into()), cx);
    }

    pub(super) fn open_cross_project(&mut self, keys: Vec<String>, project: Option<PathBuf>, action: Action, agent: Option<String>, cx: &mut Context<Self>) {
        if self.cross_project.busy { return; }
        self.cross_project.skills = keys.into_iter().collect();
        self.cross_project.agents = agent.into_iter().collect();
        if self.cross_project.agents.is_empty() && let Some(path) = &project {
            if let Some(selected) = self.projects_view.selected_project_agent {
                self.cross_project.agents.insert(selected.id().to_string());
            } else if let Some(entry) = self.all_project_entries().iter().find(|entry| &entry.path == path) {
                self.cross_project.agents.extend(entry.agents.iter().map(|a| a.id().to_string()));
            }
        }
        self.cross_project.projects = project.clone().into_iter().collect();
        self.cross_project.workspaces.clear();
        self.cross_project.collapsed.clear();
        self.cross_project.action = action;
        self.cross_project.reviewed = false;
        self.cross_project.result = None;
        self.open_dialog(DialogKind::CrossProject, cx);
        self.load_cross_project(cx);
    }

    fn load_cross_project(&mut self, cx: &mut Context<Self>) {
        if self.cross_project.busy { return; }
        self.cross_project.generation = self.cross_project.generation.wrapping_add(1);
        self.cross_project.plan_generation = self.cross_project.plan_generation.wrapping_add(1);
        let generation = self.cross_project.generation;
        self.cross_project.loading = true;
        self.cross_project.planning = false;
        self.cross_project.result = None;
        self.cross_project.error = None;
        self.cross_project.plan = None;
        self.cross_project.reviewed = false;
        let projects = self.all_project_entries();
        let installed = self.model.installed_agents.clone();
        let root = crate::proma::home();
        let skills = self.model.skills.iter().map(|s| Skill {
            key: crate::proma_batch::source_key(&s.path.join("SKILL.md")), name: s.record.name.clone(), source: s.path.clone(), provider: "library",
            install_name: (!s.record.origin.is_builtin()).then(|| skill_storage_name(s).to_string()),
        }).collect();
        let preview = cfg!(debug_assertions) && std::env::var("KITTER_PREVIEW_CROSS_PROJECT").as_deref() == Ok("1") && self.cross_project.catalog.is_none();
        let task = cx.background_executor().spawn(async move { Catalog::scan(skills, projects, installed, root.as_deref()) });
        cx.spawn(async move |this, cx| {
            let catalog = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.cross_project.generation != generation { return; }
                this.cross_project.loading = false;
                if let Some(proma) = &catalog.proma {
                    for copy in proma.workspaces.iter().flat_map(|w| &w.installations) {
                        if this.cross_project.skills.contains(&crate::proma_batch::source_key(&copy.path.join("SKILL.md"))) {
                            this.cross_project.skills.insert(crate::proma_batch::installation_key(copy));
                        }
                    }
                }
                this.cross_project.skills.retain(|key| catalog.skills.iter().any(|s| &s.key == key));
                this.cross_project.agents.retain(|id| catalog.installed_agents.iter().any(|a| a.id == id && !a.global_only));
                this.cross_project.projects.retain(|path| catalog.projects.iter().any(|p| &p.path == path));
                this.cross_project.workspaces.retain(|id| catalog.proma.as_ref().is_some_and(|p| p.workspaces.iter().any(|w| &w.workspace.id == id)));
                if preview {
                    this.cross_project.agents = ["codex", "proma"].into_iter().filter(|id| catalog.installed_agents.iter().any(|a| a.id == *id)).map(str::to_string).collect();
                    this.cross_project.projects.extend(catalog.projects.iter().filter(|p| p.is_active()).take(2).map(|p| p.path.clone()));
                    this.cross_project.workspaces.extend(catalog.proma.iter().flat_map(|p| &p.workspaces).filter(|w| !w.workspace.archived).take(3).map(|w| w.workspace.id.clone()));
                }
                this.cross_project.catalog = Some(Arc::new(catalog));
                this.update_cross_project_plan(cx);
            });
        }).detach();
        self.notify_dialog(cx);
    }

    fn update_cross_project_plan(&mut self, cx: &mut Context<Self>) {
        let Some(catalog) = self.cross_project.catalog.clone() else { return; };
        if self.cross_project.busy || self.cross_project.loading { return; }
        let state = &mut self.cross_project;
        state.plan_generation = state.plan_generation.wrapping_add(1);
        let generation = state.plan_generation;
        let (action, skills, agents, projects, workspaces) = (state.action, state.skills.clone(), state.agents.clone(), state.projects.clone(), state.workspaces.clone());
        state.planning = true; state.plan = None; state.error = None; state.reviewed = false;
        let task = cx.background_executor().spawn(async move { catalog.plan(action, &skills, &agents, &projects, &workspaces).map_err(|e| format!("{e:#}")) });
        cx.spawn(async move |this, cx| {
            let plan = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.cross_project.plan_generation != generation { return; }
                this.cross_project.planning = false;
                match plan { Ok(plan) => this.cross_project.plan = Some(plan), Err(error) => this.cross_project.error = Some(error) }
                this.notify_dialog(cx);
            });
        }).detach();
        self.notify_dialog(cx);
    }

    fn execute_cross_project(&mut self, retry: bool, cx: &mut Context<Self>) {
        let state = &mut self.cross_project;
        if state.busy || state.loading || state.planning || !state.reviewed { return; }
        let plan = if retry { state.result.as_ref().map(BatchResult::retry_plan) }
            else if state.result.is_none() { state.plan.clone() } else { None };
        let Some(plan) = plan.filter(|p| p.count(Disposition::Change) > 0) else { return; };
        state.busy = true;
        let task = cx.background_executor().spawn(async move { plan.execute() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.cross_project.busy = false;
                if let Some(previous) = &mut this.cross_project.result { previous.merge_retry(result); }
                else { this.cross_project.result = Some(result); }
                this.refresh(cx); this.notify_dialog(cx);
            });
        }).detach();
        self.notify_dialog(cx);
    }

    fn cross_project_verb(&self, action: Action) -> &'static str {
        match action { Action::Install => self.tr("安装", "Install"), Action::Remove => self.tr("移除安装", "Remove"),
            Action::Enable => self.tr("启用", "Enable"), Action::Disable => self.tr("停用", "Disable") }
    }

    fn cross_project_status(&self, disposition: Disposition, action: Action) -> String {
        match disposition {
            Disposition::Change => if self.uses_english() { format!("Will {}", self.cross_project_verb(action)) } else { format!("将{}", self.cross_project_verb(action)) },
            Disposition::Unchanged => self.tr("无需变更", "Unchanged").into(),
            Disposition::Missing => self.tr("未安装", "Not installed").into(),
            Disposition::Protected => self.tr("受保护", "Protected").into(),
            Disposition::Unavailable => self.tr("无法扫描", "Unavailable").into(),
        }
    }

    fn cross_project_check(&self, checked: bool) -> Div {
        let p = self.palette();
        div().size(px(15.)).flex_none().rounded(px(4.)).border_1().border_color(if checked { p.accent } else { p.border_strong })
            .bg(if checked { p.accent } else { p.surface }).flex().items_center().justify_center()
            .when(checked, |d| d.child(Self::icon("icons/check.svg", 11., rgb(0xffffff))))
    }

    fn cross_project_skill_keys(&self, cx: &App) -> Vec<String> {
        let state = &self.cross_project;
        let query = state.skill_search.read(cx).value().trim().to_lowercase();
        let mut skills = state.catalog.iter().flat_map(|c| &c.skills).filter(|s| query.is_empty() || s.name.to_lowercase().contains(&query)).collect::<Vec<_>>();
        skills.sort_by_key(|s| (!state.skills.contains(&s.key), s.name.to_lowercase(), s.key.clone()));
        skills.into_iter().map(|s| s.key.clone()).collect()
    }

    fn cross_project_visible_projects(&self, cx: &App) -> Vec<PathBuf> {
        let query = self.cross_project.project_search.read(cx).value().trim().to_lowercase();
        self.cross_project.catalog.iter().flat_map(|c| &c.projects).filter(|p| {
            query.is_empty() || p.name.to_lowercase().contains(&query) || p.path.to_string_lossy().to_lowercase().contains(&query)
                || self.cross_project.catalog.as_ref().and_then(|c| c.proma.as_ref()).is_some_and(|c| c.workspaces.iter().any(|w| {
                    w.workspace.project_root_path.as_ref().unwrap_or(&w.workspace.path) == &p.path
                        && format!("{} {} {}", w.workspace.name, w.workspace.slug, w.workspace.path.display()).to_lowercase().contains(&query)
                }))
        }).map(|p| p.path.clone()).collect()
    }

    fn cross_project_skills_picker(&self, height: f32, cx: &mut Context<Self>) -> Div {
        let p = self.palette(); let state = &self.cross_project;
        let visible = self.cross_project_skill_keys(cx);
        let mut list = div().id("cross-project-skills-list").h(px(height)).overflow_y_scroll().track_scroll(&state.skill_scroll);
        if let Some(catalog) = &state.catalog {
            for (index, key) in visible.iter().enumerate() {
                let Some(skill) = catalog.skills.iter().find(|s| &s.key == key) else { continue; };
                let checked = state.skills.contains(key); let key = key.clone();
                let provider = match skill.provider { "library" | "managed" => self.tr("技能库", "Library"), "builtin" => self.tr("Proma 内置预设", "Proma built-in"),
                    "integration" => self.tr("Proma 集成", "Proma integration"), _ => self.tr("项目来源", "Project source") };
                let source = format!("{provider} · {}", display_path(&skill.source));
                let tip = source.clone();
                list = list.child(gpui_base::Button::new(ElementId::Name(format!("cross-project-skill-{index}").into()))
                    .accessibility_label(format!("{} · {}", skill.name, if checked { self.tr("已选", "Selected") } else { self.tr("未选", "Not selected") }))
                    .w_full().border_0().px(px(10.)).py(px(8.)).rounded(px(6.)).flex().items_center().gap(px(9.))
                    .bg(if checked { p.selected } else { rgba(0x00000000) }).hover(move |d| d.bg(p.hover)).disabled(state.busy || state.loading)
                    .child(self.cross_project_check(checked))
                    .child(div().flex_1().min_w_0().text_color(p.text).text_size(px(12.))
                        .child(div().truncate().child(skill.name.clone()))
                        .child(div().mt(px(3.)).text_size(px(10.)).text_color(p.muted).truncate().child(source)))
                    .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.cross_project.skills.remove(&key) { this.cross_project.skills.insert(key.clone()); }
                        this.update_cross_project_plan(cx);
                    })));
            }
        }
        if visible.is_empty() { list = list.child(div().p(px(12.)).text_size(px(12.)).text_color(p.muted).child(self.tr("没有匹配的技能", "No matching skills"))); }
        let all = !visible.is_empty() && visible.iter().all(|key| state.skills.contains(key));
        div().flex_1().min_w_0().rounded(px(10.)).border_1().border_color(p.border).overflow_hidden()
            .child(div().p(px(10.)).child(div().flex().items_center()
                .child(div().flex_1().text_size(px(13.)).font_weight(FontWeight::MEDIUM).child(self.tr("选择技能", "Skills")))
                .child(div().text_size(px(11.)).text_color(p.muted).child(format!("{} / {}", state.skills.len(), visible.len()))))
                .child(Input::new(&state.skill_search).small().w_full().mt(px(8.)).prefix(Self::icon("icons/search.svg", 14., p.muted)))
                .child(self.toolbar_text_button("cross-project-select-skills", if all { self.tr("取消当前结果", "Deselect visible") } else { self.tr("全选当前结果", "Select visible") })
                    .debug_selector(|| "cross-project-select-skills".into()).mt(px(5.)).disabled(state.busy || state.loading || visible.is_empty())
                    .on_click(cx.listener(move |this, _, _, cx| { toggle_visible(&mut this.cross_project.skills, &visible); this.update_cross_project_plan(cx); }))))
            .child(list)
    }

    fn cross_project_projects_picker(&self, height: f32, cx: &mut Context<Self>) -> Div {
        let p = self.palette(); let state = &self.cross_project;
        let visible = self.cross_project_visible_projects(cx);
        let ordinary = state.agents.iter().any(|id| id != "proma"); let proma = state.agents.contains("proma");
        let mut selectable_projects = Vec::new(); let mut selectable_workspaces = Vec::new();
        let mut list = div().id("cross-project-projects-list").h(px(height)).overflow_y_scroll().track_scroll(&state.project_scroll);
        if let Some(catalog) = &state.catalog {
            for (index, path) in visible.iter().enumerate() {
                let Some(project) = catalog.projects.iter().find(|p| &p.path == path) else { continue; };
                let query = state.project_search.read(cx).value().trim().to_lowercase();
                let project_matches = query.is_empty() || project.name.to_lowercase().contains(&query) || path.to_string_lossy().to_lowercase().contains(&query);
                let ids = catalog.workspace_ids(path).into_iter().filter(|id| project_matches || catalog.proma.as_ref().is_some_and(|p| p.workspaces.iter().any(|w| {
                    &w.workspace.id == id && format!("{} {} {}", w.workspace.name, w.workspace.slug, w.workspace.path.display()).to_lowercase().contains(&query)
                }))).collect::<Vec<_>>();
                let checked = state.projects.contains(path) && ordinary; let unavailable = !path.is_dir();
                if ordinary && !unavailable { selectable_projects.push(path.clone()); }
                if proma { selectable_workspaces.extend(ids.iter().cloned()); }
                let project_path = path.clone(); let expanded = !state.collapsed.contains(path);
                let mut heading = div().flex().items_center().gap(px(5.)).px(px(5.));
                if proma && !ids.is_empty() {
                    let expand_path = path.clone();
                    heading = heading.child(self.toolbar_text_button(ElementId::Name(format!("cross-project-expand-{index}").into()), if expanded { "⌄" } else { "›" })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !this.cross_project.collapsed.remove(&expand_path) { this.cross_project.collapsed.insert(expand_path.clone()); }
                            this.notify_dialog(cx);
                        })));
                }
                heading = heading.child(gpui_base::Button::new(ElementId::Name(format!("cross-project-project-{index}").into()))
                    .accessibility_label(format!("{} · {}", project.name, if checked { self.tr("已选项目", "Selected project") } else { self.tr("项目", "Project") }))
                    .w_full().min_w_0().border_0().px(px(5.)).py(px(8.)).rounded(px(6.)).flex().items_center().gap(px(9.))
                    .bg(if checked { p.selected } else { rgba(0x00000000) }).hover(move |d| d.bg(p.hover))
                    .disabled(state.busy || state.loading || unavailable || !ordinary)
                    .when(ordinary, |d| d.child(self.cross_project_check(checked)))
                    .child(div().flex_1().min_w_0().text_color(if unavailable { p.muted } else { p.text }).text_size(px(12.))
                        .child(div().truncate().child(format!("{}{}", project.name, if !project.is_active() { self.tr(" · 已归档/隐藏", " · Archived/hidden") } else { "" })))
                        .child(div().mt(px(3.)).text_size(px(10.)).text_color(p.muted).truncate().child(display_path(path))))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.cross_project.projects.remove(&project_path) { this.cross_project.projects.insert(project_path.clone()); }
                        this.update_cross_project_plan(cx);
                    })));
                list = list.child(heading);
                if proma && expanded {
                    if ids.is_empty() {
                        list = list.child(div().pl(px(36.)).pb(px(6.)).text_size(px(10.)).text_color(p.muted).child(self.tr("未关联 Proma 工作区", "No linked Proma workspace")));
                    }
                    for id in ids {
                        let Some(entry) = catalog.proma.as_ref().and_then(|c| c.workspaces.iter().find(|w| w.workspace.id == id)) else { continue; };
                        let checked = state.workspaces.contains(&id); let has_error = entry.error.is_some(); let key = id.clone();
                        list = list.child(gpui_base::Button::new(ElementId::Name(format!("cross-project-workspace-{id}").into()))
                            .debug_selector(move || format!("cross-project-workspace-{key}"))
                            .accessibility_label(format!("Proma · {} · {}", entry.workspace.name, if checked { self.tr("已选工作区", "Selected workspace") } else { self.tr("工作区", "Workspace") }))
                            .w_full().border_0().pl(px(35.)).pr(px(10.)).py(px(6.)).flex().items_center().gap(px(9.))
                            .bg(if checked { p.selected } else { rgba(0x00000000) }).hover(move |d| d.bg(p.hover)).disabled(state.busy || state.loading || has_error)
                            .child(self.cross_project_check(checked))
                            .child(div().flex_1().min_w_0().text_size(px(11.)).text_color(if has_error { p.muted } else { p.secondary })
                                .child(div().truncate().child(format!("Proma · {}{}", entry.workspace.name, if entry.workspace.archived { self.tr(" · 已归档", " · Archived") } else { "" })))
                                .child(div().text_size(px(10.)).mt(px(2.)).text_color(p.muted).truncate().child(entry.error.clone().unwrap_or_else(|| entry.workspace.slug.clone()))))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !this.cross_project.workspaces.remove(&id) { this.cross_project.workspaces.insert(id.clone()); }
                                this.update_cross_project_plan(cx);
                            })));
                    }
                }
            }
        }
        if visible.is_empty() { list = list.child(div().p(px(12.)).text_size(px(12.)).text_color(p.muted).child(self.tr("没有匹配的项目", "No matching projects"))); }
        let all = (!selectable_projects.is_empty() || !selectable_workspaces.is_empty())
            && selectable_projects.iter().all(|p| state.projects.contains(p)) && selectable_workspaces.iter().all(|id| state.workspaces.contains(id));
        div().flex_1().min_w_0().rounded(px(10.)).border_1().border_color(p.border).overflow_hidden()
            .child(div().p(px(10.)).child(div().flex().items_center()
                .child(div().flex_1().text_size(px(13.)).font_weight(FontWeight::MEDIUM).child(self.tr("选择项目 / 工作区", "Projects / workspaces")))
                .child(div().text_size(px(11.)).text_color(p.muted).child(if self.uses_english() { format!("{} projects · {} workspaces", if ordinary { state.projects.len() } else { 0 }, if proma { state.workspaces.len() } else { 0 }) }
                    else { format!("{} 项目 · {} 工作区", if ordinary { state.projects.len() } else { 0 }, if proma { state.workspaces.len() } else { 0 }) })))
                .child(Input::new(&state.project_search).small().w_full().mt(px(8.)).prefix(Self::icon("icons/search.svg", 14., p.muted)))
                .child(self.toolbar_text_button("cross-project-select-projects", if all { self.tr("取消当前结果", "Deselect visible") } else { self.tr("全选当前结果", "Select visible") })
                    .debug_selector(|| "cross-project-select-projects".into()).mt(px(5.)).disabled(state.busy || state.loading || (!ordinary && !proma))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if all { this.cross_project.projects.retain(|p| !selectable_projects.contains(p)); this.cross_project.workspaces.retain(|id| !selectable_workspaces.contains(id)); }
                        else { this.cross_project.projects.extend(selectable_projects.iter().cloned()); this.cross_project.workspaces.extend(selectable_workspaces.iter().cloned()); }
                        this.update_cross_project_plan(cx);
                    }))))
            .child(list)
    }

    fn cross_project_items(&self, plan: &Plan, result: Option<&BatchResult>) -> impl IntoElement {
        let p = self.palette();
        let mut list = div().id("cross-project-impact-list").max_h(px(340.)).overflow_y_scroll();
        for (index, item) in plan.items.iter().enumerate() {
            let outcome = result.and_then(|r| r.outcomes.get(index));
            let status = outcome.map(|o| if o.error.is_some() { self.tr("失败", "Failed").into() } else if o.changed { self.tr("成功", "Succeeded").into() }
                else { self.cross_project_status(if item.disposition == Disposition::Change { Disposition::Unchanged } else { item.disposition }, plan.action) }).unwrap_or_else(|| self.cross_project_status(item.disposition, plan.action));
            let agents = item.affected_agents.iter().map(|a| a.name).collect::<Vec<_>>().join("、");
            let other_agents = item.affected_agents.iter().any(|a| !self.cross_project.agents.contains(a.id));
            let reason = outcome.and_then(|o| o.error.clone()).or(item.reason.clone());
            let path_tip = display_path(&item.path);
            list = list.child(div().py(px(8.)).border_b_1().border_color(p.border)
                .child(div().flex().items_center().gap(px(8.)).text_size(px(12.))
                    .child(div().min_w_0().flex_1().truncate().child(format!("{} · {}", item.skill.name, item.location)))
                    .child(div().text_color(if outcome.is_some_and(|o| o.error.is_some()) { p.danger } else { p.secondary }).child(status)))
                .child(div().mt(px(3.)).text_size(px(11.)).text_color(if other_agents { p.warning } else { p.secondary })
                    .child(format!("{}：{agents}{}", self.tr("影响 Agent", "Affected agents"), if other_agents { self.tr("（包含共享目录使用者）", " (includes shared readers)") } else { "" })))
                .child(div().id(ElementId::Name(format!("cross-project-path-{index}").into())).mt(px(3.)).truncate().font_family(MONO).text_size(px(10.)).text_color(p.muted)
                    .child(display_path(&item.path)).tooltip(move |window, cx| Tooltip::new(path_tip.clone()).build(window, cx)))
                .when(reason.is_some(), |d| d.child(div().mt(px(3.)).text_size(px(11.)).text_color(p.muted).child(reason.unwrap_or_default()))));
        }
        list
    }

    pub(super) fn cross_project_modal(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let p = self.palette(); let state = &self.cross_project;
        let mut background = p.elevated; background.a = 1.;
        let busy = state.loading || state.busy;
        let mut content = div().id("cross-project-content").px(px(20.)).pb(px(12.))
            .max_h(px((f32::from(window.bounds().size.height) - 180.).max(200.))).overflow_y_scroll();
        if let Some(result) = &state.result {
            content = content.child(div().mb(px(8.)).text_size(px(13.)).text_color(if result.failed() > 0 { p.warning } else { p.success })
                .child(if self.uses_english() { format!("{} changed · {} failed · {} skipped", result.changed(), result.failed(), result.outcomes.len() - result.changed() - result.failed()) }
                    else { format!("成功 {} 份 · 失败 {} 份 · 跳过 {} 项", result.changed(), result.failed(), result.outcomes.len() - result.changed() - result.failed()) }));
            let result_plan = Plan { action: result.action, items: result.outcomes.iter().map(|o| o.item.clone()).collect() };
            content = content.child(self.cross_project_items(&result_plan, Some(result)));
        } else if state.reviewed {
            if let Some(plan) = &state.plan {
                content = content.child(div().mb(px(10.)).p(px(10.)).rounded(px(10.)).bg(p.raised).text_size(px(12.))
                    .child(if self.uses_english() { format!("{} to change · {} unchanged · {} missing · {} protected", plan.count(Disposition::Change), plan.count(Disposition::Unchanged), plan.count(Disposition::Missing), plan.count(Disposition::Protected)) }
                        else { format!("将{} {} 份 · 无需变更 {} · 未安装 {} · 受保护 {}", self.cross_project_verb(state.action), plan.count(Disposition::Change), plan.count(Disposition::Unchanged), plan.count(Disposition::Missing), plan.count(Disposition::Protected)) }))
                    .child(div().mb(px(6.)).text_size(px(12.)).font_weight(FontWeight::MEDIUM).child(self.tr("实际影响清单", "Actual impact")))
                    .child(self.cross_project_items(plan, None));
            }
        } else {
            let mut actions = div().flex().items_center().gap(px(5.));
            for (index, action) in [Action::Install, Action::Remove, Action::Enable, Action::Disable].into_iter().enumerate() {
                let supported = !action.is_state() || action.supported(&state.agents);
                actions = actions.child(self.toolbar_text_button(["cross-project-install", "cross-project-remove", "cross-project-enable", "cross-project-disable"][index], self.cross_project_verb(action))
                    .debug_selector(move || ["cross-project-install", "cross-project-remove", "cross-project-enable", "cross-project-disable"][index].into())
                    .bg(if state.action == action { p.selected } else { rgba(0x00000000) }).disabled(busy || !supported)
                    .tooltip(move |window, cx| Tooltip::new(if action.is_state() { "启用、停用目前仅支持 Proma 已有副本；混选其他 Agent 时不可用" } else { "选择 Agent 与项目后，先查看实际影响清单" }).build(window, cx))
                    .on_click(cx.listener(move |this, _, _, cx| { this.cross_project.action = action; this.update_cross_project_plan(cx); })));
            }
            content = content.child(actions).child(div().mt(px(8.)).text_size(px(11.)).text_color(p.muted)
                .child(self.tr("Agent · 已安装的在前，灰色项不可选 · 启停仅支持 Proma", "Agents · installed first, grey items unavailable · state changes require Proma")));
            let mut agents = crate::agents::AGENT_ICON_ORDER.to_vec();
            agents.sort_by_key(|a| crate::agents::display_sort_key(a.name, self.model.installed_agents.contains(a)));
            let mut agent_picker = div().mt(px(5.)).flex().flex_wrap().gap(px(4.));
            for agent in agents {
                let installed = self.model.installed_agents.contains(&agent); let available = installed && !agent.global_only;
                let checked = state.agents.contains(agent.id);
                let label = if !installed { self.tr("未安装", "Not installed") } else if agent.global_only { self.tr("仅支持全局", "Global only") } else { "" };
                let tip = format!("{}{}", agent.name, if label.is_empty() { String::new() } else { format!(" · {label}") });
                agent_picker = agent_picker.child(gpui_base::Button::new(ElementId::Name(format!("cross-project-agent-{}", agent.id).into()))
                    .debug_selector(move || format!("cross-project-agent-{}", agent.id)).accessibility_label(tip.clone())
                    .h(px(CONTROL_HEIGHT)).px(px(8.)).border_0().rounded(px(RADIUS_CONTROL)).flex().items_center().gap(px(6.))
                    .bg(if checked { p.selected } else { rgba(0x00000000) }).text_color(if available { p.text } else { p.muted })
                    .text_size(px(12.)).font_family(FONT_UI).font_weight(FontWeight::NORMAL).hover(move |d| d.bg(p.hover)).disabled(busy || !available)
                    .child(self.cross_project_check(checked)).child(self.agent_status_icon(agent.icon_path, 14., available))
                    .child(agent.name).tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.cross_project.agents.remove(agent.id) { this.cross_project.agents.insert(agent.id.into()); }
                        this.update_cross_project_plan(cx);
                    })));
            }
            content = content.child(agent_picker)
                .child(div().mt(px(8.)).mb(px(8.)).text_size(px(11.)).text_color(p.secondary)
                    .child(self.tr("项目勾选作用于普通 Agent；Proma 请在项目下单独勾选工作区。全局继承项在全局页面管理。", "Project selections apply to other agents; select Proma workspace children separately. Manage inherited global installs in the global view.")))
                .child(div().flex().gap(px(12.))
                    .child(self.cross_project_skills_picker((f32::from(window.bounds().size.height) - 520.).clamp(135., 260.), cx))
                    .child(self.cross_project_projects_picker((f32::from(window.bounds().size.height) - 520.).clamp(135., 260.), cx)));
            if let Some(error) = &state.error {
                content = content.child(div().mt(px(8.)).text_size(px(12.)).text_color(p.warning)
                    .child(if state.agents.is_empty() { self.tr("请选择至少一个已安装的 Agent", "Select at least one installed agent").to_string() } else { error.clone() }));
            }
            if let Some(plan) = &state.plan {
                content = content.child(div().mt(px(10.)).p(px(10.)).rounded(px(10.)).bg(p.raised).text_size(px(12.))
                    .child(if self.uses_english() { format!("{} to change · {} unchanged · {} missing · {} protected · {} unavailable", plan.count(Disposition::Change), plan.count(Disposition::Unchanged), plan.count(Disposition::Missing), plan.count(Disposition::Protected), plan.count(Disposition::Unavailable)) }
                        else { format!("待{} {} 份 · 无需变更 {} · 未安装 {} · 受保护 {} · 无法扫描 {}", self.cross_project_verb(state.action), plan.count(Disposition::Change), plan.count(Disposition::Unchanged), plan.count(Disposition::Missing), plan.count(Disposition::Protected), plan.count(Disposition::Unavailable)) }));
            }
        }
        let retry = state.result.as_ref().is_some_and(|r| r.failed() > 0);
        let count = if retry { state.result.as_ref().map(BatchResult::failed).unwrap_or(0) }
            else if state.result.is_some() { 0 } else { state.plan.as_ref().map(|p| p.count(Disposition::Change)).unwrap_or(0) };
        let label = if state.busy { self.tr("正在执行…", "Applying…").to_string() }
            else if state.loading || state.planning { self.tr("正在计算…", "Calculating…").to_string() }
            else if retry { if self.uses_english() { format!("Retry {count} failed") } else { format!("仅重试失败 {count} 份") } }
            else if state.result.is_some() { self.tr("已完成", "Done").into() }
            else if !state.reviewed { self.tr("预览更改", "Review changes").into() }
            else if self.uses_english() { format!("{} {count} installations", self.cross_project_verb(state.action)) }
            else { format!("确认{} {count} 份", self.cross_project_verb(state.action)) };
        div().bg(background).text_color(p.text).font_family(FONT_UI)
            .child(div().px(px(20.)).pt(px(16.)).pb(px(10.))
                .child(div().flex().items_center().gap(px(6.))
                    .child(div().flex_1().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child(self.tr("跨项目管理", "Manage across projects")))
                    .child(self.toolbar_text_button("cross-project-refresh", self.tr("刷新", "Refresh")).disabled(busy).on_click(cx.listener(|this, _, _, cx| this.load_cross_project(cx))))
                    .child(self.toolbar_text_button("cross-project-close", self.tr("关闭", "Close")).disabled(state.busy).on_click(cx.listener(|this, _, _, cx| this.close_dialog(cx)))))
                .child(div().mt(px(7.)).text_size(px(12.)).text_color(p.secondary).child(self.cross_project_explanation())))
            .child(content)
            .child(div().px(px(20.)).py(px(12.)).border_t_1().border_color(p.border).flex().items_center().gap(px(7.))
                .child(div().min_w_0().flex_1().text_size(px(11.)).text_color(p.muted).child(if self.uses_english() { format!("{} skills · {} agents", state.skills.len(), state.agents.len()) }
                    else { format!("已选 {} 个技能 · {} 个 Agent", state.skills.len(), state.agents.len()) }))
                .when(state.result.is_some() || state.reviewed, |d| d.child(self.toolbar_text_button("cross-project-back", self.tr("返回选择", "Back to selection")).disabled(busy)
                    .on_click(cx.listener(|this, _, _, cx| {
                        if this.cross_project.result.is_some() { this.load_cross_project(cx); }
                        else { this.cross_project.reviewed = false; this.notify_dialog(cx); }
                    }))))
                .child(Button::new("cross-project-execute").debug_selector(|| "cross-project-execute".into()).small().primary()
                    .h(px(CONTROL_HEIGHT)).font_family(FONT_UI).font_weight(FontWeight::NORMAL).text_size(px(12.)).label(label)
                    .disabled(busy || state.planning || if !state.reviewed && state.result.is_none() { state.plan.as_ref().is_none_or(|p| p.items.is_empty()) } else { count == 0 }).on_click(cx.listener(move |this, _, _, cx| {
                        if !this.cross_project.reviewed { this.cross_project.reviewed = true; this.notify_dialog(cx); }
                        else { this.execute_cross_project(retry, cx); }
                    }))))
    }
}

#[cfg(test)]
mod tests {
    use super::toggle_visible;
    use std::collections::BTreeSet;
    #[test]
    fn select_visible_preserves_hidden_selection() {
        let mut selected = BTreeSet::from(["hidden".to_string()]);
        let visible = vec!["first".to_string(), "second".to_string()];
        toggle_visible(&mut selected, &visible); assert_eq!(selected.len(), 3);
        toggle_visible(&mut selected, &visible); assert_eq!(selected, BTreeSet::from(["hidden".to_string()]));
    }

    #[cfg(feature = "ui-test")]
    #[test]
    fn mixed_agents_require_review_and_proma_workspaces_are_explicit() {
        use super::*;
        use std::fs;
        let temp = tempfile::tempdir().unwrap(); let base = temp.path().canonicalize().unwrap();
        let source = base.join("source/alpha"); let project = base.join("Project A");
        fs::create_dir_all(&source).unwrap(); fs::create_dir(&project).unwrap();
        fs::write(source.join("SKILL.md"), "---\nname: alpha\ndescription: Test\n---\nBody\n").unwrap();
        let proma = base.join("proma"); fs::create_dir_all(proma.join("agent-workspaces/a")).unwrap();
        fs::write(proma.join("agent-workspaces.json"), serde_json::to_vec(&serde_json::json!({"version":2,"workspaces":[
            {"id":"a","slug":"a","name":"Workspace A","projectRootPath":project}
        ]})).unwrap()).unwrap();
        crate::proma::apply_plan(&crate::proma::plan_install(&proma, &proma.join("agent-workspaces/a"), &source, "alpha").unwrap()).unwrap();
        let installed = crate::agents::AGENT_ICON_ORDER.iter().filter(|a| ["codex","proma"].contains(&a.id)).copied().collect::<Vec<_>>();
        let skill = Skill { key: crate::proma_batch::source_key(&source.join("SKILL.md")), name:"alpha".into(), source:source.clone(), provider:"library", install_name:Some("alpha".into()) };
        let entry = crate::project_discovery::ProjectEntry { path:project.clone(), name:"Project A".into(), agents:vec![], proma_workspaces:vec![], archived:false, hidden:false, retained:false };
        let catalog = Catalog::scan(vec![skill.clone()], vec![entry], installed.clone(), Some(&proma));
        let mut test = TestAppContext::single(); test.update(gpui_component::init);
        let (app, cx) = test.add_window_view({ let data = base.join("data"); move |window, cx| KitterApp::new_in(data, window, cx) });
        app.update_in(cx, |app, _, cx| {
            app.model.installed_agents = installed;
            app.cross_project.catalog = Some(Arc::new(catalog));
            app.cross_project.skills.insert(skill.key);
            app.cross_project.agents = BTreeSet::from(["codex".into(),"proma".into()]);
            app.cross_project.projects.insert(project.clone());
            app.open_dialog(DialogKind::CrossProject, cx); app.update_cross_project_plan(cx);
        });
        cx.run_until_parked(); cx.refresh().unwrap();
        cx.read_entity(&app, |app, _| {
            assert_eq!(app.cross_project.plan.as_ref().unwrap().count(Disposition::Change), 1);
            assert!(app.cross_project.workspaces.is_empty());
            assert!(!app.cross_project.reviewed);
        });
        // A mixed selection cannot activate the Proma-only operation.
        let button = cx.debug_bounds("cross-project-enable").unwrap();
        cx.simulate_click(button.center(), Modifiers::none());
        cx.read_entity(&app, |app, _| assert_eq!(app.cross_project.action, Action::Install));
        // First click only reveals the frozen impact list.
        let button = cx.debug_bounds("cross-project-execute").unwrap();
        cx.simulate_click(button.center(), Modifiers::none());
        cx.run_until_parked(); cx.refresh().unwrap();
        assert!(!project.join(".codex/skills/alpha").exists());
        cx.read_entity(&app, |app, _| assert!(app.cross_project.reviewed));
        let button = cx.debug_bounds("cross-project-execute").unwrap();
        cx.simulate_click(button.center(), Modifiers::none());
        cx.run_until_parked(); cx.refresh().unwrap();
        cx.read_entity(&app, |app, _| { let r = app.cross_project.result.as_ref().unwrap(); assert_eq!((r.changed(),r.failed()),(1,0)); });
        assert!(project.join(".codex/skills/alpha").is_symlink());
        assert!(proma.join("agent-workspaces/a/skills/alpha").exists());
        assert!(source.join("SKILL.md").is_file());
        app.update_in(cx, |app, _, cx| {
            app.cross_project.agents = BTreeSet::from(["proma".into()]);
            app.cross_project.workspaces = BTreeSet::from(["a".into()]);
            app.cross_project.action = Action::Disable; app.cross_project.result = None;
            app.update_cross_project_plan(cx);
        });
        cx.run_until_parked(); cx.refresh().unwrap();
        let button = cx.debug_bounds("cross-project-execute").unwrap(); cx.simulate_click(button.center(), Modifiers::none());
        cx.run_until_parked(); cx.refresh().unwrap();
        let button = cx.debug_bounds("cross-project-execute").unwrap(); cx.simulate_click(button.center(), Modifiers::none());
        cx.run_until_parked(); cx.refresh().unwrap();
        cx.read_entity(&app, |app, _| { let r=app.cross_project.result.as_ref().unwrap(); assert_eq!((r.changed(),r.failed()),(1,0)); });
        assert!(proma.join("agent-workspaces/a/skills-inactive/alpha/SKILL.md").is_file());
    }
}
