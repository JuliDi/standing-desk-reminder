//! Procedurally drawn tray icons (no image assets, no image crate).
//!
//! Renders a simple up/down arrow whose colour encodes the current phase:
//! green up-arrow while standing, blue down-arrow while sitting, grey when paused.

use ksni::Icon;

use crate::config::Phase;

/// Sizes hosts can choose from; the panel picks whichever fits best.
const SIZES: [i32; 3] = [22, 32, 48];

type Rgb = (u8, u8, u8);

const GREEN: Rgb = (0x2E, 0xCC, 0x71);
const BLUE: Rgb = (0x34, 0x98, 0xDB);
const GREY: Rgb = (0x9E, 0x9E, 0x9E);

/// Build the icon set for the current phase / paused state.
pub fn render(phase: Phase, paused: bool) -> Vec<Icon> {
    let color = if paused {
        GREY
    } else {
        match phase {
            Phase::Standing => GREEN,
            Phase::Sitting => BLUE,
        }
    };
    let point_up = matches!(phase, Phase::Standing);
    SIZES.iter().map(|&size| arrow(size, point_up, color)).collect()
}

fn arrow(size: i32, point_up: bool, color: Rgb) -> Icon {
    let n = size as usize;
    let mut data = vec![0u8; n * n * 4];

    // Arrow geometry - matching original proportions
    let cx = n as f32 / 2.0;
    let pad = n as f32 * 0.16;
    
    // Arrow head: triangle from pad to base_y
    let apex_y = pad;
    let base_y = n as f32 * 0.55;
    let head_half_max = n as f32 * 0.30;
    
    // Arrow shaft: rectangle from shaft_top to bottom-pad
    let shaft_top = n as f32 * 0.50;
    let shaft_bottom = n as f32 - pad;
    let shaft_half = n as f32 * 0.12;
    
    // Anti-aliasing
    let blur = n as f32 * 0.015;

    for y in 0..n {
        // actual_y is the y-coordinate in the arrow's natural orientation (up)
        let actual_y = if point_up {
            y as f32
        } else {
            n as f32 - 1.0 - y as f32  // flip for down arrow
        };

        for x in 0..n {
            let fx = x as f32 + 0.5;
            let dx = (fx - cx).abs();

            // Distance to arrow shape (negative = inside)
            let dist = if actual_y >= apex_y && actual_y <= base_y {
                // Arrow head: a point at the apex (top) widening to the base (bottom)
                let head_y = actual_y - apex_y;
                let head_half_width = head_half_max * (head_y / (base_y - apex_y));
                dx - head_half_width
            } else if actual_y >= shaft_top && actual_y <= shaft_bottom {
                // Arrow shaft: rectangle
                dx - shaft_half
            } else {
                // Outside arrow
                f32::INFINITY
            };

            // Smooth alpha based on distance (anti-aliasing)
            let alpha = if dist <= -blur {
                255u8
            } else if dist <= blur {
                ((blur - dist) / (2.0 * blur) * 255.0).clamp(0.0, 255.0) as u8
            } else {
                0u8
            };

            if alpha > 0 {
                let i = (y * n + x) * 4;
                data[i] = alpha;
                data[i + 1] = color.0;
                data[i + 2] = color.1;
                data[i + 3] = color.2;
            }
        }
    }

    Icon {
        width: size,
        height: size,
        data,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_expected_sizes_and_buffer_lengths() {
        let icons = render(Phase::Standing, false);
        assert_eq!(icons.len(), SIZES.len());
        for icon in &icons {
            assert_eq!(icon.data.len(), (icon.width * icon.height * 4) as usize);
        }
    }

    #[test]
    fn arrow_has_some_opaque_and_some_transparent_pixels() {
        let icon = arrow(32, true, GREEN);
        let opaque = icon.data.chunks_exact(4).filter(|p| p[0] > 200).count();
        let clear = icon.data.chunks_exact(4).filter(|p| p[0] == 0).count();
        assert!(opaque > 0, "arrow should have opaque pixels");
        assert!(clear > 0, "arrow should have transparent background");
    }

    #[test]
    fn up_arrow_points_up() {
        // The topmost opaque row should be narrow (the point); a lower row
        // through the head should be wider.
        let n = 32usize;
        let icon = arrow(n as i32, true, GREEN);
        let width_at = |y: usize| {
            (0..n)
                .filter(|&x| icon.data[(y * n + x) * 4] > 180)
                .count()
        };
        let top = (0..n).find(|&y| width_at(y) > 0).unwrap();
        assert!(
            width_at(top + 4) > width_at(top),
            "head should widen downward from its apex (up-arrow)"
        );
    }
}
