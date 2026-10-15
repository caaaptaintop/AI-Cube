use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet, HashSet},
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
};

use gpui::{Entity, FocusHandle, ListState, ScrollHandle};
use gpui_base::TextSelectionHandle;
use gpui_component::{input::InputState, resizable::ResizableState, select::SelectState};

use crate::{
    InstallTarget, ProjectSkill, SkillLibrary, SkillSummary, adoption, source, tags::TagState,
};

use super::{
    AddKind, AddTask, ContentSnapshot, ContextEstimateCache, DeleteConfirmation, DialogBody,
    GroupDragScope, GroupEdit, Page, PageBody, ProjectBatchAction, ProjectChoice, ProjectSkillFilter, ProjectSkillsTab,
    SourceChoice, SpinnerView, TagAssignmentTarget, TagDropPosition, TagDropTarget, TagEdit, TagId,
    TagScope, adoption_list, effective_skills::AgentKind, skill_selection::SkillSelection,
};

pub(super) struct AppModel {
    pub installed_agents: Vec<crate::agents::AgentIconInfo>,
    pub library: SkillLibrary,
    pub skills: Vec<SkillSummary>,
    pub update_check: Option<source::UpdateCheckProgress>,
    pub updating_skill: Option<String>,
    pub proma_sync_task: Option<gpui::Task<()>>,
    pub proma_sync_warnings: Vec<String>,
}

pub(super) struct ShellState {
    pub page: Page,
    pub focus_handle: FocusHandle,
    pub header_drag_armed: bool,
    pub shell_layout: Entity<ResizableState>,
    pub content_layout: Entity<ResizableState>,
    pub spinner_accent: Entity<SpinnerView>,
    pub spinner_on_accent: Entity<SpinnerView>,
    pub notice: Option<String>,
    pub notice_generation: u64,
    pub system_dark: bool,
    pub dialog_body: Option<Entity<DialogBody>>,
    pub page_body: Entity<PageBody>,
}

pub(super) struct SkillsState {
    pub expanded_description: Option<String>,
    pub tab: super::DetailTab,
    pub selection: SkillSelection,
    pub selected_file: PathBuf,
    pub skill_search: Entity<InputState>,
    pub content_snapshot: RefCell<Option<ContentSnapshot>>,
    pub content_scroll: ScrollHandle,
    pub selectable_text_handles: RefCell<BTreeMap<String, TextSelectionHandle>>,
    pub collapsed_groups: HashSet<String>,
    pub known_groups: HashSet<String>,
    pub collapsed_content_directories: HashSet<PathBuf>,
}

#[derive(Clone, Copy)]
pub(super) enum ProjectAgentFilterOption {
    All,
    Agent(AgentKind),
    Unmarked,
}

#[derive(Clone, Default)]
pub(super) struct ProjectAgentFilter {
    pub agents: Vec<AgentKind>,
    pub unmarked: bool,
}

impl ProjectAgentFilter {
    pub(super) fn is_all(&self) -> bool {
        self.agents.is_empty() && !self.unmarked
    }

    pub(super) fn selected(&self, option: ProjectAgentFilterOption) -> bool {
        match option {
            ProjectAgentFilterOption::All => self.is_all(),
            ProjectAgentFilterOption::Agent(agent) => self.agents.contains(&agent),
            ProjectAgentFilterOption::Unmarked => self.unmarked,
        }
    }

    pub(super) fn toggle(&mut self, option: ProjectAgentFilterOption) {
        match option {
            ProjectAgentFilterOption::All => *self = Self::default(),
            ProjectAgentFilterOption::Agent(agent) => {
                if self.agents.contains(&agent) {
                    self.agents.retain(|selected| *selected != agent);
                } else {
                    self.agents.push(agent);
                }
            }
            ProjectAgentFilterOption::Unmarked => self.unmarked = !self.unmarked,
        }
    }

    pub(super) fn matches(&self, entry: &crate::project_discovery::ProjectEntry) -> bool {
        self.is_all()
            || (self.unmarked && entry.agents.is_empty())
            || self.agents.iter().any(|agent| entry.agents.contains(agent))
    }
}

pub(super) struct ProjectsState {
    pub discovered_projects: Vec<crate::project_discovery::ProjectEntry>,
    pub discovery_task: Option<gpui::Task<()>>,
    pub open_project: Option<PathBuf>,
    pub proma_directory_menu: Option<(gpui::Point<gpui::Pixels>, Entity<gpui_component::menu::PopupMenu>)>,
    pub global_project_view: bool,
    pub batch_project: Option<PathBuf>,
    pub batch_selected: HashSet<String>,
    pub batch_filter: Option<ProjectSkillFilter>,
    pub batch_action: ProjectBatchAction,
    pub project_skills_tab: ProjectSkillsTab,
    pub selected_project_agent: Option<AgentKind>,
    pub project_agents_expanded: bool,
    pub expanded_project_plugins: HashSet<String>,
    pub collapsed_skill_sources: HashSet<String>,
    pub project_search: Entity<InputState>,
    pub project_agent_filter: ProjectAgentFilter,
    pub project_select: Entity<SelectState<Vec<ProjectChoice>>>,
    pub project_snapshots: RefCell<BTreeMap<PathBuf, Vec<ProjectSkill>>>,
    pub skill_rows_cache: RefCell<super::SkillRowsCache>,
    pub skill_list: ListState,
    pub skill_list_items: RefCell<(Option<PathBuf>, Option<AgentKind>, Vec<super::ProjectSkillListItem>)>,
    pub project_snapshot_tasks: RefCell<BTreeMap<PathBuf, u64>>,
    pub context_estimates: RefCell<BTreeMap<PathBuf, ContextEstimateCache>>,
    pub context_estimate_tasks: RefCell<BTreeMap<PathBuf, u64>>,
    pub scan_generation: u64,
}

pub(super) struct AddFlowState {
    pub kind: AddKind,
    pub task: Option<AddTask>,
    pub scan: Option<source::SkillScan>,
    pub adoption_scan: Option<Arc<adoption::AdoptionScan>>,
    pub adoption_rows: Vec<adoption_list::AdoptionRow>,
    pub adoption_list: ListState,
    pub adoption_cancel: Option<Arc<AtomicBool>>,
    pub adoption_expanded: HashSet<String>,
    pub adoption_root: Option<PathBuf>,
    pub selected: HashSet<String>,
    pub error: Option<String>,
    pub group_enabled: bool,
    pub group_name: Option<String>,
    pub primary_input: Entity<InputState>,
    pub source_select: Entity<SelectState<Vec<SourceChoice>>>,
}

pub(super) struct InstallFlowState {
    pub modal: bool,
    pub global: bool,
    pub selected_projects: BTreeSet<PathBuf>,
    pub project_picker_open: bool,
    pub project_search: Entity<InputState>,
    pub project_scroll: ScrollHandle,
    pub selected_targets: HashSet<InstallTarget>,
}

pub(super) struct DeleteFlowState {
    pub confirmation: Option<DeleteConfirmation>,
    pub selected: HashSet<PathBuf>,
    pub busy: bool,
}

pub(super) struct TagsFlowState {
    pub name_input: Entity<InputState>,
    pub skills: TagState,
    pub projects: TagState,
    pub selected_skill_filter: Option<TagId>,
    pub selected_project_filter: Option<TagId>,
    pub scope: TagScope,
    pub drop_target: Option<TagDropTarget>,
    pub edit: Option<TagEdit>,
    pub delete_pending: Option<TagId>,
    pub error: Option<String>,
    pub assignment_keys: Vec<String>,
    pub assignment_label: Option<String>,
    pub return_to_assignment: Option<TagAssignmentTarget>,
}

pub(super) struct GroupsFlowState {
    pub drop_target: Option<(GroupDragScope, String, TagDropPosition)>,
    pub name_input: Entity<InputState>,
    pub edit: Option<GroupEdit>,
    pub delete_pending: Option<String>,
    pub delete_skills: bool,
    pub move_skills: Vec<String>,
}
