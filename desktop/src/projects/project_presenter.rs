use massive_applications::prelude::*;
use massive_geometry::{Color, SizedTransform, Transform};
use massive_scene::prelude::*;

use crate::desktop_system::change::Zoom;
use crate::desktop_system::{
    DesktopCommand, KeyContext, KeyHandler, KeyInput, KeyOutcome, Shortcut,
};
use crate::projects::{MatrixPlacement, ProjectId};

use super::title_bar_presenter::{TitleBarPresenter, TitleBarStyle};

const PROJECT_HEADER_FONT_SIZE: f32 = 16.0 * 8.0;
const PROJECT_HEADER_BACKGROUND_COLOR: Color = Color::rgb_u32(0x1f4d3d);

#[derive(Debug)]
pub struct ProjectPresenter {
    id: ProjectId,
    scene_transform: Handle<Transform>,
    pub header: TitleBarPresenter,
    pub matrix: ProjectMatrixPresenter,
    pub last_focused_placement: Option<MatrixPlacement>,
}

impl ProjectPresenter {
    pub fn new(id: ProjectId, name: String, parent_location: Handle<Location>) -> Self {
        let (scene_transform, location) =
            identity_location().relative_to(&parent_location).submit();
        let mut header = TitleBarPresenter::new(
            TitleBarStyle {
                background_color: PROJECT_HEADER_BACKGROUND_COLOR,
                font_size: PROJECT_HEADER_FONT_SIZE,
                indent: 0.0,
            },
            location.clone(),
        );
        header.set_text(&name);
        let matrix = ProjectMatrixPresenter::new(location.clone());

        Self {
            id,
            scene_transform,
            header,
            matrix,
            last_focused_placement: None,
        }
    }

    pub fn set_layout(&mut self, layout: SizedTransform) {
        let scene_transform = layout.to_origin_space();
        self.scene_transform.update_if_changed(scene_transform);
    }
}

/// A project receives no text input, so Enter, with or without Cmd, enters it while it is the
/// keyboard focus (ADR 0018).
impl KeyHandler for ProjectPresenter {
    fn handle_key(&self, input: &KeyInput, context: &KeyContext<'_>) -> KeyOutcome {
        let enters = matches!(
            Shortcut::from_input(input),
            Some(Shortcut::Enter | Shortcut::CommandEnter)
        ) && context.focused().is_some_and(|focused| {
            focused.enters_on_plain_enter() && focused.stands_for_project() == Some(self.id)
        });

        if enters {
            KeyOutcome::Consumed(DesktopCommand::Zoom(Zoom::Enter).into())
        } else {
            KeyOutcome::Pass
        }
    }
}

#[derive(Debug)]
pub struct ProjectMatrixPresenter {
    scene_transform: Handle<Transform>,
    location: Handle<Location>,
}

impl ProjectMatrixPresenter {
    pub fn new(parent_location: Handle<Location>) -> Self {
        let (scene_transform, location) =
            identity_location().relative_to(&parent_location).submit();

        Self {
            scene_transform,
            location,
        }
    }

    pub fn location(&self) -> Handle<Location> {
        self.location.clone()
    }

    pub fn set_layout(&mut self, layout: SizedTransform) {
        let scene_transform = layout.to_origin_space();
        self.scene_transform.update_if_changed(scene_transform);
    }
}
