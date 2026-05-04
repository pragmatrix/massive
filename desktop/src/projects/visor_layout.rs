use std::f64::consts::PI;

#[derive(Debug, Clone, Copy)]
pub struct VisorPlacement {
    pub center_x_offset: f64,
    pub center_z: f64,
    pub yaw: f64,
}

const MAX_ARC_RADIANS: f64 = PI / 2.0;
const ARC_SOFTNESS_INSTANCE_COUNT: f64 = 6.0;
const MIN_RADIUS: f64 = 1.0;
pub const FORWARD_Z: f64 = 128.0;

pub fn placement(
    index: usize,
    instance_count: usize,
    flat_span: f64,
    focused_index: Option<usize>,
    expansion_factor: f64,
) -> Option<VisorPlacement> {
    if instance_count <= 1 {
        return None;
    }

    let regular_arc = effective_arc(instance_count);
    let radius = radius_for_center_span(flat_span, regular_arc);

    let focused_rotation = focused_index
        .map(|focused| base_angle(focused, instance_count, regular_arc))
        .unwrap_or(0.0);

    let regular_angle = base_angle(index, instance_count, regular_arc) - focused_rotation;
    let angle = regular_angle * expansion_factor;

    Some(VisorPlacement {
        center_x_offset: radius * angle.sin(),
        center_z: FORWARD_Z + radius * (1.0 - angle.cos()),
        yaw: -angle,
    })
}

fn effective_arc(instance_count: usize) -> f64 {
    let softness = ARC_SOFTNESS_INSTANCE_COUNT.max(1.0);
    let count = instance_count as f64;
    let normalized = 1.0 - (-count / softness).exp();
    MAX_ARC_RADIANS * normalized
}

fn radius_for_center_span(center_span: f64, arc: f64) -> f64 {
    let span = center_span.max(1e-6);
    let arc = arc.max(1e-6);
    (span / arc).max(MIN_RADIUS)
}

fn base_angle(index: usize, instance_count: usize, arc: f64) -> f64 {
    if instance_count <= 1 {
        return 0.0;
    }

    let t = index as f64 / (instance_count.saturating_sub(1)) as f64;
    -arc * 0.5 + arc * t
}
