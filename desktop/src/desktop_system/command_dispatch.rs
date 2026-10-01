use anyhow::{Context, Result, bail, ensure};
use log::{debug, warn};
use serde_json::json;

use super::change::Zoom;
use super::change::set_focus;
use super::change::{Changes, ConfigurationChange, DesktopChange, TopologyChange};
use super::navigation::focus_depth_from_target;
use super::{
    ChangeSurface, DesktopCommand, DesktopSystem, DesktopTarget, FocusDepth, KeyboardFocusReason,
    ProjectCommand, TransactionEffectsMode,
};
use crate::desktop_system::change_surface::TargetSet;
use crate::instance_manager::{InstanceManager, ViewPath};
use crate::instance_presenter::InstanceRoot;
use crate::projects::{
    LaunchProfile, LaunchProfileId, LauncherMode, LauncherPresenter, MatrixPlacement, ProjectId,
    ProjectPresenter, SlotAssignment, SlotContent,
};

use massive_applications::prelude::*;
use massive_applications::{
    ConfigurationRequest, CreationMode, InstanceChange, InstanceId, InstanceSubmission,
    MoveDirection, SlotShift, ViewChange, ViewEvent, ViewRole,
};

/// The outcome of applying a change: its effects and any follow-up changes.
#[derive(Debug, Default)]
pub struct ChangeOutput {
    /// Additional changes to schedule.
    pub changes: Changes,
    pub surface: ChangeSurface,
}

impl ChangeOutput {
    pub fn measure(&mut self, target: DesktopTarget) {
        self.surface.size_invalid += target;
    }

    pub fn focus_changed(
        &mut self,
        previous: Option<DesktopTarget>,
        current: Option<DesktopTarget>,
    ) {
        if previous == current {
            return;
        }

        if let Some(previous) = previous {
            self.surface.size_invalid += previous;
        }
        if let Some(current) = current {
            self.surface.size_invalid += current;
        }
    }

    fn measures(measures: impl Into<TargetSet>) -> Self {
        Self {
            surface: ChangeSurface {
                size_invalid: measures.into(),
                ..Default::default()
            },
            ..Self::default()
        }
    }

    pub fn changes(changes: Changes) -> Self {
        Self {
            changes,
            ..Self::default()
        }
    }

    pub fn combine(&mut self, other: Self) {
        self.changes += other.changes;
        self.surface.combine(other.surface);
    }
}

impl DesktopSystem {
    /// Plan the execution of a command.
    pub fn plan(&self, command: DesktopCommand) -> Result<Changes> {
        match command {
            DesktopCommand::Project(project_command) => return self.plan_project(project_command),
            DesktopCommand::StartInstance {
                launcher,
                instance,
                root,
                parameters,
            } => {
                let originator_instance = self.focused_path().instance();
                let originating_details = originator_instance
                    .map(|originator| self.get_origination_details(launcher, originator));
                let insertion_pos = originating_details
                    .as_ref()
                    .map(|d| d.insertion_pos)
                    .unwrap_or(0);
                let (root, spawn) = match root {
                    Some(root) => (root, false),
                    // Rare case: only a spawned instance needs new scene objects, so the ambient
                    // change queue is used here instead of being threaded through `plan`.
                    None => (InstanceRoot::new(), true),
                };

                let mut changes: Changes = if spawn {
                    vec![DesktopChange::SpawnInstance {
                        instance,
                        root: root.clone(),
                        parameters: parameters.clone(),
                    }]
                } else {
                    Vec::new()
                }
                .into();

                changes += [
                    DesktopChange::PresentInstance {
                        launcher,
                        initial_center_translation: originating_details
                            .and_then(|od| od.initial_center_translation),
                        instance,
                        root,
                        parameters,
                    },
                    DesktopChange::Topology(TopologyChange::Insert {
                        what: instance.into(),
                        at_index: insertion_pos,
                        under: launcher.into(),
                    }),
                ];
                changes += set_focus(
                    Some(DesktopTarget::Instance(instance)),
                    KeyboardFocusReason::PresentInstance,
                );
                changes <<= DesktopChange::CommitFocusDepth(FocusDepth::default());

                return Ok(changes);
            }
            DesktopCommand::StopInstance(instance) => {
                let launcher = self.aggregates.hierarchy.launcher_of_instance(instance);

                // Set up a replacement focus first.
                //
                // Detail: This causes an unfocus event sent to the instance's view which may
                // unexpected while tear down.
                let replacement_focus = self.event_router.keyboard_focus().and_then(|focused| {
                    self.aggregates
                        .hierarchy
                        .resolve_replacement_focus_for_stopping_instance(focused, instance)
                });

                let mut changes = Changes::Empty;
                if let Some(focus) = replacement_focus {
                    changes += set_focus(Some(focus), KeyboardFocusReason::StopInstanceReplacement);
                }
                changes += [
                    DesktopChange::Topology(TopologyChange::Remove(instance.into())),
                    DesktopChange::HideInstance { launcher, instance },
                    DesktopChange::ShutdownInstance(instance),
                ];
                changes <<= DesktopChange::CommitFocusDepth(FocusDepth::default());

                return Ok(changes);
            }
            DesktopCommand::Navigate(direction) => return self.plan_navigate(direction),
            DesktopCommand::Zoom(Zoom::In) => {
                if self.focus_depth == FocusDepth::Slot
                    && matches!(
                        self.event_router.keyboard_focus(),
                        Some(DesktopTarget::Project(_))
                    )
                {
                    return Ok(DesktopChange::CommitFocusDepth(FocusDepth::Project).into());
                }
                if let Some(focus_depth) = self.focus_depth.zoom_in() {
                    return Ok(DesktopChange::CommitFocusDepth(focus_depth).into());
                }
            }
            DesktopCommand::Zoom(Zoom::Out) => {
                if self.focus_depth == FocusDepth::Project
                    && let Some(project) = self
                        .event_router
                        .keyboard_focus()
                        .map(|target| self.aggregates.hierarchy.project_of_target(target))
                    && let Some(parent) = self.aggregates.hierarchy.parent_project_of(project)
                {
                    let mut changes: Changes =
                        DesktopChange::CommitFocusDepth(FocusDepth::Slot).into();
                    changes += set_focus(
                        Some(DesktopTarget::ProjectMatrix(parent)),
                        KeyboardFocusReason::InputTransition,
                    );
                    return Ok(changes);
                }
                if let Some(focus_depth) = self.focus_depth.zoom_out() {
                    return Ok(DesktopChange::CommitFocusDepth(focus_depth).into());
                }
            }
            DesktopCommand::Zoom(Zoom::DefaultForFocused) => {
                if let Some(keyboard_focus) = self.event_router.keyboard_focus() {
                    let current_level = self.focus_depth;
                    let focus_level = focus_depth_from_target(keyboard_focus);

                    if current_level != focus_level {
                        return Ok(DesktopChange::CommitFocusDepth(focus_level).into());
                    }
                }
            }
        }

        Ok([].into())
    }

    fn plan_project(&self, command: ProjectCommand) -> Result<Changes> {
        let mut changes = Changes::Empty;
        match command {
            ProjectCommand::AddProject {
                id,
                name,
                placement,
                under,
            } => {
                let name = self.aggregates.configuration.new_project_name(
                    id,
                    DEFAULT_NEW_PROJECT_NAME,
                    &name,
                );
                let project_target = DesktopTarget::Project(id);
                let topology_parent = match under {
                    Some(parent) => {
                        changes <<= ConfigurationChange::AssignSlot {
                            parent,
                            placement,
                            content: SlotAssignment::Project {
                                id,
                                name: name.clone(),
                            },
                        };
                        DesktopTarget::ProjectMatrix(parent)
                    }
                    None => {
                        if id != ProjectId::ROOT
                            || self
                                .aggregates
                                .hierarchy
                                .exists(&DesktopTarget::Project(ProjectId::ROOT))
                        {
                            bail!(
                                "Internal error (plan AddProject): the root project is created exactly once, as `AddProject {{ under: None }}` with id {:?}",
                                ProjectId::ROOT
                            );
                        }
                        changes <<= ConfigurationChange::AddProject {
                            id,
                            name: name.clone(),
                        };
                        DesktopTarget::Desktop
                    }
                };
                changes <<= TopologyChange::Add {
                    what: project_target.clone(),
                    under: topology_parent,
                    after: None,
                };
                changes <<= TopologyChange::AddNested {
                    what: [
                        DesktopTarget::ProjectHeader(id),
                        DesktopTarget::ProjectMatrix(id),
                    ]
                    .into(),
                    under: project_target,
                };
            }
            ProjectCommand::RemoveProject(project_id) => {
                changes += self.plan_project_removal_focus(project_id);
                changes += self.plan_remove_project_content_checked(project_id)?;
                if let Some(parent) = self.aggregates.hierarchy.parent_project_of(project_id)
                    && let Some(placement) =
                        self.aggregates
                            .configuration
                            .project(parent)
                            .and_then(|project| {
                                project.placement_of_content(SlotContent::Project(project_id))
                            })
                {
                    changes += self.plan_project(ProjectCommand::ClearSlot {
                        parent,
                        placement,
                        shift: SlotShift::default(),
                    })?;
                }
            }
            ProjectCommand::AssignSlot {
                parent,
                placement,
                content,
                shift,
            } => {
                // The parsed aggregate already has boot slots, while the topology
                // starts empty; only displace occupants that are live in the scene.
                let assigned = self
                    .aggregates
                    .configuration
                    .content_at(parent, placement)
                    .filter(|previous| self.aggregates.hierarchy.exists(&previous.target()));
                if assigned.is_some() {
                    match shift {
                        SlotShift::Shift => {
                            changes +=
                                self.slot_shift_sequence(parent, placement, MoveDirection::Right)?;
                        }
                        SlotShift::Keep => {
                            changes += self.plan_clear_slot(parent, placement, SlotShift::Keep)?;
                        }
                    }
                }

                changes <<= ConfigurationChange::AssignSlot {
                    parent,
                    placement,
                    content: content.clone(),
                };
                changes <<= TopologyChange::Add {
                    what: content.content().target(),
                    under: DesktopTarget::ProjectMatrix(parent),
                    after: None,
                };
                if let SlotContent::Project(project) = content.content() {
                    changes <<= TopologyChange::AddNested {
                        what: [
                            DesktopTarget::ProjectHeader(project),
                            DesktopTarget::ProjectMatrix(project),
                        ]
                        .into(),
                        under: DesktopTarget::Project(project),
                    };
                }
            }
            ProjectCommand::ClearSlot {
                parent,
                placement,
                shift,
            } => {
                changes += self.plan_clear_slot(parent, placement, shift)?;
            }
            ProjectCommand::MoveSlot { source, dest } => {
                if self.aggregates.configuration.can_move_slot(source, dest) {
                    changes <<= ConfigurationChange::MoveSlot { source, dest };
                }
            }
            ProjectCommand::SetStartupPath(path) => {
                changes <<= ConfigurationChange::SetStartupPath(path)
            }
        }

        Ok(changes)
    }

    fn slot_shift_sequence(
        &self,
        project: ProjectId,
        placement: MatrixPlacement,
        direction: MoveDirection,
    ) -> Result<Changes> {
        let mut changes = Changes::Empty;
        for (source, dest) in self
            .aggregates
            .configuration
            .shifted_slots(project, placement, direction)?
        {
            changes <<= ConfigurationChange::MoveSlot {
                source: (project, source),
                dest: (project, dest),
            };
        }
        Ok(changes)
    }

    fn plan_clear_slot(
        &self,
        parent: ProjectId,
        placement: MatrixPlacement,
        shift: SlotShift,
    ) -> Result<Changes> {
        let Some(content) = self.aggregates.configuration.content_at(parent, placement) else {
            return Ok(Changes::Empty);
        };

        let mut changes = Changes::Empty;
        match content {
            SlotContent::Launcher(launcher) => {
                changes += self.plan_remove_launcher_instances(launcher);
                changes <<= TopologyChange::Remove(DesktopTarget::Launcher(launcher));
            }
            SlotContent::Project(project) => {
                changes += self.plan_remove_project_content(project);
                changes <<= TopologyChange::Remove(DesktopTarget::Project(project));
            }
        }
        changes <<= ConfigurationChange::ClearSlot { parent, placement };

        if shift == SlotShift::Shift {
            for (source, dest) in self
                .aggregates
                .configuration
                .shifted_left_slots(parent, placement)
            {
                changes <<= ConfigurationChange::MoveSlot {
                    source: (parent, source),
                    dest: (parent, dest),
                };
            }
        }

        Ok(changes)
    }

    fn plan_remove_launcher_instances(&self, launcher: LaunchProfileId) -> Changes {
        let mut changes = Changes::Empty;
        for instance in self.aggregates.hierarchy.launcher_instances(launcher) {
            changes += [
                DesktopChange::Topology(TopologyChange::Remove(instance.into())),
                DesktopChange::HideInstance { launcher, instance },
                DesktopChange::ShutdownInstance(instance),
            ];
        }
        changes
    }

    fn plan_remove_project_content(&self, project: ProjectId) -> Changes {
        let mut changes = Changes::Empty;
        for slot in self.aggregates.configuration.slots_ordered(project) {
            match slot.1 {
                SlotContent::Launcher(launcher) => {
                    changes += self.plan_remove_launcher_instances(launcher);
                    changes <<= TopologyChange::Remove(DesktopTarget::Launcher(launcher));
                }
                SlotContent::Project(nested) => {
                    changes += self.plan_remove_project_content(nested);
                    changes <<= TopologyChange::Remove(DesktopTarget::Project(nested));
                }
            }
        }
        changes
    }

    fn plan_remove_project_content_checked(&self, project: ProjectId) -> Result<Changes> {
        ensure!(
            self.aggregates.configuration.launcher_count()
                > self.launcher_count_of_subtree(project),
            "Configuration must define at least one launcher"
        );
        Ok(self.plan_remove_project_content(project))
    }

    fn launcher_count_of_subtree(&self, project: ProjectId) -> usize {
        self.aggregates
            .configuration
            .slots_ordered(project)
            .into_iter()
            .map(|(_, content)| match content {
                SlotContent::Launcher(_) => 1,
                SlotContent::Project(nested) => self.launcher_count_of_subtree(nested),
            })
            .sum()
    }

    fn plan_project_removal_focus(&self, project: ProjectId) -> Changes {
        let project_target = DesktopTarget::Project(project);
        if self
            .aggregates
            .hierarchy
            .path_contains_target(self.event_router.keyboard_focus(), &project_target)
        {
            return set_focus(
                Some(self.project_removal_focus(project)),
                KeyboardFocusReason::InputTransition,
            );
        }

        Changes::Empty
    }

    pub fn apply_change(
        &mut self,
        change: DesktopChange,
        instance_manager: &mut InstanceManager,
        effects_mode: TransactionEffectsMode,
    ) -> Result<ChangeOutput> {
        match change {
            DesktopChange::SpawnInstance {
                instance,
                root,
                mut parameters,
            } => {
                // Probably pull the name of the application into SpawnInstance?
                let application = self
                    .env
                    .applications
                    .get_named(&self.env.primary_application)
                    .context("Internal error, application not registered")?;

                parameters.insert(
                    "size_px".to_string(),
                    json!([
                        self.default_panel_size.width,
                        self.default_panel_size.height
                    ]),
                );
                instance_manager.spawn(
                    instance,
                    application,
                    CreationMode::New(parameters),
                    root.view_parent(),
                )?;
            }
            DesktopChange::ShutdownInstance(instance) => {
                // This might fail if StopInstance gets triggered with an instance that ended in
                // itself (shouldn't the instance_manager keep it until we finally free it).
                if let Err(e) = instance_manager.request_shutdown(instance) {
                    warn!("Failed to shutdown instance, it may be gone already: {e}");
                };
            }
            DesktopChange::PresentInstance {
                launcher,
                initial_center_translation,
                instance,
                root,
                parameters,
            } => {
                self.present_instance(
                    launcher,
                    initial_center_translation,
                    instance,
                    root,
                    parameters,
                )?;
            }
            DesktopChange::HideInstance { launcher, instance } => {
                self.hide_instance(launcher, instance)?;
            }
            DesktopChange::SetFocus { target, reason } => {
                let previous_focus = self.event_router.keyboard_focus().cloned();
                self.focus(target.as_ref(), instance_manager, reason)?;
                let current_focus = self.event_router.keyboard_focus().cloned();

                let mut output = ChangeOutput::default();
                output.focus_changed(previous_focus, current_focus);

                return Ok(output);
            }
            DesktopChange::CommitNavigationAffinity(column_affinity) => {
                self.navigation_control
                    .commit_column_affinity(column_affinity);
            }
            DesktopChange::CommitFocusDepth(focus_depth) => {
                if self.focus_depth != focus_depth {
                    self.focus_depth = focus_depth;

                    let mut output = ChangeOutput::default();
                    if let Some(focused) = self.event_router.keyboard_focus() {
                        output.measure(focused.clone());
                    }
                    return Ok(output);
                }
            }
            DesktopChange::WindowResized => {
                let mut output = ChangeOutput::default();
                output.surface.window_size_changed = true;
                // A window resize only affects the presentation of instances if we are in
                // [`FocusDepth::InstanceFullScreen`] and an instance is focused.
                if self.focus_depth == FocusDepth::InstanceFullScreen
                    && let Some(instance) = self.focused_path().instance()
                {
                    // Design: Somehow this is not a directly affected by a focus change. So there
                    // is a discrepancy between "updating the presentation" and a target affected by
                    // a focus change (somehow the target should probably decide about this if it's
                    // "presentation" is affected?).
                    output.measure(DesktopTarget::Instance(instance));
                }
                return Ok(output);
            }
            DesktopChange::ResizeAll(size_px) => {
                self.default_panel_size = size_px;
                for (instance, presenter) in self.aggregates.instances.iter_mut() {
                    let Some(view) = presenter.primary_view_id() else {
                        continue;
                    };
                    if let Err(error) = instance_manager
                        .send_view_event((*instance, view), ViewEvent::Resized(size_px))
                    {
                        warn!("Failed to resize terminal instance {instance:?}: {error}");
                    }
                }
                // Root measurement otherwise reuses descendant measurements made for the previous
                // panel extent, leaving project and matrix slots at their old sizes.
                self.layout_state.clear();
                return Ok(ChangeOutput::measures(DesktopTarget::Desktop));
            }
            DesktopChange::Topology(change) => {
                let previous_focus = self.event_router.keyboard_focus().cloned();
                // Design: That's somewhat unexpected here, that `apply_topology_change` changes
                // focus. Can we make this more obvious? We should combine the `instance_manager`
                // side effects perhaps.
                let measure_target = self.apply_topology_change(change, instance_manager)?;
                let current_focus = self.event_router.keyboard_focus().cloned();

                let mut output = ChangeOutput::measures(measure_target);
                output.focus_changed(previous_focus, current_focus);

                return Ok(output);
            }
            DesktopChange::ForwardEvents(transitions) => {
                let commands = self.forward_event_transitions(transitions, instance_manager)?;
                let mut changes = Changes::default();
                for command in commands {
                    changes += self.plan(command)?;
                }
                return Ok(ChangeOutput::changes(changes));
            }
            DesktopChange::IntegrateInstanceSubmission(instance_id, instance_submission) => {
                return self.apply_instance_submission(instance_id, instance_submission);
            }
            DesktopChange::Project(project_change) => {
                // A setup change only updates the live model; it must not mirror into
                // the persisted document.
                //
                // The mirror lands before the live model, so a failure below leaves
                // the in-memory document carrying an edit the model never applied.
                // This is one of the partial-failure states described on
                // `DesktopSystem::transact` — not yet a rollback target.
                if effects_mode != TransactionEffectsMode::Setup {
                    self.configuration.apply(project_change.clone())?;
                }
                return self.apply_project_change(project_change);
            }
        }

        Ok(ChangeOutput::default())
    }

    pub fn apply_topology_change(
        &mut self,
        change: TopologyChange,
        instance_manager: &InstanceManager,
    ) -> Result<DesktopTarget> {
        match change {
            TopologyChange::Add { what, under, after } => {
                if let Some(after) = after {
                    // Design: `under` can be resolved via `after`!
                    self.aggregates.hierarchy.add_after(after, what)?;
                } else {
                    self.aggregates.hierarchy.add(under.clone(), what)?;
                }
                Ok(under)
            }
            TopologyChange::AddNested { what, under } => {
                self.aggregates.hierarchy.add_nested(under.clone(), what)?;
                Ok(under)
            }
            TopologyChange::Insert {
                what,
                at_index,
                under,
            } => {
                self.aggregates
                    .hierarchy
                    .insert_at(under.clone(), at_index, what)?;
                Ok(under)
            }
            TopologyChange::Remove(target) => {
                // A removed subtree may still hold pointer and/or keyboard focus. Clear pointer
                // focus and retarget keyboard focus to the parent before removal so the event
                // router is not left pointing at a removed node.
                self.unfocus_pointer_if_path_contains(&target, instance_manager)?;
                self.refocus_to_parent_if_path_contains(&target, instance_manager)?;
                self.remove_target(&target)
            }
        }
    }

    fn apply_project_change(&mut self, change: ConfigurationChange) -> Result<ChangeOutput> {
        match change {
            ConfigurationChange::AddProject { id, name } => {
                self.ensure_project_presenter(id, name.clone())?;
                self.aggregates.configuration.add_project(id, name);
            }
            ConfigurationChange::AssignSlot {
                parent,
                placement,
                content,
            } => {
                let name = match &content {
                    SlotAssignment::Launcher { profile, .. } => profile.name.clone(),
                    SlotAssignment::Project { name, .. } => name.clone(),
                };
                let target = content.content();
                self.aggregates
                    .configuration
                    .assign_slot(parent, placement, content);
                match target {
                    SlotContent::Launcher(id) => {
                        self.ensure_launcher_presenter(parent, id, name)?;
                    }
                    SlotContent::Project(id) => {
                        self.ensure_project_presenter(id, name)?;
                    }
                }
            }
            ConfigurationChange::ClearSlot { parent, placement } => {
                if let Some(content) = self.aggregates.configuration.content_at(parent, placement) {
                    match content {
                        SlotContent::Launcher(launcher) => {
                            self.aggregates.launchers.remove(&launcher)?;
                        }
                        SlotContent::Project(project) => {
                            self.aggregates.projects.remove(&project)?;
                        }
                    }
                }
                self.aggregates.configuration.clear_slot(parent, placement);
            }
            ConfigurationChange::MoveSlot { source, dest } => {
                let (source_parent, _) = source;
                let (dest_parent, _) = dest;
                if !self.aggregates.configuration.move_slot(source, dest) {
                    return Ok(ChangeOutput::default());
                }
                let mut output = ChangeOutput::default();
                output.measure(DesktopTarget::ProjectMatrix(source_parent));
                output.measure(DesktopTarget::ProjectMatrix(dest_parent));
                return Ok(output);
            }
            // The startup launcher is consumed at boot (`Setup`); the runtime model
            // does not retain it. Only this dispatch must handle it.
            ConfigurationChange::SetStartupPath(_) => {}
        }

        Ok(ChangeOutput::default())
    }

    fn ensure_project_presenter(&mut self, id: ProjectId, name: String) -> Result<()> {
        if self.aggregates.projects.get(&id).is_some() {
            return Ok(());
        }
        let location = self.desktop_presenter.location.clone();
        let presenter = ProjectPresenter::new(name, location);
        self.aggregates.projects.insert(id, presenter)
    }

    fn ensure_launcher_presenter(
        &mut self,
        parent: ProjectId,
        id: LaunchProfileId,
        name: String,
    ) -> Result<()> {
        if self.aggregates.launchers.get(&id).is_some() {
            return Ok(());
        }
        let matrix_location = self
            .aggregates
            .projects
            .get(&parent)
            .with_context(|| format!("project {parent:?} has no presenter"))?
            .matrix
            .location();
        let presenter =
            LauncherPresenter::new(matrix_location, id, name, massive_geometry::Size::default());
        self.aggregates.launchers.insert(id, presenter)
    }

    fn apply_instance_submission(
        &mut self,
        instance: InstanceId,
        submission: InstanceSubmission,
    ) -> Result<ChangeOutput> {
        let (changes, pacing) = submission.into_parts();
        let mut output = ChangeOutput::default();

        for change in changes.release() {
            output.combine(self.apply_instance_change(instance, change)?);
        }

        self.set_instance_pacing(instance, pacing);
        Ok(output)
    }

    fn apply_instance_change(
        &mut self,
        instance: InstanceId,
        change: InstanceChange,
    ) -> Result<ChangeOutput> {
        match change {
            InstanceChange::Scene(change) => {
                submit(change);
                Ok(ChangeOutput::default())
            }
            InstanceChange::CreateView(creation_info) => {
                let mut output = self.present_view(instance, &creation_info)?;
                output.measure(DesktopTarget::Instance(instance));

                // If this instance is currently focused and the new view is primary, make it
                // foreground so that the view is focused. Emitted as a follow-up change so the
                // focus transition (and its navigation-affinity reset) flows through change
                // application like every other focus change.
                if let (Some(DesktopTarget::Instance(focused_instance)), ViewRole::Primary) =
                    (self.event_router.keyboard_focus(), &creation_info.role)
                    && *focused_instance == instance
                {
                    output.changes += set_focus(
                        Some(DesktopTarget::View(creation_info.id)),
                        KeyboardFocusReason::PromotePrimaryView,
                    );
                }
                Ok(output)
            }
            InstanceChange::DestroyView(id) => {
                let view_path: ViewPath = (instance, id).into();
                self.hide_view(view_path)
            }
            InstanceChange::View(view_id, command) => {
                let view_path: ViewPath = (instance, view_id).into();
                self.apply_view_change(view_path, command)?;
                Ok(ChangeOutput::default())
            }
            InstanceChange::Configuration(request) => {
                self.apply_configuration_request(instance, request)
            }
            // This makes sure that all pending Scene Changes from the Instance have been collected
            // before we drop the last ref the instance has to its parent location (which in turn
            // may push other deletes to the Scene).
            InstanceChange::End(_) => Ok(ChangeOutput::default()),
        }
    }

    fn apply_view_change(&mut self, view_path: ViewPath, change: ViewChange) -> Result<()> {
        // We can never be sure if the instance does exist here.
        if let Some(instance) = self.aggregates.instances.get_mut(&view_path.instance) {
            match change {
                ViewChange::Resize(_extends) => {
                    // Resize isn't supported yet.
                    todo!("View Resizes aren't supported yet");
                }
                ViewChange::SetTitle(title) => {
                    debug!("Setting title: {title}");
                    instance.set_view_title(view_path.view, title)?;
                }
                ViewChange::SetCursor(cursor) => {
                    debug!("Setting cursor: {cursor}");
                    instance.set_view_cursor(view_path.view, cursor)?;
                }
            }
        }

        Ok(())
    }

    fn apply_configuration_request(
        &self,
        instance: InstanceId,
        request: ConfigurationRequest,
    ) -> Result<ChangeOutput> {
        let current_project = self
            .aggregates
            .hierarchy
            .project_of_target(&instance.into());
        match &request {
            ConfigurationRequest::AddLauncher => {
                let launcher = self.aggregates.hierarchy.launcher_of_instance(instance);
                let Some((parent, current_placement)) =
                    self.aggregates.configuration.slot_of_launcher(launcher)
                else {
                    warn!("The focused launcher has no matrix placement");
                    return Ok(ChangeOutput::default());
                };

                let changes = self.plan_project(ProjectCommand::AssignSlot {
                    parent,
                    placement: MatrixPlacement {
                        column: current_placement.column + 1,
                        row: current_placement.row,
                    },
                    content: SlotAssignment::Launcher {
                        id: LaunchProfileId::new(),
                        profile: LaunchProfile {
                            name: DEFAULT_NEW_LAUNCHER_NAME.to_string(),
                            mode: LauncherMode::Visor,
                            params: Default::default(),
                        },
                    },
                    shift: SlotShift::default(),
                })?;
                Ok(ChangeOutput::changes(changes))
            }
            ConfigurationRequest::AssignProject {
                name,
                column,
                row,
                under,
                shift,
            } => {
                let parent = match under {
                    Some(path) => self
                        .aggregates
                        .configuration
                        .resolve_project_path(current_project, path),
                    None => Some(current_project),
                };
                let Some(parent) = parent else {
                    warn!("Project path '{under:?}' does not resolve");
                    return Ok(ChangeOutput::default());
                };
                let changes = self.plan_project(ProjectCommand::AssignSlot {
                    parent,
                    placement: MatrixPlacement {
                        column: *column,
                        row: *row,
                    },
                    content: SlotAssignment::Project {
                        id: ProjectId::new(),
                        name: name.clone(),
                    },
                    shift: *shift,
                })?;
                Ok(ChangeOutput::changes(changes))
            }
            ConfigurationRequest::AssignLauncher {
                name,
                column,
                row,
                under,
                shift,
            } => {
                let parent = match under {
                    Some(path) => self
                        .aggregates
                        .configuration
                        .resolve_project_path(current_project, path),
                    None => Some(current_project),
                };
                let Some(parent) = parent else {
                    warn!("Project path '{under:?}' does not resolve");
                    return Ok(ChangeOutput::default());
                };
                let changes = self.plan_project(ProjectCommand::AssignSlot {
                    parent,
                    placement: MatrixPlacement {
                        column: *column,
                        row: *row,
                    },
                    content: SlotAssignment::Launcher {
                        id: LaunchProfileId::new(),
                        profile: LaunchProfile {
                            name: name.clone(),
                            mode: LauncherMode::Visor,
                            params: Default::default(),
                        },
                    },
                    shift: *shift,
                })?;
                Ok(ChangeOutput::changes(changes))
            }
            ConfigurationRequest::RemoveProject { name } => {
                let project = match name {
                    Some(name) => {
                        match self
                            .aggregates
                            .configuration
                            .nearest_project(name, self.focused_project())
                        {
                            Some(project) => project,
                            None => {
                                warn!("Project '{name}' not found");
                                return Ok(ChangeOutput::default());
                            }
                        }
                    }
                    None => current_project,
                };

                if self.aggregates.configuration.project(project).is_none() {
                    warn!("Project is not in the configuration");
                    return Ok(ChangeOutput::default());
                }
                if self
                    .aggregates
                    .hierarchy
                    .parent_project_of(project)
                    .is_none()
                {
                    warn!("The root project cannot be removed");
                    return Ok(ChangeOutput::default());
                }

                Ok(ChangeOutput::changes(
                    self.plan_project(ProjectCommand::RemoveProject(project))?,
                ))
            }
            ConfigurationRequest::RemoveLauncher { name } => {
                let launcher = match name {
                    Some(name) => {
                        match self.aggregates.configuration.nearest_launcher(
                            current_project,
                            name,
                            self.focused_launcher(),
                        ) {
                            Some(launcher) => launcher,
                            None => {
                                warn!("Launcher '{name}' not found in the current project");
                                return Ok(ChangeOutput::default());
                            }
                        }
                    }
                    None => self.aggregates.hierarchy.launcher_of_instance(instance),
                };

                let Some((parent, placement)) =
                    self.aggregates.configuration.slot_of_launcher(launcher)
                else {
                    return Ok(ChangeOutput::default());
                };

                let mut changes = Changes::Empty;
                if let Some(focused) = self.event_router.keyboard_focus()
                    && self
                        .aggregates
                        .hierarchy
                        .path_contains_target(Some(focused), &DesktopTarget::Launcher(launcher))
                {
                    changes += set_focus(
                        Some(self.launcher_removal_focus(launcher, focused)),
                        KeyboardFocusReason::InputTransition,
                    );
                }
                changes += self.plan_project(ProjectCommand::ClearSlot {
                    parent,
                    placement,
                    shift: SlotShift::default(),
                })?;
                Ok(ChangeOutput::changes(changes))
            }
            ConfigurationRequest::MoveLauncher { direction } => {
                let launcher = self.aggregates.hierarchy.launcher_of_instance(instance);
                let Some((parent, current_placement)) =
                    self.aggregates.configuration.slot_of_launcher(launcher)
                else {
                    warn!("The focused launcher has no matrix placement");
                    return Ok(ChangeOutput::default());
                };
                let Some(placement) = current_placement.moved_placement(*direction) else {
                    warn!(
                        "Ignoring {direction:?} launcher move from matrix position ({}, {})",
                        current_placement.column, current_placement.row,
                    );
                    return Ok(ChangeOutput::default());
                };

                let source = (parent, current_placement);
                let destination = (parent, placement);
                if !self
                    .aggregates
                    .configuration
                    .can_move_slot(source, destination)
                {
                    return Ok(ChangeOutput::default());
                }

                let mut changes = Changes::Empty;
                if self
                    .aggregates
                    .configuration
                    .content_at(parent, placement)
                    .is_some()
                {
                    let temporary = self.next_free_root_slot(parent);
                    changes <<= ConfigurationChange::MoveSlot {
                        source,
                        dest: (parent, temporary),
                    };
                    changes <<= ConfigurationChange::MoveSlot {
                        source: destination,
                        dest: source,
                    };
                    changes <<= ConfigurationChange::MoveSlot {
                        source: (parent, temporary),
                        dest: destination,
                    };
                } else {
                    changes += self.plan_project(ProjectCommand::MoveSlot {
                        source,
                        dest: destination,
                    })?;
                }
                Ok(ChangeOutput::changes(changes))
            }
            ConfigurationRequest::PushLauncher { direction } => {
                let launcher = self.aggregates.hierarchy.launcher_of_instance(instance);
                let Some((parent, current_placement)) =
                    self.aggregates.configuration.slot_of_launcher(launcher)
                else {
                    warn!("The focused launcher has no matrix placement");
                    return Ok(ChangeOutput::default());
                };
                match self.slot_shift_sequence(parent, current_placement, *direction) {
                    Ok(changes) => Ok(ChangeOutput::changes(changes)),
                    Err(_) => {
                        warn!(
                            "Ignoring {direction:?} launcher push from matrix position ({}, {})",
                            current_placement.column, current_placement.row,
                        );
                        Ok(ChangeOutput::default())
                    }
                }
            }
            ConfigurationRequest::SetStartup { path } => {
                let path = match path {
                    Some(path) => {
                        let Some(launcher) = self
                            .aggregates
                            .configuration
                            .resolve_launcher_path(current_project, path)
                        else {
                            warn!("Startup path '{path}' does not resolve");
                            return Ok(ChangeOutput::default());
                        };
                        Some(
                            self.aggregates
                                .configuration
                                .launcher_address_path(launcher)?,
                        )
                    }
                    None => None,
                };
                Ok(ChangeOutput::changes(
                    self.plan_project(ProjectCommand::SetStartupPath(path))?,
                ))
            }
            ConfigurationRequest::Resize { size_px } => {
                let mut changes = Changes::Empty;

                // If we are in fullscreen, show the changes by resetting the zoom level, otherwise
                // the user would see nothing.
                if self.focus_depth == FocusDepth::InstanceFullScreen {
                    changes <<= DesktopChange::CommitFocusDepth(FocusDepth::Instance);
                }

                changes <<= DesktopChange::ResizeAll((*size_px).into());

                Ok(ChangeOutput::changes(changes))
            }
            ConfigurationRequest::Undo => todo!(),
            ConfigurationRequest::Redo => todo!(),
        }
    }

    fn next_free_root_slot(&self, project: ProjectId) -> MatrixPlacement {
        let mut row = 0;
        while self
            .aggregates
            .configuration
            .content_at(project, MatrixPlacement { column: 0, row })
            .is_some()
        {
            row += 1;
        }
        MatrixPlacement { column: 0, row }
    }

    fn focused_project(&self) -> Option<ProjectId> {
        let focused = self.event_router.keyboard_focus()?;
        let project = self.aggregates.hierarchy.project_of_target(focused);
        self.aggregates
            .configuration
            .project(project)
            .map(|_| project)
    }

    fn focused_launcher(&self) -> Option<LaunchProfileId> {
        let focused = self.event_router.keyboard_focus()?;
        let launcher = self.aggregates.hierarchy.launcher_of_target(focused)?;
        self.aggregates
            .configuration
            .launcher(launcher)
            .map(|_| launcher)
    }
}

const DEFAULT_NEW_PROJECT_NAME: &str = "New Project";
const DEFAULT_NEW_LAUNCHER_NAME: &str = "New Launcher";

#[cfg(test)]
mod tests {
    use std::path::Path;

    use massive_animation::{AnimationCoordinator, MovementRuntime};
    use massive_geometry::SizePx;
    use massive_layout::LayoutTopology;
    use massive_renderer::{FontManager, ShapingEngineKind};
    use massive_scene::{AnyCollector, SceneChange};

    use super::*;
    use crate::desktop_environment::DesktopEnvironment;
    use crate::instance_manager::InstanceManager;
    use crate::projects::persistence::ConfigurationDocument;
    use massive_applications::task_context::{self, TaskContext};
    use massive_applications::{InstanceEnvironment, InstanceSubmission};
    use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

    const CONFIG: &str =
        "startup \"shell\"\nlauncher \"shell\" column=0 row=0 { command \"echo\" }\n";

    fn task_context() -> TaskContext {
        TaskContext::new(
            AnyCollector::for_type::<SceneChange>(),
            AnimationCoordinator::new(),
            MovementRuntime::default(),
            FontManager::system(ShapingEngineKind::available()[0])
                .expect("system fonts are available")
                .new_shaping_context(),
        )
    }

    fn system() -> (
        DesktopSystem,
        UnboundedReceiver<(InstanceId, InstanceSubmission)>,
    ) {
        let (_sender, receiver) = unbounded_channel();
        let environment = DesktopEnvironment {
            primary_application: "terminal".into(),
            applications: crate::application_registry::ApplicationRegistry::new(Vec::new()),
            projects_dir: None,
        };
        let (document, aggregate) =
            ConfigurationDocument::from_str(Path::new("/config/desktop.kdl"), CONFIG).unwrap();
        let system =
            DesktopSystem::new(environment, SizePx::new(800, 600), document, aggregate).unwrap();
        (system, receiver)
    }

    fn instance_manager(
        _receiver: &UnboundedReceiver<(InstanceId, InstanceSubmission)>,
    ) -> InstanceManager {
        let (sender, _receiver) = unbounded_channel();
        InstanceManager::new(InstanceEnvironment::new(sender, 1.0))
    }

    #[tokio::test]
    async fn add_project_under_none_creates_the_root_under_desktop() -> Result<()> {
        task_context::with_context(task_context(), async {
            let (mut system, receiver) = system();
            let mut instance_manager = instance_manager(&receiver);

            let changes = system.plan(DesktopCommand::Project(ProjectCommand::AddProject {
                id: ProjectId::ROOT,
                name: crate::projects::ROOT_PROJECT_NAME.into(),
                placement: MatrixPlacement { column: 0, row: 0 },
                under: None,
            }))?;
            system.transact(
                changes,
                &mut instance_manager,
                TransactionEffectsMode::Setup,
                SizePx::new(800, 600),
            )?;

            let hierarchy = &system.aggregates.hierarchy;
            assert!(
                hierarchy
                    .parent_of(&DesktopTarget::Project(ProjectId::ROOT))
                    .is_some(),
                "the root project hangs under the implicit Desktop root"
            );
            assert!(hierarchy.exists(&DesktopTarget::Desktop));
            assert!(system.aggregates.projects.get(&ProjectId::ROOT).is_some());

            Ok(())
        })
        .await
    }

    #[tokio::test]
    async fn a_second_parentless_add_project_is_rejected() -> Result<()> {
        task_context::with_context(task_context(), async {
            let (mut system, receiver) = system();
            let mut instance_manager = instance_manager(&receiver);

            let changes = system.plan(DesktopCommand::Project(ProjectCommand::AddProject {
                id: ProjectId::ROOT,
                name: crate::projects::ROOT_PROJECT_NAME.into(),
                placement: MatrixPlacement { column: 0, row: 0 },
                under: None,
            }))?;
            system.transact(
                changes,
                &mut instance_manager,
                TransactionEffectsMode::Setup,
                SizePx::new(800, 600),
            )?;

            for id in [ProjectId::ROOT, ProjectId::new()] {
                assert!(
                    system
                        .plan(DesktopCommand::Project(ProjectCommand::AddProject {
                            id,
                            name: "again".into(),
                            placement: MatrixPlacement { column: 0, row: 0 },
                            under: None,
                        }))
                        .is_err(),
                    "a second parentless AddProject must be rejected"
                );
            }

            Ok(())
        })
        .await
    }

    #[tokio::test]
    async fn the_boot_replay_builds_the_tree_through_plan_and_transact() -> Result<()> {
        task_context::with_context(task_context(), async {
            let (mut system, receiver) = system();
            let mut instance_manager = instance_manager(&receiver);

            let (_, aggregate) = ConfigurationDocument::from_str(
                Path::new("/config/desktop.kdl"),
                "project \"work\" {\n    launcher \"shell\" column=0 row=0\n}\n",
            )?;
            for command in crate::projects::to_commands(&aggregate) {
                let changes = system.plan(DesktopCommand::Project(command))?;
                system.transact(
                    changes,
                    &mut instance_manager,
                    TransactionEffectsMode::Setup,
                    SizePx::new(800, 600),
                )?;
            }

            let hierarchy = &system.aggregates.hierarchy;
            assert!(hierarchy.exists(&DesktopTarget::Desktop));
            let root = DesktopTarget::Project(ProjectId::ROOT);
            assert!(hierarchy.exists(&root));
            assert!(hierarchy.exists(&DesktopTarget::ProjectMatrix(ProjectId::ROOT)));
            let nested_slot = hierarchy
                .get_nested(&DesktopTarget::ProjectMatrix(ProjectId::ROOT))
                .first()
                .cloned()
                .expect("the boot replay assigned the parsed project slot");
            assert!(matches!(nested_slot, DesktopTarget::Project(_)));

            Ok(())
        })
        .await
    }
}
