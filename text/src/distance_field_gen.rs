// Ported from m115.

//
//  Copyright 2014 Google Inc.
//
//  Use of this source code is governed by a BSD-style license that can be
//  found in the LICENSE file.

use crate::point::Point;
use bitflags::bitflags;
use std::{f32, mem, slice};

// the max magnitude for the distance field
// distance values are limited to the range (-SK_DistanceFieldMagnitude, SK_DistanceFieldMagnitude]
const DISTANCE_FIELD_MAGNITUDE: usize = 4;

// we need to pad around the original glyph to allow our maximum distance of
// SK_DistanceFieldMagnitude texels away from any edge
pub(crate) const DISTANCE_FIELD_PAD: usize = 4;

#[derive(Debug)]
struct DFData {
    alpha: f32,         // alpha value of source texel
    dist_sq: f32,       // distance squared to nearest (so far) edge texel
    dist_vector: Point, // distance vector to nearest (so far) edge texel
}

bitflags! {
    struct NeighborFlags: u8 {
        const LEFT = 0x01;
        const RIGHT = 0x02;
        const TOP_LEFT = 0x04;
        const TOP = 0x08;
        const TOP_RIGHT = 0x10;
        const BOTTOM_LEFT = 0x20;
        const BOTTOM = 0x40;
        const BOTTOM_RIGHT = 0x80;
        const ALL = 0xff;
    }
}

// We treat an "edge" as a place where we cross from >=128 to <128, or vice versa, or
// where we have two non-zero pixels that are <128.
// 'neighborFlags' is used to limit the directions in which we test to avoid indexing
// outside of the image
unsafe fn found_edge(image_ptr: *const u8, width: usize, neighbor_flags: NeighborFlags) -> bool {
    const NUM_8_CONNECTED_NEIGHBORS: usize = 8;
    let offsets: [isize; NUM_8_CONNECTED_NEIGHBORS] = [
        -1,
        1,
        -(width as isize) - 1,
        -(width as isize),
        -(width as isize) + 1,
        width as isize - 1,
        width as isize,
        width as isize + 1,
    ];
    debug_assert_eq!(
        NUM_8_CONNECTED_NEIGHBORS,
        NeighborFlags::ALL.bits().count_ones() as usize
    );

    let curr_val = *image_ptr;
    let curr_check = curr_val >> 7;
    for i in 0..NUM_8_CONNECTED_NEIGHBORS {
        if (1 << i) & neighbor_flags.bits() == 0 {
            continue;
        }
        let check_ptr = image_ptr.offset(offsets[i]);
        let neighbor_val = *check_ptr;
        let neighbor_check = neighbor_val >> 7;
        debug_assert!(curr_check == 0 || curr_check == 1);
        debug_assert!(neighbor_check == 0 || neighbor_check == 1);
        if curr_check != neighbor_check
            || (curr_check == 0 && neighbor_check == 0 && curr_val != 0 && neighbor_val != 0)
        {
            return true;
        }
    }

    false
}

#[allow(clippy::too_many_arguments)]
fn init_glyph_data(
    data: &mut [DFData],
    edges: &mut [u8],
    image: &[u8],
    data_width: usize,
    _data_height: usize,
    image_width: usize,
    image_height: usize,
    pad: usize,
) {
    let mut data = &mut data[pad * data_width + pad..];
    let mut edges = &mut edges[pad * data_width + pad..];
    let mut image = image.as_ptr();
    for j in 0..image_height {
        for i in 0..image_width {
            if 255 == unsafe { *image } {
                data[0].alpha = 1.0;
            } else {
                #[allow(clippy::excessive_precision)]
                {
                    data[0].alpha = unsafe { *image } as f32 * 0.00392156862;
                }
                // 1/255
            }
            let mut check_mask = NeighborFlags::ALL;
            if i == 0 {
                check_mask &=
                    !(NeighborFlags::LEFT | NeighborFlags::TOP | NeighborFlags::BOTTOM_LEFT);
            }
            if i == image_width - 1 {
                check_mask &= !(NeighborFlags::RIGHT
                    | NeighborFlags::TOP_RIGHT
                    | NeighborFlags::BOTTOM_RIGHT);
            }
            if j == 0 {
                check_mask &=
                    !(NeighborFlags::TOP_LEFT | NeighborFlags::TOP | NeighborFlags::TOP_RIGHT);
            }
            if j == image_height - 1 {
                check_mask &= !(NeighborFlags::BOTTOM_LEFT
                    | NeighborFlags::BOTTOM
                    | NeighborFlags::BOTTOM_RIGHT);
            }
            if unsafe { found_edge(image, image_width, check_mask) } {
                edges[0] = 255; // using 255 makes for convenient debug rendering
            }
            data = &mut data[1..];
            image = unsafe { image.add(1) };
            edges = &mut edges[1..];
        }
        data = &mut data[2 * pad..];
        edges = &mut edges[2 * pad..];
    }
}

// from Gustavson (2011)
// computes the distance to an edge given an edge normal vector and a pixel's alpha value
// assumes that direction has been pre-normalized
fn edge_distance(direction: Point, alpha: f32) -> f32 {
    let dx = direction.x;
    let dy = direction.y;
    let distance;
    if dx.abs() < f32::EPSILON || dy.abs() < f32::EPSILON {
        distance = 0.5 - alpha;
    } else {
        // this is easier if we treat the direction as being in the first octant
        // (other octants are symmetrical)
        let (dx, dy) = if dx < dy { (dy, dx) } else { (dx, dy) };

        // a1 = 0.5*dy/dx is the smaller fractional area chopped off by the edge
        // to avoid the divide, we just consider the numerator
        let a1num = 0.5 * dy;

        // we now compute the approximate distance, depending where the alpha falls
        // relative to the edge fractional area

        // if 0 <= alpha < a1
        if alpha * dx < a1num {
            // TODO: find a way to do this without square roots?
            distance = 0.5 * (dx + dy) - (2.0 * dx * dy * alpha).sqrt();
        // if a1 <= alpha <= 1 - a1
        } else if alpha * dx < dx - a1num {
            distance = (0.5 - alpha) * dx;
        // if 1 - a1 < alpha <= 1
        } else {
            // TODO: find a way to do this without square roots?
            distance = -0.5 * (dx + dy) + (2.0 * dx * dy * (1.0 - alpha)).sqrt();
        }
    }

    distance
}

unsafe fn init_distances(data: *mut DFData, mut edges: *const u8, width: usize, height: usize) {
    // skip one pixel border
    let mut curr_data = data;
    let mut prev_data = data.sub(width);
    let mut next_data = data.add(width);

    for j in 0..height {
        for i in 0..width {
            if *edges != 0 {
                // we should not be in the one-pixel outside band
                debug_assert!(i > 0 && i < width - 1 && j > 0 && j < height - 1);
                // gradient will point from low to high
                // +y is down in this case
                // i.e., if you're outside, gradient points towards edge
                // if you're inside, gradient points away from edge
                let mut curr_grad = Point {
                    x: (*prev_data.offset(1)).alpha - (*prev_data.offset(-1)).alpha
                        + (*curr_data.offset(1)).alpha * f32::consts::SQRT_2
                        - (*curr_data.offset(-1)).alpha * f32::consts::SQRT_2
                        + (*next_data.offset(1)).alpha
                        - (*next_data.offset(-1)).alpha,
                    y: (*next_data.offset(-1)).alpha - (*prev_data.offset(-1)).alpha
                        + (*next_data).alpha * f32::consts::SQRT_2
                        - (*prev_data).alpha * f32::consts::SQRT_2
                        + (*next_data.offset(1)).alpha
                        - (*prev_data.offset(1)).alpha,
                };
                curr_grad.set_length_fast(1.0);

                // init squared distance to edge and distance vector
                let dist = edge_distance(curr_grad, (*curr_data).alpha);
                curr_grad.x *= dist;
                curr_grad.y *= dist;
                (*curr_data).dist_vector = curr_grad;
                (*curr_data).dist_sq = dist * dist;
            } else {
                // init distance to "far away"
                (*curr_data).dist_sq = 2000000.0;
                (*curr_data).dist_vector = Point {
                    x: 1000.0,
                    y: 1000.0,
                };
            }
            curr_data = curr_data.offset(1);
            prev_data = prev_data.offset(1);
            next_data = next_data.offset(1);
            edges = edges.offset(1);
        }
    }
}

// Danielsson's 8SSEDT

// first stage forward pass
// (forward in Y, forward in X)
unsafe fn f1(curr: *mut DFData, width: usize) {
    // upper left
    let check = curr.offset(-(width as isize) - 1);
    let mut dist_vec = (*check).dist_vector;
    let dist_sq = (*check).dist_sq - 2.0 * (dist_vec.x + dist_vec.y - 1.0);
    if dist_sq < (*curr).dist_sq {
        dist_vec.x -= 1.0;
        dist_vec.y -= 1.0;
        (*curr).dist_sq = dist_sq;
        (*curr).dist_vector = dist_vec;
    }

    // up
    let check = curr.offset(-(width as isize));
    let mut dist_vec = (*check).dist_vector;
    let dist_sq = (*check).dist_sq - 2.0 * dist_vec.y + 1.0;
    if dist_sq < (*curr).dist_sq {
        dist_vec.y -= 1.0;
        (*curr).dist_sq = dist_sq;
        (*curr).dist_vector = dist_vec;
    }

    // upper right
    let check = curr.offset(-(width as isize) + 1);
    let mut dist_vec = (*check).dist_vector;
    let dist_sq = (*check).dist_sq + 2.0 * (dist_vec.x - dist_vec.y + 1.0);
    if dist_sq < (*curr).dist_sq {
        dist_vec.x += 1.0;
        dist_vec.y -= 1.0;
        (*curr).dist_sq = dist_sq;
        (*curr).dist_vector = dist_vec;
    }

    // left
    let check = curr.offset(-1);
    let mut dist_vec = (*check).dist_vector;
    let dist_sq = (*check).dist_sq - 2.0 * dist_vec.x + 1.0;
    if dist_sq < (*curr).dist_sq {
        dist_vec.x -= 1.0;
        (*curr).dist_sq = dist_sq;
        (*curr).dist_vector = dist_vec;
    }
}

// second stage forward pass
// (forward in Y, backward in X)
unsafe fn f2(curr: *mut DFData, _width: usize) {
    // right
    let check = curr.offset(1);
    let mut dist_vec = (*check).dist_vector;
    let dist_sq = (*check).dist_sq + 2.0 * dist_vec.x + 1.0;
    if dist_sq < (*curr).dist_sq {
        dist_vec.x += 1.0;
        (*curr).dist_sq = dist_sq;
        (*curr).dist_vector = dist_vec;
    }
}

// first stage backward pass
// (backward in Y, forward in X)
unsafe fn b1(curr: *mut DFData, _width: usize) {
    // left
    let check = curr.offset(-1);
    let mut dist_vec = (*check).dist_vector;
    let dist_sq = (*check).dist_sq - 2.0 * dist_vec.x + 1.0;
    if dist_sq < (*curr).dist_sq {
        dist_vec.x -= 1.0;
        (*curr).dist_sq = dist_sq;
        (*curr).dist_vector = dist_vec;
    }
}

// second stage backward pass
// (backward in Y, backwards in X)
unsafe fn b2(curr: *mut DFData, width: usize) {
    // right
    let mut check = curr.add(1);
    let mut dist_vec = (*check).dist_vector;
    let mut dist_sq = (*check).dist_sq + 2.0 * dist_vec.x + 1.0;
    if dist_sq < (*curr).dist_sq {
        dist_vec.x += 1.0;
        (*curr).dist_sq = dist_sq;
        (*curr).dist_vector = dist_vec;
    }

    // bottom left
    check = curr.add(width - 1);
    dist_vec = (*check).dist_vector;
    dist_sq = (*check).dist_sq - 2.0 * (dist_vec.x - dist_vec.y - 1.0);
    if dist_sq < (*curr).dist_sq {
        dist_vec.x -= 1.0;
        dist_vec.y += 1.0;
        (*curr).dist_sq = dist_sq;
        (*curr).dist_vector = dist_vec;
    }

    // bottom
    check = curr.add(width);
    dist_vec = (*check).dist_vector;
    dist_sq = (*check).dist_sq + 2.0 * dist_vec.y + 1.0;
    if dist_sq < (*curr).dist_sq {
        dist_vec.y += 1.0;
        (*curr).dist_sq = dist_sq;
        (*curr).dist_vector = dist_vec;
    }

    // bottom right
    check = curr.add(width + 1);
    dist_vec = (*check).dist_vector;
    dist_sq = (*check).dist_sq + 2.0 * (dist_vec.x + dist_vec.y + 1.0);
    if dist_sq < (*curr).dist_sq {
        dist_vec.x += 1.0;
        dist_vec.y += 1.0;
        (*curr).dist_sq = dist_sq;
        (*curr).dist_vector = dist_vec;
    }
}

/// Return x pinned (clamped) between lo and hi, inclusively.
/// Unlike std::clamp(), SkTPin() always returns a value between lo and hi.
/// If x is NaN, SkTPin() returns lo but std::clamp() returns NaN.
fn pin(x: f32, lo: f32, hi: f32) -> f32 {
    lo.max(x.min(hi))
}

fn pack_distance_field_val<const DISTANCE_MAGNITUDE: usize>(dist: f32) -> u8 {
    let distance_magnitude = DISTANCE_MAGNITUDE as f32;

    // The distance field is constructed as unsigned char values, so that the zero value is at 128,
    // Beside 128, we have 128 values in range [0, 128), but only 127 values in range (128, 255].
    // So we multiply distanceMagnitude by 127/128 at the latter range to avoid overflow.

    let dist = pin(
        -dist,
        -distance_magnitude,
        distance_magnitude * 127.0 / 128.0,
    );

    // Scale into the positive range for unsigned distance.
    let dist = dist + distance_magnitude;

    // Scale into unsigned char range.
    // Round to place negative and positive values as equally as possible around 128
    // (which represents zero).
    (dist / (2.0 * distance_magnitude) * 256.0).round() as u8
}

// assumes a padded 8-bit image and distance field
// width and height are the original width and height of the image
pub(crate) unsafe fn generate_distance_field_from_image(
    distance_field: &mut [u8],
    copy_ptr: &[u8],
    width: usize,
    height: usize,
) -> bool {
    // we expand our temp data by one more on each side to simplify
    // the scanning code -- will always be treated as infinitely far away
    let pad = DISTANCE_FIELD_PAD + 1;

    debug_assert!(
        distance_field.len()
            == (width + 2 * DISTANCE_FIELD_PAD) * (height + 2 * DISTANCE_FIELD_PAD)
    );
    debug_assert!(copy_ptr.len() == (width + 2) * (height + 2));

    // set params for distance field data
    let data_width = width + 2 * pad;
    let data_height = height + 2 * pad;

    // create zeroed temp DFData+edge storage
    let storage = vec![0u8; data_width * data_height * (mem::size_of::<DFData>() + 1)];

    {
        let data_slice =
            slice::from_raw_parts_mut(storage.as_ptr() as *mut DFData, data_width * data_height);
        let edge_slice = slice::from_raw_parts_mut(
            (storage.as_ptr() as *mut DFData).add(data_width * data_height) as *mut u8,
            data_width * data_height,
        );

        // copy glyph into distance field storage
        init_glyph_data(
            data_slice,
            edge_slice,
            copy_ptr,
            data_width,
            data_height,
            width + 2,
            height + 2,
            DISTANCE_FIELD_PAD,
        );
    }

    let data_ptr = storage.as_ptr() as *mut DFData;
    let edge_ptr = data_ptr.add(data_width * data_height) as *mut u8;

    // create initial distance data, particularly at edges
    init_distances(data_ptr, edge_ptr, data_width, data_height);

    // now perform Euclidean distance transform to propagate distances

    // forwards in y
    let mut curr_data = data_ptr.add(data_width + 1); // skip outer buffer
    let mut curr_edge = edge_ptr.add(data_width + 1);
    for _j in 1..data_height - 1 {
        // forwards in x
        for _i in 1..data_width - 1 {
            // don't need to calculate distance for edge pixels
            if *curr_edge == 0 {
                f1(curr_data, data_width);
            }
            curr_data = curr_data.add(1);
            curr_edge = curr_edge.add(1);
        }

        // backwards in x
        curr_data = curr_data.sub(1); // reset to end
        curr_edge = curr_edge.sub(1);
        for _i in 1..data_width - 1 {
            // don't need to calculate distance for edge pixels
            if *curr_edge == 0 {
                f2(curr_data, data_width);
            }
            curr_data = curr_data.sub(1);
            curr_edge = curr_edge.sub(1);
        }

        curr_data = curr_data.add(data_width + 1);
        curr_edge = curr_edge.add(data_width + 1);
    }

    // backwards in y
    curr_data = data_ptr.add(data_width * (data_height - 2) - 1); // skip outer buffer
    curr_edge = edge_ptr.add(data_width * (data_height - 2) - 1);
    for _j in 1..data_height - 1 {
        // forwards in x
        for _i in 1..data_width - 1 {
            // don't need to calculate distance for edge pixels
            if *curr_edge == 0 {
                b1(curr_data, data_width);
            }
            curr_data = curr_data.add(1);
            curr_edge = curr_edge.add(1);
        }

        // backwards in x
        curr_data = curr_data.offset(-1); // reset to end
        curr_edge = curr_edge.offset(-1);
        for _i in 1..data_width - 1 {
            // don't need to calculate distance for edge pixels
            if *curr_edge == 0 {
                b2(curr_data, data_width);
            }
            curr_data = curr_data.sub(1);
            curr_edge = curr_edge.sub(1);
        }

        curr_data = curr_data.sub(data_width - 1);
        curr_edge = curr_edge.sub(data_width - 1);
    }

    // copy results to final distance field data
    curr_data = data_ptr.add(data_width + 1);
    curr_edge = edge_ptr.add(data_width + 1);
    let mut df_ptr = distance_field.as_mut_ptr();
    for _j in 1..data_height - 1 {
        for _i in 1..data_width - 1 {
            #[cfg(target_feature = "dump_edge")]
            {
                let alpha = curr_data.fAlpha;
                let edge = if *curr_edge != 0 { 0.25 } else { 0.0 };
                // blend with original image
                let result = alpha + (1.0 - alpha) * edge;
                let val = (255.0 * result).round() as u8;
                unsafe { *df_ptr = val };
                df_ptr = unsafe { df_ptr.add(1) };
            }
            #[cfg(not(target_feature = "dump_edge"))]
            {
                let dist = if (*curr_data).alpha > 0.5 {
                    -(*curr_data).dist_sq.sqrt()
                } else {
                    (*curr_data).dist_sq.sqrt()
                };
                *df_ptr = pack_distance_field_val::<DISTANCE_FIELD_MAGNITUDE>(dist);
                df_ptr = df_ptr.add(1);
            }
            curr_data = curr_data.add(1);
            curr_edge = curr_edge.add(1);
        }
        curr_data = curr_data.add(2);
        curr_edge = curr_edge.add(2);
    }

    true
}
