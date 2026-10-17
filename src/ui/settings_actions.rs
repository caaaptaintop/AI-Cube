use super::*;
use crate::library::{StorageAction, StoragePlan};

#[derive(Default)]
pub(super) struct StorageState {
    pub advanced: bool,
    pub action: Option<StorageAction>,
    pub plan: Option<StoragePlan>,
    pub error: Option<String>,
    pub busy: bool,
}

impl KitterApp {
    fn begin_storage_change(&mut self, action: StorageAction, cx: &mut Context<Self>) -> bool {
        if self.model.proma_sync_running
            || self.model.update_check.is_some()
            || self.model.updating_skill.is_some()
            || self.cross_project_busy()
        {
            self.show_notice(
                self.tr(
                    "正在处理技能，请完成后再更换位置",
                    "Wait for skill operations to finish before changing storage",
                ),
                cx,
            );
            return false;
        }
        self.settings_storage.action = Some(action);
        self.settings_storage.plan = None;
        self.settings_storage.error = None;
        self.open_dialog(DialogKind::Storage, cx);
        true
    }

    pub(super) fn choose_storage_target(
        &mut self,
        action: StorageAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.begin_storage_change(action, cx) {
            return;
        }
        let dialog_id = self.shell.dialog_body.as_ref().unwrap().entity_id();
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(self.tr("选择文件夹", "Choose folder").into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = receiver.await
                && let Some(path) = paths.into_iter().next()
            {
                let _ = this.update(cx, |this, cx| {
                    this.accept_storage_target(dialog_id, action, &path, cx);
                });
            }
        })
        .detach();
    }

    fn accept_storage_target(
        &mut self,
        dialog_id: gpui::EntityId,
        action: StorageAction,
        target: &Path,
        cx: &mut Context<Self>,
    ) {
        if self.settings_storage.action == Some(action)
            && self.shell.dialog_body.as_ref().is_some_and(|body| {
                body.entity_id() == dialog_id && matches!(body.read(cx).kind, DialogKind::Storage)
            })
        {
            self.preview_storage_change(action, target, cx);
        }
    }

    fn preview_storage_change(
        &mut self,
        action: StorageAction,
        target: &Path,
        cx: &mut Context<Self>,
    ) {
        match self.model.library.plan_storage_change(action, target) {
            Ok(plan) => {
                self.settings_storage.plan = Some(plan);
                self.settings_storage.error = None;
            }
            Err(error) => {
                self.settings_storage.plan = None;
                self.settings_storage.error = Some(error.to_string());
            }
        }
        self.notify_dialog(cx);
    }

    pub(super) fn preview_pleat_data_migration(&mut self, cx: &mut Context<Self>) {
        if !self.begin_storage_change(StorageAction::MoveAppData, cx) {
            return;
        }
        let target = self.model.library.data_dir().with_file_name("Pleat");
        self.preview_storage_change(StorageAction::MoveAppData, &target, cx);
    }

    fn apply_storage_change(&mut self, cx: &mut Context<Self>) {
        let Some(plan) = self.settings_storage.plan.clone() else {
            return;
        };
        if self.settings_storage.busy
            || self.model.proma_sync_running
            || self.settings_storage.action != Some(plan.action)
            || !self
                .shell
                .dialog_body
                .as_ref()
                .is_some_and(|body| matches!(body.read(cx).kind, DialogKind::Storage))
        {
            return;
        }
        self.settings_storage.busy = true;
        match self.model.library.apply_storage_change(&plan) {
            Ok(()) => {
                self.settings_storage.busy = false;
                self.settings_storage.plan = None;
                self.close_dialog(cx);
                self.refresh(cx);
                self.show_notice(
                    self.tr(
                        "存储位置已更新，原有文件与安装保留",
                        "Storage updated; existing files and installations retained",
                    ),
                    cx,
                );
            }
            Err(error) => {
                self.settings_storage.busy = false;
                self.settings_storage.plan = None;
                self.settings_storage.error = Some(error.to_string());
                self.notify_dialog(cx);
            }
        }
    }

    pub(super) fn storage_modal(&self, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let action = self
            .settings_storage
            .action
            .unwrap_or(StorageAction::MoveLibrary);
        let (title, explanation, apply_label) = match action {
            StorageAction::MoveLibrary => (
                self.tr("迁移当前技能库", "Move current library"),
                self.tr("将技能和本机配套一起迁移到空文件夹；原位置保留兼容入口，已有安装继续可用。跨磁盘或目标已有内容时不会执行。", "Move skills and local resources to an empty folder, retaining a compatibility entry at the old location. Cross-volume moves and occupied targets are refused."),
                self.tr("确认迁移", "Confirm move"),
            ),
            StorageAction::UseLibrary => (
                self.tr("使用已有技能库", "Use existing library"),
                self.tr("读取已有技能目录，保留当前库及项目安装。不会把原库的远端来源套用到不同文件；原登记先保全，以便恢复。", "Use an existing skill folder while retaining the current library and project installations. Preserve original metadata; never apply old remote origins to different files."),
                self.tr("确认使用", "Confirm selection"),
            ),
            StorageAction::MoveAppData => (
                self.tr("迁移应用数据到 Pleat", "Move app data to Pleat"),
                self.tr("迁移应用配置、登记、管理资料和内部技能库，保留原 Kitter 路径兼容入口。现有脚本、技能引用和本机环境继续通过原路径访问；外部技能源保持原位。", "Move app settings, registry, support files and the internal library. Retain the old Kitter entry for scripts and local runtimes; external skill sources stay in place."),
                self.tr("确认迁移", "Confirm move"),
            ),
        };
        let mut content = div()
            .p(px(20.))
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(
                div()
                    .text_size(px(16.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(p.secondary)
                    .child(explanation),
            );
        if let Some(plan) = &self.settings_storage.plan {
            for (label, path) in [
                (self.tr("当前位置", "Current location"), &plan.from),
                (self.tr("目标位置", "Destination"), &plan.to),
            ] {
                content = content.child(
                    div()
                        .p(px(12.))
                        .rounded(px(RADIUS_CONTROL))
                        .bg(p.raised)
                        .child(div().text_size(px(12.)).text_color(p.muted).child(label))
                        .child(
                            div()
                                .mt(px(5.))
                                .font_family(MONO)
                                .text_size(px(12.))
                                .child(display_path(path)),
                        ),
                );
            }
            content = content.child(div().text_size(px(12.)).text_color(p.secondary).child(
                if self.uses_english() {
                    format!(
                        "{} skills · {}",
                        plan.skill_count,
                        if plan.compatibility_alias {
                            "retain the old path"
                        } else {
                            "retain the original library"
                        }
                    )
                } else {
                    format!(
                        "{} 个技能 · {}",
                        plan.skill_count,
                        if plan.compatibility_alias {
                            "保留旧路径兼容入口"
                        } else {
                            "原技能库保留"
                        }
                    )
                },
            ));
        } else if self.settings_storage.error.is_none() {
            content = content.child(div().text_size(px(13.)).text_color(p.muted).child(self.tr(
                "请选择目标文件夹，预览后再确认。",
                "Choose a target folder and review the plan before confirming.",
            )));
        }
        if let Some(error) = &self.settings_storage.error {
            content = content.child(
                div()
                    .text_size(px(12.))
                    .text_color(p.danger)
                    .child(error.clone()),
            );
        }
        div().bg(p.elevated).child(content).child(
            div()
                .px(px(20.))
                .py(px(12.))
                .border_t_1()
                .border_color(p.border)
                .flex()
                .items_center()
                .justify_end()
                .gap(px(8.))
                .when(action != StorageAction::MoveAppData, |row| {
                    row.child(
                        self.toolbar_text_button(
                            "storage-choose-target",
                            self.tr("选择文件夹", "Choose folder"),
                        )
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.choose_storage_target(action, window, cx)
                            },
                        )),
                    )
                })
                .child(
                    self.toolbar_text_button("storage-cancel", self.tr("取消", "Cancel"))
                        .disabled(self.settings_storage.busy)
                        .on_click(cx.listener(|this, _, _, cx| this.close_dialog(cx))),
                )
                .child(
                    self.toolbar_text_button("storage-apply", apply_label)
                        .bg(p.selected)
                        .font_weight(FontWeight::MEDIUM)
                        .disabled(
                            self.settings_storage.plan.is_none() || self.settings_storage.busy,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.apply_storage_change(cx))),
                ),
        )
    }
}

#[cfg(all(test, feature = "ui-test"))]
mod tests {
    use super::{KitterApp, StorageAction};
    use gpui::TestAppContext;
    use std::fs;

    #[test]
    fn app_data_move_requires_confirmation_and_refreshes_the_open_library() {
        let mut test = TestAppContext::single();
        test.update(gpui_component::init);
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let original = root.join("Kitter");
        let destination = root.join("Pleat");
        let (app, cx) = test.add_window_view({
            let original = original.clone();
            move |window, cx| KitterApp::new_in(original, window, cx)
        });
        app.update(cx, |app, cx| {
            app.model.library.save().unwrap();
            let config_before = fs::read(original.join("config.json")).unwrap();
            app.preview_pleat_data_migration(cx);
            assert!(app.settings_storage.plan.is_some());
            let _ = app.storage_modal(cx);
            assert!(original.is_dir());
            assert!(!destination.exists());
            assert_eq!(
                fs::read(original.join("config.json")).unwrap(),
                config_before
            );
            app.apply_storage_change(cx);
            assert!(
                app.settings_storage.error.is_none(),
                "{:?}",
                app.settings_storage.error
            );
            assert_eq!(app.model.library.data_dir(), destination);
            assert!(original.is_dir());
            assert!(
                destination
                    .join("skills")
                    .join(crate::library::KITTER_SKILL_STORAGE)
                    .join("SKILL.md")
                    .is_file()
            );
            assert_eq!(
                app.model.library.config.library_dir,
                destination.join("skills")
            );
            assert!(!app.model.skills.is_empty());
            assert!(app.shell.dialog_body.is_none());
        });
    }

    #[test]
    fn storage_dialog_waits_for_an_active_mirror_sync() {
        let mut test = TestAppContext::single();
        test.update(gpui_component::init);
        let temp = tempfile::tempdir().unwrap();
        let (app, cx) = test.add_window_view({
            let data = temp.path().join("Kitter");
            move |window, cx| KitterApp::new_in(data, window, cx)
        });
        app.update(cx, |app, cx| {
            app.model.proma_sync_running = true;
            app.preview_pleat_data_migration(cx);
            assert!(app.settings_storage.plan.is_none());
            assert!(app.shell.dialog_body.is_none());
            assert!(!temp.path().join("Pleat").exists());
        });
    }

    #[test]
    fn cancelled_folder_picker_cannot_replace_a_new_storage_plan() {
        let mut test = TestAppContext::single();
        test.update(gpui_component::init);
        let temp = tempfile::tempdir().unwrap();
        let (app, cx) = test.add_window_view({
            let data = temp.path().join("Kitter");
            move |window, cx| KitterApp::new_in(data, window, cx)
        });
        app.update(cx, |app, cx| {
            app.model.library.save().unwrap();
            for action in [
                StorageAction::MoveLibrary,
                StorageAction::UseLibrary,
                StorageAction::MoveAppData,
            ] {
                assert!(app.begin_storage_change(StorageAction::MoveLibrary, cx));
                let old_dialog = app.shell.dialog_body.as_ref().unwrap().entity_id();
                app.close_dialog(cx);
                assert!(app.settings_storage.plan.is_none());
                assert!(app.settings_storage.action.is_none());
                assert!(app.begin_storage_change(action, cx));
                if action == StorageAction::MoveAppData {
                    app.preview_storage_change(action, &temp.path().join("Pleat"), cx);
                }
                let expected = app
                    .settings_storage
                    .plan
                    .as_ref()
                    .map(|p| (p.action, p.to.clone()));
                app.accept_storage_target(
                    old_dialog,
                    StorageAction::MoveLibrary,
                    &temp.path().join("Other"),
                    cx,
                );
                assert!(app.settings_storage.action == Some(action));
                assert!(
                    app.settings_storage
                        .plan
                        .as_ref()
                        .map(|p| (p.action, p.to.clone()))
                        == expected
                );
                app.close_dialog(cx);
                app.apply_storage_change(cx);
                assert!(!temp.path().join("Pleat").exists());
                assert!(!temp.path().join("Other").exists());
            }
        });
    }
}
