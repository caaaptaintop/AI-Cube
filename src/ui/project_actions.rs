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
    }

    pub(super) fn project_entries(&self) -> Vec<crate::project_discovery::ProjectEntry> {
        crate::project_discovery::merge_projects(
            &self.model.library.config.project_paths(),
            &self.projects_view.discovered_projects,
            &self.model.library.config.hidden_discovered_projects,
        )
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
