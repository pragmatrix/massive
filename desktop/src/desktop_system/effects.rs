//! The desktop's effects: the work that a transaction's changes make necessary, the kinds whose
//! results each of them reads, and the task-local through which the desktop marks them (ADR 0043).

use std::cell::RefCell;
use std::sync::LazyLock;

use strum::{EnumDiscriminants, VariantArray};
use tokio::task_local;

use super::DesktopTarget;
use crate::effect_scheduler::{self, Effect, EffectGraph, EffectKind, PendingEffects};
use crate::projects::ProjectId;

/// Work that a transaction's changes make necessary (ADR 0043). Its kind, `DesktopEffectKind`, is
/// generated from the variants, so the two never drift apart.
#[derive(Debug, Clone, PartialEq, Eq, Hash, EnumDiscriminants)]
#[strum_discriminants(name(DesktopEffectKind), derive(VariantArray))]
pub enum DesktopEffect {
    /// Refreshes the shared column counts after configuration changes edited the project's slots,
    /// or drops the counts of a project they removed (ADR 0035).
    RefreshColumnCount(ProjectId),
    /// Refreshes the MCP indicators of the project and everything below it, after configuration
    /// changes may have changed them (ADR 0036).
    RefreshMcpIndicators(ProjectId),
    /// Collapses the selection unless the transaction set it. Changing the focus or the zoom level
    /// marks it, and so does a configuration change that moves slots under the selection
    /// (ADR 0025, ADR 0033).
    CollapseSelection,
    /// Shows the zoom level indicator for the focused target, except in setup transactions
    /// (ADR 0018).
    ShowZoomLevel,
    Measure(DesktopTarget),
    Place(DesktopTarget),
    /// Converts the on-screen position of a target that moved to a new parent into that parent's
    /// coordinates, so that it slides from where it was instead of jumping.
    Reparent(DesktopTarget),
    ApplyLayout(DesktopTarget),
    /// Resolves the camera's desired view, once all placements are final.
    UpdateDesiredCamera,
    /// Moves the presented camera toward the desired view in the transaction's camera mode.
    SynchronizeCamera,
    /// Places the zoom level indicator in the window's top-right corner at the current window size.
    LayoutZoomLevelIndicator,
    /// Moves the hover outline to the pointer focus, or to the keyboard focus or selection.
    UpdateHover,
    /// Asks the host to set the window's fullscreen state the transaction requested.
    SetWindowFullScreen,
    /// Asks the host to close the tool connections bound to the MCP root (ADR 0036).
    AnnounceMcpRoot(ProjectId),
    /// Asks the host to persist the configuration, except in setup transactions (ADR 0013).
    PersistConfiguration,
}

impl EffectKind for DesktopEffectKind {
    const ALL: &'static [Self] = Self::VARIANTS;

    fn index(self) -> usize {
        self as usize
    }

    /// The order of the desktop's effects follows from these reads alone (ADR 0043).
    fn reads(self) -> &'static [Self] {
        use DesktopEffectKind::*;
        match self {
            RefreshColumnCount | RefreshMcpIndicators | CollapseSelection | ShowZoomLevel => &[],
            // A matrix lays out its column count, and a header measures its MCP indicator text.
            // A target is measured from the sizes of its children.
            Measure => &[RefreshColumnCount, RefreshMcpIndicators, Measure],
            // A child is placed within the placement of its parent.
            Place => &[Measure, Place],
            // The new parent's placement must be final.
            Reparent => &[Place],
            ApplyLayout => &[Place, Reparent],
            // The camera frames the placed focus and keeps the selection in view (ADR 0032).
            UpdateDesiredCamera => &[Measure, Place, CollapseSelection],
            SynchronizeCamera => &[UpdateDesiredCamera],
            LayoutZoomLevelIndicator => &[ShowZoomLevel],
            // The hover outline marks a placement, the selection, or the keyboard focus.
            UpdateHover => &[Place, CollapseSelection],
            SetWindowFullScreen | AnnounceMcpRoot | PersistConfiguration => &[],
        }
    }
}

impl Effect for DesktopEffect {
    type Kind = DesktopEffectKind;

    fn kind(&self) -> DesktopEffectKind {
        self.into()
    }

    fn without_subject(kind: DesktopEffectKind) -> Option<Self> {
        use DesktopEffectKind::*;
        match kind {
            CollapseSelection => Some(Self::CollapseSelection),
            ShowZoomLevel => Some(Self::ShowZoomLevel),
            UpdateDesiredCamera => Some(Self::UpdateDesiredCamera),
            SynchronizeCamera => Some(Self::SynchronizeCamera),
            LayoutZoomLevelIndicator => Some(Self::LayoutZoomLevelIndicator),
            UpdateHover => Some(Self::UpdateHover),
            SetWindowFullScreen => Some(Self::SetWindowFullScreen),
            PersistConfiguration => Some(Self::PersistConfiguration),
            RefreshColumnCount | RefreshMcpIndicators | Measure | Place | Reparent
            | ApplyLayout | AnnounceMcpRoot => None,
        }
    }
}

impl DesktopEffect {
    /// Marks this effect in the current transaction (ADR 0043).
    pub fn mark(self) {
        effect_scheduler::mark(&PENDING, self);
    }

    /// The target the effect works on, if its subject is one. A later change of the same
    /// transaction may have removed it.
    pub fn target(&self) -> Option<&DesktopTarget> {
        match self {
            Self::Measure(target)
            | Self::Place(target)
            | Self::Reparent(target)
            | Self::ApplyLayout(target) => Some(target),
            _ => None,
        }
    }
}

task_local! {
    static PENDING: RefCell<PendingEffects<DesktopEffect>>;
}

/// Built by the first transaction, the desktop's setup at startup, so a cycle in the reads panics
/// there.
static GRAPH: LazyLock<EffectGraph<DesktopEffect>> = LazyLock::new(EffectGraph::new);

/// Runs `f` as one transaction, in which the desktop's effects can be marked.
pub fn scope<R>(f: impl FnOnce() -> R) -> R {
    effect_scheduler::scope(&PENDING, &GRAPH, f)
}

/// Drops the pending effects that `keep` rejects, between a transaction's changes and its effects.
///
/// All changes of a transaction run before its effects (ADR 0043). So an earlier change can mark an
/// effect for a target that a later change of the same transaction removes, such as `Measure` of a
/// launcher whose slot the next change clears. Handling it would work on a target the hierarchy no
/// longer has, so `transact` drops such effects here.
pub fn retain(keep: impl FnMut(&DesktopEffect) -> bool) {
    effect_scheduler::retain(&PENDING, keep);
}

/// Handles the pending effects of the current transaction until none are pending.
pub fn run(handle: impl FnMut(DesktopEffect) -> anyhow::Result<()>) -> anyhow::Result<()> {
    effect_scheduler::run(&PENDING, handle)
}
