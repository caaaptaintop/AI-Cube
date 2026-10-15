use super::*;

impl KitterApp {
    pub(super) fn apply_project_proma_state(&mut self, skills: &[ProjectSkill], project: &Path, enabled: bool, cx: &mut Context<Self>) {
        let action = if enabled { ProjectBatchAction::EnableProma } else { ProjectBatchAction::DisableProma };
        if self.projects_view.batch_project.as_deref() != Some(project) || self.projects_view.batch_action != action {
            self.show_notice(self.tr("当前项目或操作已变化，请重新选择", "The project or action changed; select again"), cx);
            return;
        }
        let mut seen = HashSet::new();
        let mut changed = 0;
        let mut failures = Vec::new();
        for skill in skills {
            let mut succeeded = true;
            for installation in &skill.installations {
                if installation.target != InstallTarget::Proma || !seen.insert(installation.path.clone()) { continue; }
                match crate::proma::set_enabled_path(&installation.path, enabled) {
                    Ok(()) => changed += 1,
                    Err(error) => {
                        succeeded = false;
                        failures.push(format!("{}：{error:#}", skill.name));
                    }
                }
            }
            if succeeded { self.projects_view.batch_selected.remove(&skill.name); }
        }
        if changed > 0 { self.refresh(cx); }
        let verb = self.tr(if enabled { "启用" } else { "停用" }, if enabled { "enabled" } else { "disabled" });
        let mut message = if self.uses_english() { format!("{changed} Proma copies {verb}") }
            else { format!("已{verb} {changed} 个 Proma 副本") };
        if !failures.is_empty() {
            message.push_str(&format!("\n{}", failures.join("\n")));
            self.show_sticky_notice(message, cx);
        } else { self.show_notice(message, cx); }
    }

    pub(super) fn normalize_open_project(&mut self) {
        if let Some(path) = &self.projects_view.open_project {
            let identity = crate::project_discovery::project_identity(
                path, &self.projects_view.discovered_projects,
            );
            if &identity != path {
                self.projects_view.open_project = Some(identity);
                self.projects_view.batch_project = None;
                self.projects_view.batch_selected.clear();
                self.projects_view.batch_filter = None;
            }
        }
        if self.projects_view.list_view == ProjectListView::Active
            && self.all_project_entries().iter().any(|entry| Some(&entry.path) == self.projects_view.open_project.as_ref() && !entry.is_active())
        {
            self.projects_view.open_project = None;
            self.projects_view.global_project_view = true;
            self.projects_view.batch_project = None;
            self.projects_view.batch_selected.clear();
            self.projects_view.batch_filter = None;
        }
    }

    pub(super) fn project_entries(&self) -> Vec<crate::project_discovery::ProjectEntry> {
        self.all_project_entries().into_iter().filter(crate::project_discovery::ProjectEntry::is_active).collect()
    }

    pub(super) fn all_project_entries(&self) -> Vec<crate::project_discovery::ProjectEntry> {
        self.model.library.config.project_entries(&self.projects_view.discovered_projects)
    }

    pub(super) fn change_project_visibility(&mut self, path: &Path, action: ProjectVisibilityAction, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(action, ProjectVisibilityAction::Restore) && !path.is_dir() {
            self.show_notice(self.tr("项目目录不存在，暂时无法恢复", "The project folder is unavailable"), cx);
            return;
        }
        let before = self.model.library.config.clone();
        match action {
            ProjectVisibilityAction::Hide => self.model.library.config.remove_project(path),
            ProjectVisibilityAction::Restore => self.model.library.config.restore_project(path.to_path_buf()),
            ProjectVisibilityAction::FollowAgent => {
                let identity = crate::project_discovery::path_identity(path);
                self.model.library.config.retained_projects.retain(|project| crate::project_discovery::path_identity(project) != identity);
            }
        }
        if self.model.library.config.save_to(self.model.library.data_dir()).is_err() {
            self.model.library.config = before;
            self.show_notice(self.tr("无法保存项目显示设置，请重试", "Could not save project visibility; try again"), cx);
            return;
        }
        if matches!(action, ProjectVisibilityAction::Restore) {
            self.projects_view.list_view = ProjectListView::Active;
            self.projects_view.open_project = Some(path.to_path_buf());
            self.projects_view.global_project_view = false;
        } else if self.projects_view.open_project.as_deref() == Some(path) {
            self.projects_view.open_project = None;
            self.projects_view.global_project_view = true;
        }
        self.projects_view.batch_project = None;
        self.projects_view.batch_selected.clear();
        self.projects_view.batch_filter = None;
        self.sync_project_select(window, cx);
        let message = match action {
            ProjectVisibilityAction::Hide => self.tr("已在 Kitter 中隐藏，可从“已归档／隐藏”恢复", "Hidden in Kitter; restore from Archived / hidden"),
            ProjectVisibilityAction::Restore => self.tr("已恢复到活跃项目", "Restored to active projects"),
            ProjectVisibilityAction::FollowAgent => self.tr("已恢复跟随 Agent 归档状态", "Now following Agent archive status"),
        };
        self.show_notice(message, cx);
    }

    pub(super) fn browse_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let installing = self.install_flow.modal;
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: installing,
            prompt: Some(self.tr("选择", "Choose").into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = receiver.await {
                let _ = this.update(cx, |this, cx| {
                    if installing {
                        if !this.install_flow.modal { return; }
                        for path in paths {
                            let path = crate::project_discovery::project_identity(
                                &path, &this.projects_view.discovered_projects,
                            );
                            if dirs::home_dir().as_ref() == Some(&path) { continue; }
                            this.model.library.config.hidden_discovered_projects.retain(|hidden| {
                                crate::project_discovery::project_identity(
                                    hidden, &this.projects_view.discovered_projects,
                                ) != path
                            });
                            this.model.library.config.remember_project(path.clone());
                            this.install_flow.selected_projects.insert(path);
                        }
                        this.install_flow.global = false;
                        let _ = this.model.library.save();
                        this.notify_dialog(cx);
                        return;
                    }
                    let Some(path) = paths.into_iter().next() else { return; };
                    let path = crate::project_discovery::project_identity(
                        &path, &this.projects_view.discovered_projects,
                    );
                    if this.projects_view.open_project.as_ref() != Some(&path) {
                        this.projects_view.batch_project = None;
                        this.projects_view.batch_selected.clear();
                        this.projects_view.batch_filter = None;
                    }
                    this.projects_view.open_project = Some(path.clone());
                    this.projects_view.list_view = ProjectListView::Active;
                    if this.all_project_entries().iter().any(|entry| entry.path == path && !entry.is_active()) {
                        this.model.library.config.restore_project(path.clone());
                    }
                    if !this.install_flow.modal {
                        this.projects_view.global_project_view = false;
                        this.projects_view.selected_project_agent = None;
                        this.projects_view.project_agents_expanded = false;
                    }
                    this.model.library.config.hidden_discovered_projects.retain(|hidden| {
                        crate::project_discovery::project_identity(
                            hidden, &this.projects_view.discovered_projects,
                        ) != path
                    });
                    this.model.library.config.remember_project(path);
                    let _ = this.model.library.save();
                    if this.install_flow.modal {
                        this.install_flow.global =
                            dirs::home_dir().as_ref() == this.projects_view.open_project.as_ref();
                        if !this.can_install_proma() {
                            this.install_flow.selected_targets.remove(&InstallTarget::Proma);
                        }
                        this.notify_dialog(cx);
                    } else {
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }
}
