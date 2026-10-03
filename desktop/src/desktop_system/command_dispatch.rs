use anyhow::{Context, Result, bail};
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
    DEFAULT_NEW_LAUNCHER_NAME, LaunchProfile, LaunchProfileId, LauncherMode, LauncherPresenter,
    MatrixPlacement, ProjectId, ProjectPresenter, SlotAssignment, SlotIds,
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
            ProjectCommand::AssignSlot {
                parent,
                placement,
                content,
                shift,
            } => {
                let Some(parent) = parent else {
                    return self.plan_root_creation(placement, content);
                };
                // The parsed aggregate already has boot slots, while the topology
                // starts empty; only displace occupants that are live in the scene.
                let assigned = self
                    .aggregates
                    .configuration
                    .ids_at(parent, placement)
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
                    parent: Some(parent),
                    placement,
                    assignment: content.clone(),
                };
                changes <<= TopologyChange::Add {
                    what: content.content().target(),
                    under: DesktopTarget::ProjectMatrix(parent),
                    after: None,
                };
                if let SlotIds::Project(project) = content.content() {
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

    /// Creating the root is a parentless assignment of [`ProjectId::ROOT`] under
    /// the `Desktop` target: the root is hosted by no project and lives in no
    /// slot, so `placement` is carried through but never slotted.
    fn plan_root_creation(
        &self,
        placement: MatrixPlacement,
        content: SlotAssignment,
    ) -> Result<Changes> {
        let (id, name) = match content {
            SlotAssignment::Project { id, name } => (id, name),
            // Only boot plans a parentless assignment, and it only creates the root.
            SlotAssignment::Launcher { .. } => {
                panic!("a launcher is always assigned into a project's slot")
            }
        };
        if id != ProjectId::ROOT
            || self
                .aggregates
                .hierarchy
                .exists(&DesktopTarget::Project(ProjectId::ROOT))
        {
            bail!(
                "Internal error (plan AssignSlot): the root project is created exactly once, as `AssignSlot {{ parent: None }}` with id {:?}",
                ProjectId::ROOT
            );
        }
        let mut changes: Changes = Changes::Empty;
        changes <<= ConfigurationChange::AssignSlot {
            parent: None,
            placement,
            assignment: SlotAssignment::Project { id, name },
        };
        changes <<= TopologyChange::Add {
            what: DesktopTarget::Project(id),
            under: DesktopTarget::Desktop,
            after: None,
        };
        changes <<= TopologyChange::AddNested {
            what: [
                DesktopTarget::ProjectHeader(id),
                DesktopTarget::ProjectMatrix(id),
            ]
            .into(),
            under: DesktopTarget::Project(id),
        };
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
        let Some(content) = self.aggregates.configuration.ids_at(parent, placement) else {
            return Ok(Changes::Empty);
        };

        let mut changes = Changes::Empty;
        // Retarget focus before the content leaves the topology, so the removal does
        // not fall back to the generic parent retarget.
        changes += self.clear_slot_focus(content);
        match content {
            SlotIds::Launcher(launcher) => {
                changes += self.plan_remove_launcher_instances(launcher);
                changes <<= TopologyChange::Remove(DesktopTarget::Launcher(launcher));
            }
            SlotIds::Project(project) => {
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

    /// Retargets keyboard focus when the cleared slot holds it, so the removal does
    /// not leave the router pointing at a target about to leave the topology: the
    /// removed content gets a neighbouring replacement instead of its parent.
    fn clear_slot_focus(&self, content: SlotIds) -> Changes {
        let Some(focused) = self.event_router.keyboard_focus() else {
            return Changes::Empty;
        };
        if !self
            .aggregates
            .hierarchy
            .path_contains_target(Some(focused), &content.target())
        {
            return Changes::Empty;
        }
        let replacement = match content {
            SlotIds::Launcher(launcher) => self.launcher_removal_focus(launcher, focused),
            SlotIds::Project(project) => self.project_removal_focus(project),
        };
        set_focus(Some(replacement), KeyboardFocusReason::InputTransition)
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
                SlotIds::Launcher(launcher) => {
                    changes += self.plan_remove_launcher_instances(launcher);
                    changes <<= TopologyChange::Remove(DesktopTarget::Launcher(launcher));
                }
                SlotIds::Project(nested) => {
                    changes += self.plan_remove_project_content(nested);
                    changes <<= TopologyChange::Remove(DesktopTarget::Project(nested));
                }
            }
        }
        changes
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
                let output = self.apply_project_change(project_change)?;
                // A setup change only updates the live model; it must not mark the
                // file pending — setup's changes are the ones the file already
                // carries.
                if effects_mode != TransactionEffectsMode::Setup {
                    self.configuration.mark_pending();
                }
                return Ok(output);
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
            ConfigurationChange::AssignSlot {
                parent,
                placement,
                assignment,
            } => {
                let target = assignment.content();
                // The presenter is built from what the aggregate stored, not what
                // the payload claimed: the host comes back from the mutation, the
                // stored name is read from the aggregate (it may have been
                // re-indexed to stay unique).
                let host = self
                    .aggregates
                    .configuration
                    .assign_slot(parent, placement, assignment);
                self.insert_slot_presenter(target, host)?;
            }
            ConfigurationChange::ClearSlot { parent, placement } => {
                if let Some(content) = self.aggregates.configuration.ids_at(parent, placement) {
                    match content {
                        SlotIds::Launcher(launcher) => {
                            self.aggregates.launchers.remove(&launcher)?;
                        }
                        SlotIds::Project(project) => {
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
            // does not retain the resolved launcher. The path is what persists, so
            // the aggregate records it and the document derives from it.
            ConfigurationChange::SetStartupPath(path) => {
                self.aggregates.configuration.set_startup_path(path);
            }
        }

        Ok(ChangeOutput::default())
    }

    /// Builds the presenter for newly assigned slot content. The host is the
    /// one `assign_slot` returned — no re-search over the aggregate. The stored
    /// name is read back from the aggregate, so the builder sees what was
    /// actually stored rather than what the change payload claimed.
    ///
    /// A launcher always has a hosting project (`None` is an invariant
    /// violation); the root project is hosted by the desktop itself and takes
    /// the desktop location instead of a matrix's.
    fn insert_slot_presenter(&mut self, content: SlotIds, host: Option<ProjectId>) -> Result<()> {
        match content {
            SlotIds::Launcher(id) => {
                let parent = host.context(format!("a launcher is hosted by a project: {id:?}"))?;
                let name = self.aggregates.configuration[id].name.clone();
                self.insert_launcher_presenter(parent, id, name)
            }
            SlotIds::Project(id) => {
                let name = self.aggregates.configuration[id].name.clone();
                self.insert_project_presenter(host, id, name)
            }
        }
    }

    /// Inserts the projector presenter for a newly created project. The plan
    /// guarantees the id does not exist yet — the root is created by a parentless
    /// `AssignSlot`, and every nested project arrives via `AssignSlot` with a
    /// fresh id — so an existing presenter is an invariant violation and fails
    /// loudly here.
    fn insert_project_presenter(
        &mut self,
        host: Option<ProjectId>,
        id: ProjectId,
        name: String,
    ) -> Result<()> {
        // A nested project's scene node hangs under the matrix that hosts its slot:
        // its layout transform is relative to that matrix. The root project — with
        // no host — is placed in the desktop's own space.
        let location = match host.map(|parent| self.aggregates.project_matrix_location(parent)) {
            Some(location) => location,
            None => self.desktop_presenter.location.clone(),
        };
        let presenter = ProjectPresenter::new(name, location);
        self.aggregates.projects.insert(id, presenter)
    }

    /// Inserts the launcher presenter for a newly assigned launcher. `AssignSlot`
    /// never re-assigns an existing launcher id, so an existing presenter is an
    /// invariant violation and fails loudly here.
    fn insert_launcher_presenter(
        &mut self,
        parent: ProjectId,
        id: LaunchProfileId,
        name: String,
    ) -> Result<()> {
        let matrix_location = self.aggregates.project_matrix_location(parent);
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
                    parent: Some(parent),
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
                    parent: Some(parent),
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
                    parent: Some(parent),
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

                // The root is hosted by the `Desktop` target rather than a slot, so
                // it has no parent project to be cleared from.
                if project == ProjectId::ROOT {
                    warn!("The root project cannot be removed");
                    return Ok(ChangeOutput::default());
                }
                let parent = self
                    .aggregates
                    .hierarchy
                    .parent_project_of(project)
                    .expect("a non-root project hangs under its parent's matrix");
                let placement = self
                    .aggregates
                    .configuration
                    .project(parent)
                    .and_then(|parent| parent.placement_of_content(SlotIds::Project(project)))
                    .expect("the parent project holds the nested project in a slot");

                Ok(ChangeOutput::changes(self.plan_project(
                    ProjectCommand::ClearSlot {
                        parent,
                        placement,
                        shift: SlotShift::default(),
                    },
                )?))
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

                Ok(ChangeOutput::changes(self.plan_project(
                    ProjectCommand::ClearSlot {
                        parent,
                        placement,
                        shift: SlotShift::default(),
                    },
                )?))
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
                    .ids_at(parent, placement)
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
            .ids_at(project, MatrixPlacement { column: 0, row })
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use massive_animation::{AnimationCoordinator, MovementRuntime};
    use massive_geometry::{Rect, RectPx, SizePx};
    use massive_layout::LayoutTopology;
    use massive_renderer::{FontManager, ShapingEngineKind};
    use massive_scene::{AnyCollector, SceneChange};

    use super::*;
    use crate::desktop_environment::DesktopEnvironment;
    use crate::instance_manager::InstanceManager;
    use crate::projects::persistence::{ConfigurationPersistence, parse_configuration};
    use massive_applications::task_context::{self, TaskContext};
    use massive_applications::{InstanceEnvironment, InstanceSubmission};
    use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

    const CONFIG: &str = r#"{
        "startup": "/shell",
        "slots": [
            { "at": [0, 0], "launcher": { "name": "shell", "mode": "visor", "params": { "command": "echo" } } }
        ]
    }"#;

    /// The startup launcher nested in a project, so it sits inside a project slot.
    const NESTED_PROJECT_CONFIG: &str = r#"{
        "startup": "/labs/shell",
        "slots": [
            { "at": [0, 0], "project": { "name": "labs", "slots": [
                { "at": [0, 0], "launcher": { "name": "shell", "mode": "visor" } }
            ] } }
        ]
    }"#;

    #[tokio::test]
    async fn a_parentless_assign_slot_creates_the_root_under_desktop() -> Result<()> {
        task_context::with_context(task_context(), async {
            let (mut system, receiver) = system();
            let mut instance_manager = instance_manager(&receiver);

            let changes = system.plan(DesktopCommand::Project(ProjectCommand::AssignSlot {
                parent: None,
                placement: MatrixPlacement { column: 0, row: 0 },
                content: SlotAssignment::Project {
                    id: ProjectId::ROOT,
                    name: crate::projects::ROOT_PROJECT_NAME.into(),
                },
                shift: SlotShift::default(),
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
    async fn startup_setup_snaps_camera_to_the_focused_instance() -> Result<()> {
        task_context::with_context(task_context(), async {
            let (mut system, receiver) = system();
            let mut instance_manager = instance_manager(&receiver);
            let launcher = system
                .aggregates
                .configuration
                .boot_launcher()
                .expect("the test configuration has a startup launcher");
            let frame = massive_applications::begin_frame();
            for command in crate::projects::to_commands(&system.aggregates.configuration) {
                let changes = system.plan(DesktopCommand::Project(command))?;
                system.transact(
                    changes,
                    &mut instance_manager,
                    TransactionEffectsMode::Setup,
                    SizePx::new(800, 600),
                )?;
            }

            let instance = uuid::Uuid::new_v4().into();
            let start = system.plan(DesktopCommand::StartInstance {
                launcher,
                instance,
                root: Some(InstanceRoot::new()),
                parameters: Default::default(),
            })?;
            let mut submission_changes = massive_util::ChangeSet::default();
            submission_changes.push(InstanceChange::CreateView(
                massive_applications::ViewCreationInfo {
                    id: uuid::Uuid::new_v4().into(),
                    role: ViewRole::Primary,
                    extents: massive_geometry::BoxPx::new(
                        massive_geometry::PointPx::new(0, 0),
                        massive_geometry::PointPx::new(800, 600),
                    ),
                },
            ));
            let initial_submission = InstanceSubmission::new(
                submission_changes,
                massive_renderer::RenderPacing::default(),
            );
            let mut changes: Changes = start;
            changes <<= DesktopChange::IntegrateInstanceSubmission(instance, initial_submission);
            system.transact(
                changes,
                &mut instance_manager,
                TransactionEffectsMode::Setup,
                SizePx::new(800, 600),
            )?;

            assert_eq!(
                system.event_router.keyboard_focus(),
                Some(&DesktopTarget::View(
                    system.aggregates.instances[&instance]
                        .primary_view_id()
                        .expect("the initial submission created a primary view")
                ))
            );
            assert_eq!(system.focus_depth, FocusDepth::Instance);
            let focused = system.event_router.keyboard_focus().unwrap();
            let expected = system.resolve_camera_for_target_or_ancestor(
                focused,
                system.focus_depth,
                SizePx::new(800, 600),
            );
            assert_eq!(
                *system.camera(),
                expected,
                "setup should snap to the focused instance camera without an initial transition"
            );

            drop(frame.submission::<SceneChange>());
            Ok(())
        })
        .await
    }

    /// The camera and the hover read a target's absolute placement, so the scene must
    /// resolve that target to the same transform — a project nested in a parent slot
    /// included.
    #[tokio::test]
    async fn nested_project_launcher_scene_transform_matches_its_placement() -> Result<()> {
        task_context::with_context(task_context(), async {
            let (mut system, receiver) = system_from(NESTED_PROJECT_CONFIG);
            let mut instance_manager = instance_manager(&receiver);
            let frame = massive_applications::begin_frame();
            for command in crate::projects::to_commands(&system.aggregates.configuration) {
                let changes = system.plan(DesktopCommand::Project(command))?;
                system.transact(
                    changes,
                    &mut instance_manager,
                    TransactionEffectsMode::Setup,
                    SizePx::new(800, 600),
                )?;
            }

            let launcher = system
                .aggregates
                .configuration
                .boot_launcher()
                .expect("the test configuration has a startup launcher");
            let location = system.aggregates.launchers[&launcher].location();
            let resolved = massive_scene::TransformResolver::default()
                .resolve(&location.to_ref())
                .transform;

            let placement = system.placement(&DesktopTarget::Launcher(launcher));
            let rect_px: RectPx = placement.rect.into();
            let local_center = Rect::from(rect_px).size().to_rect().center();
            let expected = placement.transform.to_origin_space(local_center);

            assert_eq!(resolved.scale, expected.scale);
            let offset = resolved.translate - expected.translate;
            assert!(
                offset.x.abs() < 0.001 && offset.y.abs() < 0.001 && offset.z.abs() < 0.001,
                "the scene draws the launcher at {resolved:?}, but the model places it at {expected:?}"
            );

            drop(frame.submission::<SceneChange>());
            Ok(())
        })
        .await
    }

    #[tokio::test]
    async fn a_second_parentless_assign_slot_is_rejected() -> Result<()> {
        task_context::with_context(task_context(), async {
            let (mut system, receiver) = system();
            let mut instance_manager = instance_manager(&receiver);

            let changes = system.plan(DesktopCommand::Project(ProjectCommand::AssignSlot {
                parent: None,
                placement: MatrixPlacement { column: 0, row: 0 },
                content: SlotAssignment::Project {
                    id: ProjectId::ROOT,
                    name: crate::projects::ROOT_PROJECT_NAME.into(),
                },
                shift: SlotShift::default(),
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
                        .plan(DesktopCommand::Project(ProjectCommand::AssignSlot {
                            parent: None,
                            placement: MatrixPlacement { column: 0, row: 0 },
                            content: SlotAssignment::Project {
                                id,
                                name: "again".into(),
                            },
                            shift: SlotShift::default(),
                        }))
                        .is_err(),
                    "a second parentless AssignSlot must be rejected"
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

            let aggregate = parse_configuration(
                Path::new("/config/desktop.json"),
                r#"{ "slots": [ { "at": [0, 0], "project": { "name": "work", "slots": [
                    { "at": [0, 0], "launcher": { "name": "shell", "mode": "visor" } }
                ] } } ] }"#,
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
        system_from(CONFIG)
    }

    fn system_from(
        config: &str,
    ) -> (
        DesktopSystem,
        UnboundedReceiver<(InstanceId, InstanceSubmission)>,
    ) {
        let (_sender, receiver) = unbounded_channel();
        let environment = DesktopEnvironment {
            primary_application: "terminal".into(),
            applications: crate::application_registry::ApplicationRegistry::new(Vec::new()),
            projects_dir: None,
        };
        let path = Path::new("/config/desktop.json");
        let aggregate = parse_configuration(path, config).unwrap();
        let document = ConfigurationPersistence::new(path);
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
}
