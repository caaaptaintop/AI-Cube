use super::*;

fn selected_project_skills(
    rows: &[EffectiveSkillRow],
    selected_names: &HashSet<String>,
    action: ProjectBatchAction,
) -> Vec<ProjectSkill> {
    rows.iter()
        .filter(|row| selected_names.contains(&row.name))
        .map(|row| ProjectSkill {
            name: row.name.clone(),
            installations: row.batch_installations(action),
        })
        .filter(|skill| !skill.installations.is_empty())
        .collect()
}

impl KitterApp {
    fn project_batch_action_label(&self, action: ProjectBatchAction) -> &'static str {
        match action {
            ProjectBatchAction::Remove => self.tr("移除安装", "Remove installs"),
            ProjectBatchAction::DisableProma => self.tr("停用 Proma", "Disable Proma"),
            ProjectBatchAction::EnableProma => self.tr("启用 Proma", "Enable Proma"),
        }
    }

    fn project_batch_unavailable_reason(&self, row: &EffectiveSkillRow) -> String {
        match self.projects_view.batch_action {
            ProjectBatchAction::Remove => {
                if row.direct_installations.iter().any(|item| item.bundled) {
                    self.tr("Proma 内置预设可停用，请切换到“停用 Proma”", "Switch to Disable Proma to deactivate built-in presets")
                } else if row.direct_installations.is_empty() {
                    self.tr("此项来自全局、父目录或系统；此项目没有可移除的安装", "Inherited or system skill; no removable installation in this project")
                } else {
                    self.tr("此安装受保护，不能从这里删除", "This installation is protected from removal")
                }
            }
            action => {
                let proma = row.direct_installations.iter().filter(|item| item.target == InstallTarget::Proma).collect::<Vec<_>>();
                if !cfg!(unix) {
                    self.tr("此平台尚未支持 Proma 技能启停", "Proma skill state changes are not supported on this platform")
                } else if proma.is_empty() {
                    self.tr("此项目没有该技能的 Proma 副本；全局和系统来源不受此操作影响", "No Proma copy in this project; inherited and system sources are unaffected")
                } else if proma.iter().any(|item| item.toggleable) {
                    if action == ProjectBatchAction::DisableProma {
                        self.tr("Proma 副本已停用；切换到“启用 Proma”可恢复", "Already inactive; switch to Enable Proma to restore")
                    } else {
                        self.tr("Proma 副本已启用；切换到“停用 Proma”可停用", "Already active; switch to Disable Proma to deactivate")
                    }
                } else {
                    self.tr("只支持已核验的内置预设和完好的 Kitter 受管副本；此副本受保护", "Only verified built-in presets and intact Kitter-managed copies can be changed")
                }
            }
        }.to_string()
    }

    fn project_batch_action_control(&self, has_proma: bool, cx: &mut Context<Self>) -> Popover {
        let p = self.palette();
        let app = cx.entity().downgrade();
        let selected = self.projects_view.batch_action;
        let actions = [ProjectBatchAction::Remove, ProjectBatchAction::DisableProma, ProjectBatchAction::EnableProma]
            .into_iter().filter(|action| has_proma || *action == ProjectBatchAction::Remove)
            .map(|action| (action, self.project_batch_action_label(action).to_string())).collect::<Vec<_>>();
        Popover::new("project-batch-action").appearance(false).anchor(Anchor::TopRight)
            .trigger(self.dropdown_button("project-batch-action-trigger", self.project_batch_action_label(selected), 12.)
                .min_w(px(102.)).border_0().bg(rgba(0x00000000)).font_weight(FontWeight::NORMAL))
            .content(move |_, _, popover_cx| {
                let mut menu = div().w(px(180.)).p(px(4.)).rounded(px(RADIUS_MENU))
                    .border_1().border_color(p.border).bg(p.elevated).shadow_lg();
                for (action, label) in &actions {
                    let action = *action;
                    let app = app.clone();
                    menu = menu.child(div().id(ElementId::Name(format!("project-batch-action-{}", action as u8).into()))
                        .h(px(30.)).px(px(9.)).rounded(px(RADIUS_CONTROL)).flex().items_center().cursor_pointer()
                        .font_family(FONT_UI).text_size(px(13.)).text_color(p.text)
                        .bg(if action == selected { p.selected } else { rgba(0x00000000) })
                        .hover(move |row| row.bg(p.hover)).child(label.clone())
                        .on_click(popover_cx.listener(move |_, _, _, cx| {
                            let _ = app.update(cx, |this, cx| {
                                this.projects_view.batch_action = action;
                                this.projects_view.batch_selected.clear();
                                this.projects_view.batch_filter = None;
                                cx.notify();
                            });
                            cx.emit(DismissEvent);
                        })));
                }
                menu
            })
    }

    fn project_proma_state_control(&self, selected: Vec<ProjectSkill>, project: PathBuf, cx: &mut Context<Self>) -> Popover {
        let p = self.palette();
        let app = cx.entity().downgrade();
        let enabled = self.projects_view.batch_action == ProjectBatchAction::EnableProma;
        let action = self.tr(if enabled { "启用" } else { "停用" }, if enabled { "Enable" } else { "Disable" }).to_string();
        let has_selection = !selected.is_empty();
        let title = if self.uses_english() { format!("{action} {} skills in Proma", selected.len()) }
            else { format!("在 Proma 中{action} {} 个技能", selected.len()) };
        let description = self.tr(
            if enabled { "将以下工作区副本移回 skills。全局及系统来源保持不变。" }
            else { "将以下工作区副本移到 skills-inactive，保留文件，可随时重新启用。全局及系统来源保持不变。" },
            if enabled { "Move these workspace copies back to skills. Global and system sources remain unchanged." }
            else { "Move these workspace copies to skills-inactive. Files are retained and can be enabled again. Global and system sources remain unchanged." }
        ).to_string();
        let cancel = self.tr("取消", "Cancel").to_string();
        Popover::new("project-proma-state-confirmation").appearance(false).anchor(Anchor::TopRight)
            .trigger(gpui_base::Button::new("project-proma-state-trigger")
                .accessibility_label(action.clone()).disabled(!has_selection)
                .h(px(CONTROL_HEIGHT)).min_w(px(42.)).px(px(8.)).rounded(px(RADIUS_CONTROL))
                .font_family(FONT_UI).font_weight(FontWeight::NORMAL).text_size(px(12.))
                .text_color(p.text).bg(rgba(0x00000000)).flex().items_center().justify_center()
                .hover(move |button| button.bg(p.hover)).active(move |button| button.bg(p.selected))
                .styles(|styles| styles.disabled(|style| style.bg(rgba(0x00000000)).text_color(p.muted)))
                .child(div().line_height(relative(1.5)).child(action.clone())))
            .content(move |_, _, popover_cx| {
                let confirm_app = app.clone();
                let selected = selected.clone();
                let project = project.clone();
                let mut locations = div().id("project-proma-state-locations").max_h(px(240.)).overflow_y_scroll();
                for skill in &selected {
                    for installation in &skill.installations {
                        locations = locations.child(div().py(px(5.)).text_size(px(11.)).font_family(MONO)
                            .child(div().text_color(p.text).child(skill.name.clone()))
                            .child(div().text_color(p.muted).child(display_path(&installation.path))));
                    }
                }
                div().w(px(400.)).p(px(16.)).rounded(px(RADIUS_MENU)).border_1().border_color(p.border)
                    .bg(p.elevated).shadow_lg().flex().flex_col().gap(px(12.))
                    .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(title.clone()))
                    .child(div().text_size(px(12.)).text_color(p.secondary).child(description.clone()))
                    .child(locations)
                    .child(div().flex().justify_end().gap(px(8.))
                        .child(Button::new("cancel-proma-state").small().label(cancel.clone())
                            .on_click(popover_cx.listener(|_, _, _, cx| cx.emit(DismissEvent))))
                        .child(Button::new("confirm-proma-state").small().primary().label(action.clone())
                            .on_click(popover_cx.listener(move |_, _, _, cx| {
                                let _ = confirm_app.update(cx, |this, cx| this.apply_project_proma_state(&selected, &project, enabled, cx));
                                cx.emit(DismissEvent);
                            }))))
            })
    }

    fn skill_origin_label(&self, origin: SkillOrigin, global: bool) -> &'static str {
        match origin {
            SkillOrigin::Project => self.tr("项目安装", "Project install"),
            SkillOrigin::Parent => self.tr("父目录继承", "Inherited from parent"),
            SkillOrigin::User if global => self.tr("全局安装", "Global install"),
            SkillOrigin::User => self.tr("全局继承", "Inherited globally"),
            SkillOrigin::System => self.tr("系统提供", "System-provided"),
            SkillOrigin::Builtin => self.tr("内置", "Built-in"),
            SkillOrigin::Integration => self.tr("集成预设", "Integration preset"),
        }
    }

    fn skill_origin_badge(&self, origin: SkillOrigin, global: bool) -> Div {
        let p = self.palette();
        let label = self.skill_origin_label(origin, global);
        div().ml(px(8.)).px(px(7.)).h(px(20.)).rounded(px(7.5))
            .bg(p.raised).flex().flex_none().items_center().text_size(px(11.))
            .text_color(if origin == SkillOrigin::Project { p.accent } else { p.muted })
            .child(label)
    }

    fn project_skill_group_header(
        &self, group: SkillSourceGroup, count: usize, cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette();
        let key = group.key().to_string();
        let debug_key = key.clone();
        let collapsed = self.projects_view.collapsed_skill_sources.contains(&key);
        let label = match group {
            SkillSourceGroup::Origin(origin) => self.skill_origin_label(origin, self.projects_view.global_project_view),
            SkillSourceGroup::Multiple => self.tr("多种来源", "Multiple sources"),
        };
        div().id(ElementId::Name(format!("project-skill-source-{key}").into()))
            .debug_selector(move || format!("project-skill-source-{debug_key}").into())
            .h(px(28.)).w_full().px(px(32.)).flex().items_center().gap(px(6.))
            .bg(p.surface).cursor_pointer()
            .hover(move |row| row.bg(p.hover))
            .child(Self::icon(if collapsed { "icons/chevron-right.svg" } else { "icons/chevron-down.svg" }, 12., p.muted))
            .child(div().text_size(px(11.)).font_weight(FontWeight::MEDIUM).text_color(p.secondary).child(label))
            .child(div().text_size(px(10.)).text_color(p.muted).child(count.to_string()))
            .child(div().ml(px(4.)).flex_1().h(px(1.)).bg(p.border))
            .on_click(cx.listener(move |this, _, _, cx| {
                if !this.projects_view.collapsed_skill_sources.remove(&key) {
                    this.projects_view.collapsed_skill_sources.insert(key.clone());
                }
                cx.notify();
            })).into_any_element()
    }

    fn matches_project_skill_filter(&self, row: &EffectiveSkillRow) -> bool {
        let Some(filter) = self.projects_view.batch_filter.as_ref() else {
            return true;
        };
        self.model.skills.iter().any(|skill| {
            skill.record.name == row.name
                && row.direct_installations.iter().any(|installation| {
                    same_file(
                        &installation.path.join("SKILL.md"),
                        &skill.path.join("SKILL.md"),
                    )
                        || (installation.target == InstallTarget::Proma
                            && crate::proma::is_managed(&installation.path, &skill.path))
                })
                && match filter {
                    ProjectSkillFilter::Group(group) => {
                        skill.record.group_id.as_ref() == Some(group)
                    }
                    ProjectSkillFilter::Tag(tag) => self
                        .tags_flow
                        .skills
                        .matches_filter(skill_storage_name(skill), *tag),
                }
        })
    }

    fn project_skill_filter_control(&self, cx: &mut Context<Self>) -> Popover {
        let p = self.palette();
        let app = cx.entity().downgrade();
        let groups = self.model.library.groups();
        let tags = self
            .tags_flow
            .skills
            .tags()
            .iter()
            .map(|tag| {
                (
                    tag.id,
                    self.tags_flow
                        .skills
                        .path(tag.id)
                        .unwrap_or_else(|| tag.name.clone()),
                )
            })
            .collect::<Vec<_>>();
        let selected = self.projects_view.batch_filter.clone();
        let filter_tooltip = if selected.is_some() {
            self.tr("更改筛选", "Change filter")
        } else {
            self.tr("筛选", "Filter")
        }
        .to_string();
        let all_label = self.tr("全部技能", "All skills").to_string();
        let group_label = self.tr("分组", "Groups").to_string();
        let tag_label = self.tr("标签", "Tags").to_string();
        Popover::new("project-skill-batch-filter")
            .appearance(false)
            .anchor(Anchor::TopRight)
            .trigger(
                Button::new("project-skill-batch-filter-trigger")
                    .small()
                    .custom(
                        ButtonCustomVariant::new(cx)
                            .color(rgba(0x00000000).into())
                            .foreground(
                                if selected.is_some() {
                                    p.accent
                                } else {
                                    p.muted
                                }
                                .into(),
                            )
                            .hover(p.hover.into()),
                    )
                    .h(px(CONTROL_HEIGHT))
                    .w(px(CONTROL_HEIGHT))
                    .rounded(px(RADIUS_CONTROL))
                    .child(Self::icon(
                        "icons/hash.svg",
                        15.,
                        if selected.is_some() {
                            p.accent
                        } else {
                            p.muted
                        },
                    ))
                    .tooltip(filter_tooltip),
            )
            .content(move |_, _, popover_cx| {
                let mut menu = div()
                    .id("project-skill-filter-scroll")
                    .w(px(220.))
                    .max_h(px(360.))
                    .overflow_y_scroll()
                    .p(px(4.))
                    .rounded(px(RADIUS_MENU))
                    .border_1()
                    .border_color(p.border)
                    .bg(p.elevated)
                    .shadow_lg();
                let all_app = app.clone();
                menu = menu.child(
                    div()
                        .id("project-skill-filter-all")
                        .h(px(30.))
                        .px(px(8.))
                        .rounded(px(RADIUS_CONTROL))
                        .flex()
                        .items_center()
                        .cursor_pointer()
                        .bg(if selected.is_none() {
                            p.selected
                        } else {
                            rgba(0x00000000)
                        })
                        .hover(move |row| row.bg(p.hover))
                        .child(all_label.clone())
                        .on_click(popover_cx.listener(move |_, _, _, cx| {
                            let _ = all_app.update(cx, |this, cx| {
                                this.projects_view.batch_filter = None;
                                cx.notify();
                            });
                            cx.emit(DismissEvent);
                        })),
                );
                if !groups.is_empty() {
                    menu = menu.child(
                        div()
                            .px(px(8.))
                            .pt(px(8.))
                            .pb(px(3.))
                            .text_size(px(11.))
                            .text_color(p.muted)
                            .child(group_label.clone()),
                    );
                }
                for group in &groups {
                    let filter = ProjectSkillFilter::Group(group.id.clone());
                    let group_app = app.clone();
                    let checked = selected.as_ref() == Some(&filter);
                    menu = menu.child(
                        div()
                            .id(ElementId::Name(
                                format!("project-skill-filter-group-{}", group.id).into(),
                            ))
                            .h(px(30.))
                            .px(px(8.))
                            .rounded(px(RADIUS_CONTROL))
                            .flex()
                            .items_center()
                            .cursor_pointer()
                            .bg(if checked {
                                p.selected
                            } else {
                                rgba(0x00000000)
                            })
                            .hover(move |row| row.bg(p.hover))
                            .child(group.name.clone())
                            .on_click(popover_cx.listener(move |_, _, _, cx| {
                                let _ = group_app.update(cx, |this, cx| {
                                    this.projects_view.batch_filter = Some(filter.clone());
                                    cx.notify();
                                });
                                cx.emit(DismissEvent);
                            })),
                    );
                }
                if !tags.is_empty() {
                    menu = menu.child(
                        div()
                            .px(px(8.))
                            .pt(px(8.))
                            .pb(px(3.))
                            .text_size(px(11.))
                            .text_color(p.muted)
                            .child(tag_label.clone()),
                    );
                }
                for (id, name) in &tags {
                    let filter = ProjectSkillFilter::Tag(*id);
                    let tag_app = app.clone();
                    let checked = selected.as_ref() == Some(&filter);
                    menu = menu.child(
                        div()
                            .id(ElementId::Name(
                                format!("project-skill-filter-tag-{id:?}").into(),
                            ))
                            .h(px(30.))
                            .px(px(8.))
                            .rounded(px(RADIUS_CONTROL))
                            .flex()
                            .items_center()
                            .cursor_pointer()
                            .bg(if checked {
                                p.selected
                            } else {
                                rgba(0x00000000)
                            })
                            .hover(move |row| row.bg(p.hover))
                            .child(name.clone())
                            .on_click(popover_cx.listener(move |_, _, _, cx| {
                                let _ = tag_app.update(cx, |this, cx| {
                                    this.projects_view.batch_filter = Some(filter.clone());
                                    cx.notify();
                                });
                                cx.emit(DismissEvent);
                            })),
                    );
                }
                menu
            })
    }

    fn project_batch_controls(
        &self,
        open_project: &Path,
        all_rows: &[EffectiveSkillRow],
        visible_rows: &[EffectiveSkillRow],
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.palette();
        let action = self.projects_view.batch_action;
        let selected = selected_project_skills(all_rows, &self.projects_view.batch_selected, action);
        let selected_count = selected.len();
        let selectable_names = visible_rows
            .iter()
            .filter(|row| !row.batch_installations(action).is_empty())
            .map(|row| row.name.clone())
            .collect::<Vec<_>>();
        let all_selected = !selectable_names.is_empty()
            && selectable_names
                .iter()
                .all(|name| self.projects_view.batch_selected.contains(name));
        let visible_selected_count = selectable_names
            .iter()
            .filter(|name| self.projects_view.batch_selected.contains(*name))
            .count();
        let partially_selected = visible_selected_count > 0 && !all_selected;
        let project = open_project.to_path_buf();
        let global_scope = dirs::home_dir()
            .as_ref()
            .is_some_and(|home| home == open_project);
        let select_all_label = if all_selected {
            self.tr("取消全选", "Deselect all")
        } else {
            self.tr("全选当前结果", "Select visible")
        }
        .to_string();
        let remove_label = if global_scope {
            self.tr("从用户级目录移除", "Remove from user-level locations")
        } else {
            self.tr("从此项目移除", "Remove from project")
        }
        .to_string();
        let select_first_label = self.tr("请先选择技能", "Select skills first").to_string();
        let exit_label = self.tr("退出选择", "Exit selection").to_string();
        let select_names = selectable_names.clone();
        let has_proma = all_rows.iter().any(|row| row.direct_installations.iter().any(|item| item.target == InstallTarget::Proma));
        let mut controls = div().h(px(CONTROL_HEIGHT)).font_family(FONT_UI).font_weight(FontWeight::NORMAL)
            .flex().items_center().gap(px(6.));
        if !select_names.is_empty() {
            controls = controls.child(
                div()
                    .id("project-batch-select-visible")
                    .size(px(CONTROL_HEIGHT))
                    .rounded(px(RADIUS_CONTROL))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(move |button| button.bg(p.hover))
                    .child(
                        div()
                            .size(px(16.))
                            .rounded(px(4.))
                            .border_1()
                            .border_color(if all_selected || partially_selected {
                                p.text
                            } else {
                                p.border_strong
                            })
                            .bg(if all_selected || partially_selected {
                                p.text
                            } else {
                                p.surface
                            })
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(all_selected, |indicator| {
                                indicator.child(Self::icon("icons/check.svg", 12., p.on_accent))
                            })
                            .when(partially_selected, |indicator| {
                                indicator.child(
                                    div().w(px(8.)).h(px(1.5)).rounded(px(1.)).bg(p.on_accent),
                                )
                            }),
                    )
                    .tooltip(move |window, cx| {
                        Tooltip::new(select_all_label.clone()).build(window, cx)
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        for name in &select_names {
                            if all_selected {
                                this.projects_view.batch_selected.remove(name);
                            } else {
                                this.projects_view.batch_selected.insert(name.clone());
                            }
                        }
                        cx.notify();
                    })),
            );
        }
        let selected_label = if self.uses_english() { format!("{selected_count} selected") }
            else { format!("已选 {selected_count}") };
        controls = controls.child(div().h(px(CONTROL_HEIGHT)).min_w(px(42.))
            .text_size(px(12.)).text_color(p.muted).flex().items_center().child(selected_label))
            .child(self.project_batch_action_control(has_proma, cx));
        controls = controls.child(self.project_skill_filter_control(cx));
        if action == ProjectBatchAction::Remove {
            controls = controls.child(
                div()
                    .id("project-batch-remove")
                    .size(px(CONTROL_HEIGHT))
                    .rounded(px(RADIUS_CONTROL))
                    .bg(if selected_count > 0 {
                        p.danger_soft
                    } else {
                        rgba(0x00000000)
                    })
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor(if selected_count > 0 {
                        CursorStyle::PointingHand
                    } else {
                        CursorStyle::Arrow
                    })
                    .child(Self::icon(
                        "icons/trash.svg",
                        15.,
                        if selected_count > 0 {
                            p.danger
                        } else {
                            p.muted
                        },
                    ))
                    .tooltip(move |window, cx| {
                        Tooltip::new(if selected_count > 0 {
                            remove_label.clone()
                        } else {
                            select_first_label.clone()
                        })
                        .build(window, cx)
                    })
                    .when(selected_count > 0, |button| {
                        button
                            .hover(move |button| button.opacity(0.8))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, window, cx| {
                                    this.delete_flow.selected = selected
                                        .iter()
                                        .flat_map(|skill| {
                                            unique_installation_paths(&skill.installations)
                                        })
                                        .collect();
                                    this.delete_flow.confirmation =
                                        Some(DeleteConfirmation::ProjectSkills {
                                            project: project.clone(),
                                            skills: selected.clone(),
                                            batch: true,
                                        });
                                    this.open_delete_dialog(window, cx);
                                }),
                            )
                    }),
            );
        } else {
            controls = controls.child(self.project_proma_state_control(selected, project, cx));
        }
        controls.child(
                div()
                    .id("project-batch-cancel")
                    .size(px(CONTROL_HEIGHT))
                    .rounded(px(RADIUS_CONTROL))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(move |button| button.bg(p.hover))
                    .child(Self::icon("icons/x.svg", 15., p.secondary))
                    .tooltip(move |window, cx| Tooltip::new(exit_label.clone()).build(window, cx))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.projects_view.batch_project = None;
                        this.projects_view.batch_selected.clear();
                        this.projects_view.batch_filter = None;
                        cx.notify();
                    })),
            )
    }

    fn project_skill_row(
        &self,
        effective: &EffectiveSkillRow,
        open_project: &Path,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette();
        let managed = effective
            .direct_installations
            .iter()
            .any(|installation| installation.managed);
        let mut location_labels = effective
            .locations
            .iter()
            .map(|location| display_effective_root(location, open_project))
            .collect::<Vec<_>>();
        if effective.built_in {
            location_labels.push(self.tr("内置", "Built-in").to_string());
        }
        let locations = location_labels.join(" · ");
        let proma_states = effective.direct_installations.iter()
            .filter(|installation| installation.target == InstallTarget::Proma)
            .map(|installation| {
                let state = match installation.state.as_deref() {
                    Some("active") => self.tr("已启用", "Active"),
                    Some("inactive") => self.tr("已停用", "Inactive"),
                    _ => self.tr("状态未知", "Unknown state"),
                };
                let status = match installation.status.as_deref() {
                    _ if installation.bundled => self.tr("预设副本", "Bundled copy"),
                    _ if installation.integration.is_some() => installation.integration.as_deref().unwrap_or_default(),
                    Some("current") => self.tr("最新", "Current"),
                    Some("update_pending") => self.tr("可同步", "Update pending"),
                    Some("modified") => self.tr("副本已修改", "Modified"),
                    Some("external_link") => self.tr("外部链接", "External link"),
                    Some("unmanaged") => self.tr("未受管", "Unmanaged"),
                    _ => self.tr("待核验", "Unverified"),
                };
                format!("Proma {state} · {status}")
            }).collect::<Vec<_>>().join(" / ");
        let name = effective.name.clone();
        let description = effective.description.clone();
        let installed_for_delete = ProjectSkill {
            name: effective.name.clone(),
            installations: effective.removable_installations(),
        };
        let can_delete = !installed_for_delete.installations.is_empty();
        let batch_mode = self.projects_view.batch_project.as_deref() == Some(open_project);
        let can_select = !effective.batch_installations(self.projects_view.batch_action).is_empty();
        let batch_checked = self.projects_view.batch_selected.contains(&name);
        let row_selector = format!("project-skill-{name}");
        let mut row = div()
            .id(ElementId::Name(row_selector.clone().into()))
            .debug_selector(move || row_selector)
            .w_full()
            .h(px(60.))
            .relative()
            .px(px(32.))
            .border_b_1()
            .border_color(p.border)
            .flex()
            .items_center()
            .hover(move |row| row.bg(p.hover))
            .when(batch_mode && can_select, |row| {
                row.child(
                    div()
                        .absolute()
                        .left(px(8.))
                        .top(px(0.))
                        .bottom(px(0.))
                        .flex()
                        .items_center()
                        .child(
                            Checkbox::new(ElementId::Name(
                                format!("batch-project-skill-{name}").into(),
                            ))
                            .tab_stop(false)
                            .checked(batch_checked),
                        ),
                )
            })
            .when(batch_mode && !can_select, |row| {
                let reason = self.project_batch_unavailable_reason(effective);
                row.child(div().id(ElementId::Name(format!("batch-project-unavailable-{name}").into()))
                    .absolute().left(px(8.)).top(px(0.)).bottom(px(0.))
                    .flex().items_center().justify_center().w(px(16.))
                    .child(div().size(px(16.)).rounded(px(4.)).border_1().border_color(p.border)
                        .flex().items_center().justify_center().text_size(px(11.)).text_color(p.muted).child("−"))
                    .tooltip(move |window, cx| Tooltip::new(reason.clone()).build(window, cx)))
            })
            .child(div().size(px(15.)).flex_none().child(Self::icon(
                "icons/package.svg",
                15.,
                p.secondary,
            )))
            .child(
                div()
                    .ml(px(10.))
                    .min_w_0()
                    .flex_1()
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .child(
                                div()
                                    .id(ElementId::Name(
                                        format!("project-skill-name-{name}").into(),
                                    ))
                                    .min_w_0()
                                    .truncate()
                                    .font_family(MONO)
                                    .text_size(px(13.))
                                    .child(effective.name.clone())
                                    .when(!description.is_empty(), |name| {
                                        let tooltip_description = description.clone();
                                        name.tooltip(move |window, cx| {
                                            let description = tooltip_description.clone();
                                            Tooltip::element(move |_, _| {
                                                div()
                                                    .w(px(320.))
                                                    .max_w(px(320.))
                                                    .font_family(MONO)
                                                    .whitespace_normal()
                                                    .line_height(relative(1.45))
                                                    .text_size(px(12.))
                                                    .child(description.clone())
                                            })
                                            .build(window, cx)
                                        })
                                    }),
                            )
                            .when(effective.manual_only, |title| {
                                title.child(self.manual_skill_badge().ml(px(10.)))
                            })
                            .when(managed, |title| {
                                title.child(self.managed_skill_badge().ml(px(8.)))
                            })
                            .children(effective.origins.iter().map(|origin| {
                                self.skill_origin_badge(*origin, self.projects_view.global_project_view)
                            }))
                            .when(!proma_states.is_empty(), |title| {
                                title.child(div().ml(px(8.)).text_size(px(11.))
                                    .text_color(p.muted).child(proma_states.clone()))
                            }),
                    )
                    .child(
                        div()
                            .mt(px(3.))
                            .min_w_0()
                            .truncate()
                            .font_family(MONO)
                            .text_size(px(12.))
                            .text_color(p.muted)
                            .child(locations),
                    ),
            )
            .child(
                self.effective_agent_badges(format!("project-{name}"), &effective.agents, cx)
                    .ml(px(12.)),
            );
        if batch_mode && can_select {
            let selected_name = name.clone();
            row = row
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.projects_view.batch_selected.remove(&selected_name) {
                        this.projects_view
                            .batch_selected
                            .insert(selected_name.clone());
                    }
                    cx.notify();
                }));
        } else if !batch_mode && can_delete {
            let project_for_delete = open_project.to_path_buf();
            row = row.child(
                self.danger_icon_button(
                    ElementId::Name(format!("remove-project-{name}").into()),
                    "icons/trash.svg",
                    cx,
                )
                .ml(px(10.))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        this.delete_flow.selected =
                            unique_installation_paths(&installed_for_delete.installations)
                                .into_iter()
                                .collect();
                        this.delete_flow.confirmation = Some(DeleteConfirmation::ProjectSkills {
                            project: project_for_delete.clone(),
                            skills: vec![installed_for_delete.clone()],
                            batch: false,
                        });
                        this.open_delete_dialog(window, cx);
                    }),
                ),
            );
        }
        row.into_any_element()
    }

    pub(super) fn projects_page(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        #[cfg(debug_assertions)]
        let render_started = Instant::now();
        let p = self.palette();
        let query = self
            .projects_view
            .project_search
            .read(cx)
            .value()
            .trim()
            .to_lowercase();
        let global_root = dirs::home_dir();
        let global_skills = global_root
            .as_ref()
            .and_then(|path| self.project_snapshot(path, cx));
        let global_skill_count = global_skills.as_ref().map(Vec::len);
        let projects = self.project_entries()
            .into_iter()
            .filter(|entry| {
                let path = &entry.path;
                let matches_query = query.is_empty()
                    || entry.name.to_lowercase().contains(&query)
                    || path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_lowercase()
                        .contains(&query)
                    || path.display().to_string().to_lowercase().contains(&query);
                let matches_tag = self.tags_flow.selected_project_filter.is_none_or(|tag| {
                    self.tags_flow
                        .projects
                        .matches_filter(&project_tag_key(path), tag)
                });
                matches_query && matches_tag
            })
            .collect::<Vec<_>>();
        let visible_project_count = projects.len();
        let project_context_app = cx.entity().downgrade();
        let mut recent_rows = div()
            .id("projects-scroll")
            .flex_1()
            .min_h_0()
            .px(px(8.))
            .overflow_y_scroll();
        if projects.is_empty() {
            recent_rows = recent_rows.child(
                div()
                    .mt(px(30.))
                    .px(px(16.))
                    .text_size(px(13.))
                    .text_color(p.muted)
                    .text_center()
                    .child(if self.project_entries().is_empty() {
                        self.tr("还没有项目", "No projects yet")
                    } else {
                        self.tr("没有匹配的项目", "No matching projects")
                    }),
            );
        }
        for entry in projects {
            let path = entry.path;
            let project_label = entry.name;
            let selected_path = path.clone();
            let selected = !self.projects_view.global_project_view
                && self.projects_view.open_project.as_ref() == Some(&path);
            let project_key = project_tag_key(&path);
            let assigned_project_tags = self
                .tags_flow
                .projects
                .assigned_tags(&project_key)
                .into_iter()
                .filter_map(|tag| self.tags_flow.projects.path(tag.id))
                .collect::<Vec<_>>();
            let mut project_tag_chips = div().mt(px(4.)).flex().gap(px(4.)).flex_wrap();
            for (index, tag) in assigned_project_tags.iter().enumerate() {
                project_tag_chips = project_tag_chips.child(
                    div()
                        .id(ElementId::Name(
                            format!("project-tag-{}-{index}", path.display()).into(),
                        ))
                        .px(px(3.))
                        .rounded(px(5.))
                        .font_family(MONO)
                        .text_size(px(10.))
                        .text_color(p.secondary)
                        .child(format!("#{tag}")),
                );
            }
            let reveal_path = path.clone();
            let set_tags_app = project_context_app.clone();
            let set_tags_path = path.clone();
            let remove_app = project_context_app.clone();
            let remove_path = path.clone();
            let set_tags_label = self.tr("设置标签", "Set tags").to_string();
            let reveal_label = if cfg!(target_os = "macos") {
                self.tr("在访达中显示", "Show in Finder")
            } else {
                self.tr("在文件夹中显示", "Show in folder")
            }
            .to_string();
            let remove_label = self.tr("移除项目", "Remove project").to_string();
            let remove_menu_color = p.danger;
            recent_rows = recent_rows.child(
                div()
                    .id(ElementId::Name(
                        format!("project-{}", path.display()).into(),
                    ))
                    .min_h(px(48.))
                    .px(px(8.))
                    .my(px(2.))
                    .rounded(px(RADIUS_CONTROL))
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .bg(if selected {
                        p.selected
                    } else {
                        rgba(0x00000000)
                    })
                    .hover(move |row| row.bg(p.hover))
                    .child(
                        div()
                            .size(px(28.))
                            .flex_none()
                            .rounded(px(RADIUS_CONTROL))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(Self::icon("icons/folder.svg", 16., p.secondary)),
                    )
                    .child(
                        div()
                            .ml(px(10.))
                            .min_w_0()
                            .flex_1()
                            .child(
                                div()
                                    .font_family(MONO)
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .truncate()
                                    .child(project_label),
                            )
                            .child(
                                div()
                                    .mt(px(3.))
                                    .font_family(MONO)
                                    .text_size(px(11.))
                                    .text_color(p.muted)
                                    .truncate()
                                    .child(display_path(&path)),
                            )
                            .when(!assigned_project_tags.is_empty(), |content| {
                                content.child(project_tag_chips)
                            }),
                    )
                    .child(self.effective_agent_badges(
                        format!("project-sources-{}", path.display()), &entry.agents, cx,
                    ))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.projects_view.open_project.as_ref() != Some(&selected_path) {
                            this.projects_view.batch_project = None;
                            this.projects_view.batch_selected.clear();
                            this.projects_view.batch_filter = None;
                        }
                        this.projects_view.open_project = Some(selected_path.clone());
                        this.projects_view.global_project_view = false;
                        this.projects_view.selected_project_agent = None;
                        this.projects_view.project_agents_expanded = false;
                        cx.notify();
                    }))
                    .context_menu(move |menu, _, _| {
                        let reveal_path = reveal_path.clone();
                        let set_tags_app = set_tags_app.clone();
                        let set_tags_path = set_tags_path.clone();
                        let remove_app = remove_app.clone();
                        let remove_path = remove_path.clone();
                        let remove_menu_item =
                            Self::danger_menu_item(remove_label.clone(), remove_menu_color);
                        menu.min_w(px(160.))
                            .item(
                                PopupMenuItem::new(set_tags_label.clone())
                                    .icon(Icon::default().path("icons/hash.svg"))
                                    .on_click(move |_, _, cx| {
                                        let _ = set_tags_app.update(cx, |this, cx| {
                                            this.open_project_tag_assignment_dialog(
                                                set_tags_path.clone(),
                                                cx,
                                            );
                                        });
                                    }),
                            )
                            .item(
                                PopupMenuItem::new(reveal_label.clone())
                                    .icon(Icon::new(IconName::Folder))
                                    .on_click(move |_, _, cx| cx.reveal_path(&reveal_path)),
                            )
                            .item(remove_menu_item.on_click(move |_, window, cx| {
                                let _ = remove_app.update(cx, |this, cx| {
                                    this.model.library.config.remove_project(&remove_path);
                                    if this.projects_view.open_project.as_ref()
                                        == Some(&remove_path)
                                    {
                                        this.projects_view.open_project = None;
                                        this.projects_view.global_project_view = true;
                                    }
                                    if this.projects_view.batch_project.as_ref()
                                        == Some(&remove_path)
                                    {
                                        this.projects_view.batch_project = None;
                                        this.projects_view.batch_selected.clear();
                                        this.projects_view.batch_filter = None;
                                    }
                                    let _ = this.model.library.save();
                                    this.sync_project_select(window, cx);
                                    cx.notify();
                                });
                            }))
                    }),
            );
        }
        let global_selected = self.projects_view.global_project_view;
        let global_row = div()
            .id("global-project-skills")
            .min_h(px(52.))
            .mx(px(8.))
            .mb(px(6.))
            .px(px(8.))
            .rounded(px(RADIUS_CONTROL))
            .flex()
            .items_center()
            .cursor_pointer()
            .bg(if global_selected {
                p.selected
            } else {
                rgba(0x00000000)
            })
            .hover(move |row| row.bg(p.hover))
            .child(
                div()
                    .size(px(28.))
                    .flex_none()
                    .rounded(px(RADIUS_CONTROL))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(Self::icon("icons/house.svg", 16., p.secondary)),
            )
            .child(
                div()
                    .ml(px(10.))
                    .min_w_0()
                    .flex_1()
                    .child(
                        div()
                            .font_family(MONO)
                            .text_size(px(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(self.tr("全局生效", "Global")),
                    )
                    .child(
                        div()
                            .mt(px(3.))
                            .font_family(MONO)
                            .text_size(px(11.))
                            .text_color(p.muted)
                            .truncate()
                            .child(match global_skill_count {
                                Some(count) if self.uses_english() => {
                                    counted(count, "user-level skill", "user-level skills")
                                }
                                Some(count) => format!("{count} 个用户级技能"),
                                None => self.tr("正在扫描…", "Scanning…").into(),
                            }),
                    ),
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.projects_view.batch_project = None;
                this.projects_view.batch_selected.clear();
                this.projects_view.batch_filter = None;
                this.projects_view.global_project_view = true;
                this.projects_view.selected_project_agent = None;
                this.projects_view.project_agents_expanded = false;
                cx.notify();
            }));
        let recent_panel = div()
            .w_full()
            .min_w_0()
            .min_h_0()
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .bg(p.surface)
            .child(
                self.panel_header(
                    "projects-panel-window-drag",
                    self.tr("项目", "Projects"),
                    self.project_entries().len(),
                    cx,
                )
                .child(
                    self.labeled_icon_button(
                        "open-project-folder",
                        "icons/plus.svg",
                        p.text,
                        self.tr("打开项目文件夹", "Open project folder"),
                        cx,
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| this.browse_project(window, cx)),
                    ),
                ),
            )
            .child(
                div().px(px(10.)).py(px(8.)).child(
                    Input::new(&self.projects_view.project_search)
                        .small()
                        .h(px(SEARCH_HEIGHT))
                        .w_full()
                        .rounded(px(999.))
                        .bg(p.surface)
                        .border_color(p.border_strong)
                        .text_size(px(14.))
                        .prefix(Self::icon("icons/search.svg", 16., p.muted)),
                ),
            )
            .child(global_row)
            .child(
                div()
                    .px(px(16.))
                    .pb(px(6.))
                    .flex()
                    .items_center()
                    .text_size(px(12.))
                    .text_color(p.muted)
                    .child(
                        self.tags_flow
                            .selected_project_filter
                            .and_then(|tag| self.tags_flow.projects.path(tag))
                            .map(|path| format!("#{path}  {visible_project_count}"))
                            .unwrap_or_else(|| {
                                format!("{}  {visible_project_count}", self.tr("项目", "Projects"))
                            }),
                    )
                    .child(div().flex_1())
                    .child(self.tag_filter_control(TagScope::Projects, cx)),
            )
            .child(recent_rows);

        let selected_root = if self.projects_view.global_project_view {
            global_root.as_ref()
        } else {
            self.projects_view.open_project.as_ref()
        };
        let detail = if let Some(open_project) = selected_root {
            let is_global = self.projects_view.global_project_view;
            let project_skills = if is_global {
                global_skills
            } else {
                self.project_snapshot(open_project, cx)
            };
            let estimates = project_skills
                .as_ref()
                .and_then(|_| self.context_estimate_snapshot(open_project, cx));
            let loading = project_skills.is_none() || estimates.is_none();
            let project_skills = project_skills.unwrap_or_default();
            let estimates = estimates.unwrap_or_default();
            let context_panel = if loading {
                div()
                    .h(px(86.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(9.))
                    .text_size(px(13.))
                    .text_color(p.muted)
                    .child(self.shell.spinner_accent.clone())
                    .child(self.tr("正在扫描项目…", "Scanning project…"))
            } else {
                self.context_estimate_panel(open_project, &estimates, cx)
            };
            let all_effective_rows = if loading {
                Arc::new(Vec::new())
            } else {
                let scanned_at = self.projects_view.context_estimates.borrow()[open_project].scanned_at;
                self.projects_view.skill_rows_cache.borrow_mut().get_or_build(
                    SkillRowsCacheKey {
                        project: open_project.clone(),
                        generation: self.projects_view.scan_generation,
                        agent: self.projects_view.selected_project_agent,
                        scanned_at,
                    },
                    &estimates,
                    &project_skills,
                )
            };
            let skill_count = all_effective_rows.len();
            let batch_mode = self.projects_view.batch_project.as_deref() == Some(open_project);
            let can_batch = batch_mode
                || all_effective_rows
                    .iter()
                    .any(|row| row.can_remove() || !row.batch_installations(ProjectBatchAction::DisableProma).is_empty()
                        || !row.batch_installations(ProjectBatchAction::EnableProma).is_empty());
            let effective_rows = if batch_mode {
                Arc::new(all_effective_rows
                    .iter()
                    .filter(|row| self.matches_project_skill_filter(row))
                    .cloned()
                    .collect::<Vec<_>>())
            } else {
                Arc::clone(&all_effective_rows)
            };
            let mut skills = div()
                .id("project-skills-scroll")
                .debug_selector(|| "project-skills-scroll".into())
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .px(px(0.))
                .pt(px(14.))
                .pb(px(28.));
            if effective_rows.is_empty() && !loading {
                let selected_agent = self
                    .projects_view
                    .selected_project_agent
                    .map(AgentKind::label)
                    .unwrap_or_default();
                skills = skills.child(
                    div()
                        .mt(px(70.))
                        .flex()
                        .flex_col()
                        .items_center()
                        .text_color(p.muted)
                        .child(Self::icon("icons/package.svg", 28., p.muted))
                        .child(div().mt(px(10.)).text_size(px(14.)).child(
                            if self.projects_view.selected_project_agent.is_some() {
                                if self.uses_english() {
                                    format!("No effective skills for {selected_agent}")
                                } else {
                                    format!("没有检测到对 {selected_agent} 生效的技能")
                                }
                            } else if is_global {
                                self.tr(
                                    "没有检测到用户级生效的技能",
                                    "No user-level skills detected",
                                )
                                .to_string()
                            } else {
                                self.tr(
                                    "没有检测到对这个项目生效的技能",
                                    "No effective skills detected for this project",
                                )
                                .to_string()
                            },
                        )),
                );
            }
            if !effective_rows.is_empty() {
                let rows = Arc::clone(&effective_rows);
                let items = Arc::new(grouped_skill_items(&rows, &self.projects_view.collapsed_skill_sources));
                let list_key = (Some(open_project.clone()), self.projects_view.selected_project_agent, items.as_ref().clone());
                if *self.projects_view.skill_list_items.borrow() != list_key {
                    self.projects_view.skill_list.reset(items.len());
                    *self.projects_view.skill_list_items.borrow_mut() = list_key;
                }
                let project = open_project.clone();
                skills = skills.child(
                    div().id("project-skills-virtual-list")
                        .debug_selector(|| "project-skills-virtual-list".into()).flex_1().min_h_0()
                        .child(list(
                        self.projects_view.skill_list.clone(),
                        cx.processor(move |this, index, _, cx| {
                            match &items[index] {
                                    ProjectSkillListItem::Header { group, count } => this.project_skill_group_header(*group, *count, cx),
                                    ProjectSkillListItem::Skill(index) => this.project_skill_row(&rows[*index], &project, cx),
                            }
                        }),
                    )
                    .size_full()),
                );
            }
            let plugin_groups =
                effective_plugin_groups(&estimates, self.projects_view.selected_project_agent);
            let plugin_skill_count = plugin_groups
                .iter()
                .map(|plugin| plugin.skills.len())
                .sum::<usize>();
            let mut tabs = self
                .project_skills_tabs(skill_count, plugin_skill_count, cx)
                .child(div().flex_1());
            if !effective_rows.is_empty() && self.projects_view.project_skills_tab == ProjectSkillsTab::Skills {
                let groups = SkillSourceGroup::ALL.into_iter()
                    .filter(|group| effective_rows.iter().any(|row| row.source_group() == *group))
                    .map(|group| group.key().to_string()).collect::<Vec<_>>();
                let all_collapsed = groups.iter().all(|key| self.projects_view.collapsed_skill_sources.contains(key));
                tabs = tabs.child(
                    div().id("project-skill-source-toggle-all")
                        .debug_selector(|| "project-skill-source-toggle-all".into()).h(px(CONTROL_HEIGHT)).px(px(8.))
                        .rounded(px(RADIUS_CONTROL)).flex().items_center().cursor_pointer()
                        .font_family(FONT_UI).text_size(px(12.)).text_color(p.secondary).hover(move |button| button.bg(p.hover))
                        .child(if all_collapsed { self.tr("展开分组", "Expand groups") } else { self.tr("收起分组", "Collapse groups") })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            for key in &groups {
                                if all_collapsed { this.projects_view.collapsed_skill_sources.remove(key); }
                                else { this.projects_view.collapsed_skill_sources.insert(key.clone()); }
                            }
                            cx.notify();
                        })),
                );
            }
            if can_batch && self.projects_view.project_skills_tab == ProjectSkillsTab::Skills {
                if batch_mode {
                    tabs = tabs.child(self.project_batch_controls(
                        open_project,
                        &all_effective_rows,
                        &effective_rows,
                        cx,
                    ));
                } else {
                    let batch_project_path = open_project.clone();
                    let batch_label = self.tr("批量管理", "Batch manage").to_string();
                    tabs = tabs.child(
                        div().h(px(CONTROL_HEIGHT)).flex().items_center().child(
                            div()
                                .id("project-batch-toggle")
                                .h(px(CONTROL_HEIGHT)).px(px(7.))
                                .rounded(px(RADIUS_CONTROL))
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .hover(move |button| button.bg(p.hover))
                                .child(Self::icon("icons/check.svg", 15., p.secondary))
                                .child(div().ml(px(4.)).text_size(px(12.)).text_color(p.secondary).child(batch_label.clone()))
                                .tooltip(move |window, cx| {
                                    Tooltip::new(batch_label.clone()).build(window, cx)
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.projects_view.batch_project =
                                        Some(batch_project_path.clone());
                                    this.projects_view.batch_selected.clear();
                                    this.projects_view.batch_action = if this.projects_view.selected_project_agent == Some(AgentKind::Proma) {
                                        ProjectBatchAction::DisableProma
                                    } else { ProjectBatchAction::Remove };
                                    cx.notify();
                                })),
                        ),
                    );
                }
            }
            let skills = match self.projects_view.project_skills_tab {
                ProjectSkillsTab::Skills => skills,
                ProjectSkillsTab::Plugins => self.effective_plugins_list(plugin_groups, cx),
            };
            div()
                .relative()
                .min_w_0()
                .flex_1()
                .h_full()
                .min_h_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .flex_none()
                        .px(px(24.))
                        .pt(px(24.))
                        .pb(px(18.))
                        .border_b_1()
                        .border_color(p.border)
                        .child(
                            div()
                                .font_family(MONO)
                                .text_size(px(16.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(self.selectable_text(
                                    "project-detail-name",
                                    300,
                                    if is_global {
                                        self.tr("全局生效", "Global").to_string()
                                    } else {
                                        self.project_entries().into_iter()
                                            .find(|entry| entry.path == *open_project)
                                            .map(|entry| entry.name)
                                            .unwrap_or_else(|| open_project.file_name().unwrap_or_default()
                                                .to_string_lossy().into_owned())
                                    },
                                    window,
                                    cx,
                                )),
                        )
                        .when(!is_global, |header| {
                            header.child(
                                div()
                                    .mt(px(4.))
                                    .font_family(MONO)
                                    .text_size(px(12.))
                                    .text_color(p.muted)
                                    .child(self.selectable_text(
                                        "project-detail-path",
                                        301,
                                        display_path(open_project),
                                        window,
                                        cx,
                                    )),
                            )
                        }),
                )
                .child(
                    div()
                        .px(px(28.))
                        .pt(px(10.))
                        .pb(px(12.))
                        .flex_none()
                        .border_b_1()
                        .border_color(p.border)
                        .child(context_panel),
                )
                .when(estimates.iter().any(|estimate| estimate.agent == AgentKind::Proma), |detail| {
                    detail.child(div().px(px(28.)).pb(px(8.)).text_size(px(12.))
                        .text_color(p.muted).child(self.tr(
                            "已扫描 Proma 工作区技能目录；当前会话的技能可用情况请在 Proma 中查看。",
                            "Proma workspace skill folders have been scanned. Check Proma for skill availability in the current session.",
                        )))
                })
                .child(tabs)
                .child(skills)
                .child(self.window_drag_strip("project-detail-window-drag", 24., cx))
        } else {
            let first = self.model.library.config.recent_projects.is_empty();
            div()
                .relative()
                .min_w_0()
                .flex_1()
                .h_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .size(px(48.))
                        .rounded(px(RADIUS_CARD))
                        .bg(p.raised)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(Self::icon("icons/folder.svg", 22., p.secondary)),
                )
                .child(
                    div()
                        .mt(px(16.))
                        .text_size(px(16.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(if first {
                            self.tr("打开第一个项目文件夹", "Open your first project folder")
                        } else {
                            self.tr("打开一个项目文件夹", "Open a project folder")
                        }),
                )
                .child(
                    div()
                        .mt(px(7.))
                        .max_w(px(430.))
                        .text_center()
                        .text_size(px(14.))
                        .line_height(relative(1.55))
                        .text_color(p.muted)
                        .child(self.tr(
                            "Kitter 会读取项目里的技能，并让你管理它们的安装边界。",
                            "Kitter reads the skills in a project and lets you manage where they are installed.",
                        )),
                )
                .child(
                    self.primary_icon_button(
                        "empty-open-project",
                        "icons/folder.svg",
                        self.tr("打开项目文件夹", "Open project folder"),
                        cx,
                    )
                    .mt(px(17.))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| this.browse_project(window, cx)),
                    ),
                )
                .child(self.window_drag_strip("empty-project-detail-window-drag", 24., cx))
        };
        let page = layout::content(
            &self.shell.content_layout,
            self.content_pane_width(window, cx),
            recent_panel,
            detail,
        );
        #[cfg(debug_assertions)]
        if let Some(path) = std::env::var_os("KITTER_PREVIEW_PROJECT_TIMINGS")
            && let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path)
        {
            use std::io::Write;
            let _ = writeln!(file, "{:.6}", render_started.elapsed().as_secs_f64() * 1000.);
        }
        page
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, path::PathBuf};

    use crate::{InstallTarget, ProjectSkillInstallation};

    use super::{EffectiveSkillRow, selected_project_skills};

    fn row(name: &str, paths: &[&str]) -> EffectiveSkillRow {
        EffectiveSkillRow {
            name: name.into(),
            description: String::new(),
            locations: Vec::new(),
            built_in: false,
            agents: Vec::new(),
            manual_only: false,
            origins: Vec::new(),
            direct_installations: paths
                .iter()
                .map(|path| ProjectSkillInstallation {
                    target: InstallTarget::Universal,
                    path: PathBuf::from(path),
                    managed: true,
                    removable: true, toggleable: false,
                    bundled: false,
                    integration: None,
                    state: None,
                    status: None,
                })
                .collect(),
        }
    }

    #[test]
    fn batch_removal_uses_only_the_selected_rows_direct_installations() {
        let rows = vec![
            row("selected", &["/project/.agents/skills/selected"]),
            row("shadowed", &["/project/.claude/skills/shadowed"]),
            row("external-only", &[]),
        ];
        let selected = HashSet::from(["selected".to_string(), "external-only".to_string()]);

        let targets = selected_project_skills(&rows, &selected, super::ProjectBatchAction::Remove);

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].name, "selected");
        assert_eq!(targets[0].installations.len(), 1);
        assert_eq!(
            targets[0].installations[0].path,
            PathBuf::from("/project/.agents/skills/selected")
        );
    }

    #[test]
    fn batch_removal_excludes_protected_installations_in_mixed_rows() {
        let mut mixed = row("mixed", &["/project/.agents/skills/mixed", "/pilot/skills/mixed"]);
        mixed.direct_installations[1].target = InstallTarget::Proma;
        mixed.direct_installations[1].managed = false;
        mixed.direct_installations[1].removable = false;
        let mut protected = row("protected", &["/pilot/skills/protected"]);
        protected.direct_installations[0].removable = false;
        assert!(mixed.can_remove());
        assert!(!protected.can_remove());
        let selected = HashSet::from(["mixed".into(), "protected".into()]);
        let targets = selected_project_skills(&[mixed, protected], &selected, super::ProjectBatchAction::Remove);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].installations.len(), 1);
        assert_eq!(targets[0].installations[0].target, InstallTarget::Universal);
    }

    #[cfg(unix)]
    #[test]
    fn preset_state_selection_targets_only_matching_proma_copies() {
        use super::ProjectBatchAction;
        let mut mixed = row("pdf", &["/project/.agents/skills/pdf", "/pilot/skills/pdf", "/pilot/skills-inactive/pdf"]);
        for (index, state) in [(1, "active"), (2, "inactive")] {
            let installation = &mut mixed.direct_installations[index];
            installation.target = InstallTarget::Proma;
            installation.managed = false;
            installation.removable = false;
            installation.bundled = true;
            installation.toggleable = true;
            installation.state = Some(state.into());
        }
        let mut external = mixed.clone();
        external.name = "protected".into();
        for installation in &mut external.direct_installations { installation.bundled = false; installation.toggleable = false; }
        let inherited = row("inherited", &[]);
        let rows = [mixed, external, inherited];
        let selected = HashSet::from(["pdf".into(), "protected".into(), "inherited".into()]);
        for (action, path) in [(ProjectBatchAction::DisableProma, "/pilot/skills/pdf"),
                               (ProjectBatchAction::EnableProma, "/pilot/skills-inactive/pdf")] {
            let targets = selected_project_skills(&rows, &selected, action);
            assert_eq!(targets.len(), 1);
            assert_eq!(targets[0].name, "pdf");
            assert_eq!(targets[0].installations.len(), 1);
            assert_eq!(targets[0].installations[0].path, PathBuf::from(path));
        }
        let removal = selected_project_skills(&rows, &selected, ProjectBatchAction::Remove);
        assert_eq!(removal[0].installations[0].target, InstallTarget::Universal);
    }
}
