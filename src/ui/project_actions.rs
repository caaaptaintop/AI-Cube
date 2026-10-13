use super::*;

impl KitterApp {
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
