use std::{
    mem,
    sync::{
        Arc,
        mpsc::{self, TryRecvError},
    },
};

use anyhow::{Result, anyhow};
use parking_lot::Mutex;
use tokio::sync::mpsc::WeakUnboundedSender;

use massive_geometry::{Color, Matrix4, SizePx};
use massive_scene::SceneChanges;
use massive_util::message_filter;

use crate::{ShellEvent, WindowRenderer};

#[derive(Debug)]
pub enum RendererMessage {
    Resize(SizePx),
    Redraw,
    SetBackgroundColor(Option<Color>),
    // Protocol: When adding a new RenderMessage, consider message_filter::keep_last_per_variant().
}

// Detail: The render loop will only end regularly if the channel that sends renderer messages
// is closed.
pub fn render_thread(
    msg_receiver: mpsc::Receiver<RendererMessage>,
    mut window_renderer: WindowRenderer,
    submission: Arc<Mutex<RenderThreadSubmission>>,
    shell_events: WeakUnboundedSender<ShellEvent>,
) -> Result<()> {
    let mut messages = Vec::new();

    loop {
        // Detail: Because the previous event may take some time to process, there might be some
        // additional messages pending, but we don't pull them to avoid getting into a situation
        // in smooth rendering that there is never a rendering.
        let smooth = window_renderer.present_mode() == wgpu::PresentMode::AutoVsync;
        if messages.is_empty() {
            // blocking path.
            if smooth {
                // Smooth rendering. This may block.
                render_frame(&mut window_renderer, &shell_events, &submission)?;
            } else {
                // Fast mode. Wait until at least one event is there.
                if wait_for_events(&msg_receiver, &mut messages) != FlowControl::Continue {
                    return Ok(());
                };
            }
        }

        // Pull _all_ events so that we can coalesce them.
        if retrieve_pending_events(&msg_receiver, &mut messages) != FlowControl::Continue {
            return Ok(());
        };
        messages = message_filter::keep_last_per_variant(messages, |_| true);

        if messages.is_empty() {
            continue;
        }
        // Detail: I think there could only be 4 events in there because of keep_last_per_variant?
        match messages.remove(0) {
            RendererMessage::Resize(new_size) => {
                // Optimization: If we resize and change present mode the same time, we would only need to reconfigure
                // the surface once. Renderer might even reconfigure lazily.
                window_renderer.resize(new_size);
            }
            RendererMessage::Redraw => {
                // In smooth mode, we ignore explicit redraw requests.
                if !smooth {
                    render_frame(&mut window_renderer, &shell_events, &submission)?;
                } else {
                    // Architecture: Well, what to do with all the Redraw requests in smooth
                    // rendering mode? Currently the problem is that we don't even know when to send
                    // them from the AsyncWindowRenderer. We could look into RenderThreadSubmission
                    // for that, but is this really the right way?

                    // Currently, it does feel better to have every Redraw handled, because we don't
                    // want to end up in rare situation where the last frame of animation isn't
                    // rendered.
                }
            }
            RendererMessage::SetBackgroundColor(color) => {
                window_renderer.set_background_color(color);
            }
        }
    }
}

/// Wait until events are available. Blocks if none available.
///
/// Blocks until at least one event is available.
fn wait_for_events(
    msg_receiver: &mpsc::Receiver<RendererMessage>,
    events: &mut Vec<RendererMessage>,
) -> FlowControl {
    if !events.is_empty() {
        return FlowControl::Continue;
    }

    let Ok(msg) = msg_receiver.recv() else {
        return FlowControl::Disconnected;
    };

    events.push(msg);
    FlowControl::Continue
}

/// Retrieve all pending events. Non blocking. Ignores when the channel gets closed.
fn retrieve_pending_events(
    msg_receiver: &mpsc::Receiver<RendererMessage>,
    events: &mut Vec<RendererMessage>,
) -> FlowControl {
    loop {
        match msg_receiver.try_recv() {
            Ok(event) => {
                events.push(event);
            }
            Err(TryRecvError::Disconnected) => return FlowControl::Disconnected,
            Err(TryRecvError::Empty) => return FlowControl::Continue,
        }
    }
}

// Detail: This always produces a new frame. Even if there are no changes.
fn render_frame(
    renderer: &mut WindowRenderer,
    apply_animations_to: &WeakUnboundedSender<ShellEvent>,
    submission: &Arc<Mutex<RenderThreadSubmission>>,
) -> Result<()> {
    // Detail: In VSync presentation mode, this blocks until the next VSync beginning
    // with the second frame after that. Therefore we apply scene changes afterwards.
    // This improves time of first change to render time considerably.
    let texture = renderer.get_next_texture()?;

    // Detail: Presentation timestamps are only sent when the presentation waited for a VSync.
    if renderer.present_mode() == wgpu::PresentMode::AutoVsync {
        let sender = apply_animations_to
                .upgrade()
                .ok_or(anyhow!("Failed to dispatch apply animations (no receiver for ShellEvents anymore, application vanished)"))?;

        sender.send(ShellEvent::ApplyAnimations(renderer.window_id()))?;
    }

    let submission = submission.lock().take();

    // Apply scene changes after we retrieved the texture (because retrieving the texture may
    // take time, we want to wait until the last moment before pulling changes), even though the
    // time between retrieving the texture and final rendering takes longer. This reduces lag
    // noticeably (for example in the logs example while smooth rendering).
    renderer.apply_scene_changes(submission.changes)?;

    renderer.render_and_present(&submission.view_projection, texture);

    // Update pacing now

    // Detail: Switching from NoVSync to VSync takes ~200 microseconds,
    // from VSync to NoVSync around ~2.7 milliseconds (measured in the logs example --release).

    // Robustness: Does this wait for the frame to be rendered, or is it lost? If so, should do this
    // before get_next_texture()?
    renderer.set_present_mode(submission.present_mode);

    Ok(())
}

#[must_use]
#[derive(Debug, PartialEq, Eq)]
enum FlowControl {
    Continue,
    Disconnected,
}

/// An extended accumulable submission structure that contains everything the renderer needs to know.
#[derive(Debug)]
pub struct RenderThreadSubmission {
    // We basically ignore the camera in there.
    pub changes: SceneChanges,
    pub present_mode: wgpu::PresentMode,
    pub view_projection: Matrix4,
}

impl RenderThreadSubmission {
    pub fn new(view_projection: Matrix4) -> Self {
        Self {
            changes: SceneChanges::default(),
            view_projection,
            present_mode: wgpu::PresentMode::AutoNoVsync,
        }
    }

    pub fn take(&mut self) -> Self {
        Self {
            changes: mem::take(&mut self.changes),
            present_mode: self.present_mode,
            view_projection: self.view_projection,
        }
    }

    pub fn accumulate(&mut self, other: Self) {
        self.changes.accumulate(other.changes);
        self.present_mode = other.present_mode;
        self.view_projection = other.view_projection;
    }
}
