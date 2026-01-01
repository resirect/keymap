//! Physical ANSI keyboard geometry. Keys are identified by Linux evdev codes;
//! legends and binding identities come from `xkb::Resolver`, never from here.

use crate::store::FormFactor;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KeyGeom {
    pub evdev: u32,
    /// Position and size in key units (1u = one alphanumeric key).
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Clone)]
pub struct Layout {
    pub keys: Vec<KeyGeom>,
    pub width: f64,
    pub height: f64,
}

fn row(out: &mut Vec<KeyGeom>, y: f64, x0: f64, keys: &[(u32, f64)]) {
    let mut x = x0;
    for &(evdev, w) in keys {
        out.push(KeyGeom { evdev, x, y, w, h: 1.0 });
        x += w;
    }
}

fn at(out: &mut Vec<KeyGeom>, evdev: u32, x: f64, y: f64, w: f64, h: f64) {
    out.push(KeyGeom { evdev, x, y, w, h });
}

const NUMBER_ROW: [(u32, f64); 14] = [
    (41, 1.0),
    (2, 1.0),
    (3, 1.0),
    (4, 1.0),
    (5, 1.0),
    (6, 1.0),
    (7, 1.0),
    (8, 1.0),
    (9, 1.0),
    (10, 1.0),
    (11, 1.0),
    (12, 1.0),
    (13, 1.0),
    (14, 2.0),
];
const TOP_ROW: [(u32, f64); 14] = [
    (15, 1.5),
    (16, 1.0),
    (17, 1.0),
    (18, 1.0),
    (19, 1.0),
    (20, 1.0),
    (21, 1.0),
    (22, 1.0),
    (23, 1.0),
    (24, 1.0),
    (25, 1.0),
    (26, 1.0),
    (27, 1.0),
    (43, 1.5),
];
const HOME_ROW: [(u32, f64); 13] = [
    (58, 1.75),
    (30, 1.0),
    (31, 1.0),
    (32, 1.0),
    (33, 1.0),
    (34, 1.0),
    (35, 1.0),
    (36, 1.0),
    (37, 1.0),
    (38, 1.0),
    (39, 1.0),
    (40, 1.0),
    (28, 2.25),
];
const SHIFT_ROW: [(u32, f64); 11] =
    [(42, 2.25), (44, 1.0), (45, 1.0), (46, 1.0), (47, 1.0), (48, 1.0), (49, 1.0), (50, 1.0), (51, 1.0), (52, 1.0), (53, 1.0)];

/// The 15u-wide alphanumeric block starting at row `y0`.
fn main_block(out: &mut Vec<KeyGeom>, y0: f64, first_key_esc: bool, rshift: f64) {
    let mut num = NUMBER_ROW.to_vec();
    if first_key_esc {
        num[0].0 = 1; // 60%: Esc takes the grave position
    }
    row(out, y0, 0.0, &num);
    row(out, y0 + 1.0, 0.0, &TOP_ROW);
    row(out, y0 + 2.0, 0.0, &HOME_ROW);
    let mut sr = SHIFT_ROW.to_vec();
    sr.push((54, rshift));
    row(out, y0 + 3.0, 0.0, &sr);
}

fn function_row(out: &mut Vec<KeyGeom>, gap_after_esc: f64, group_gap: f64) -> f64 {
    at(out, 1, 0.0, 0.0, 1.0, 1.0);
    let mut x = 1.0 + gap_after_esc;
    let fkeys = [59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 87, 88];
    for (i, k) in fkeys.iter().enumerate() {
        if i > 0 && i % 4 == 0 {
            x += group_gap;
        }
        at(out, *k, x, 0.0, 1.0, 1.0);
        x += 1.0;
    }
    x
}

pub fn layout(ff: FormFactor) -> Layout {
    let mut k = Vec::new();
    match ff {
        FormFactor::Sixty => {
            main_block(&mut k, 0.0, true, 2.75);
            row(&mut k, 4.0, 0.0, &[(29, 1.25), (125, 1.25), (56, 1.25), (57, 6.25), (100, 1.25), (126, 1.25), (127, 1.25), (97, 1.25)]);
            Layout { keys: k, width: 15.0, height: 5.0 }
        }
        FormFactor::SeventyFive => {
            function_row(&mut k, 0.25, 0.25);
            at(&mut k, 99, 14.0, 0.0, 1.0, 1.0); // Print
            at(&mut k, 111, 15.25, 0.0, 1.0, 1.0); // Delete
            let y = 1.25;
            main_block(&mut k, y, false, 1.75);
            for (i, code) in [102u32, 104, 109].iter().enumerate() {
                at(&mut k, *code, 15.25, y + i as f64, 1.0, 1.0); // Home, PgUp, PgDn
            }
            at(&mut k, 107, 15.25, y + 3.0, 1.0, 1.0); // End
            at(&mut k, 103, 14.25, y + 3.0, 1.0, 1.0); // Up
            row(&mut k, y + 4.0, 0.0, &[(29, 1.25), (125, 1.25), (56, 1.25), (57, 6.25), (100, 1.0), (127, 1.0), (97, 1.0)]);
            row(&mut k, y + 4.0, 13.25, &[(105, 1.0), (108, 1.0), (106, 1.0)]);
            Layout { keys: k, width: 16.25, height: y + 5.0 }
        }
        FormFactor::Full => {
            function_row(&mut k, 1.0, 0.5);
            row(&mut k, 0.0, 15.25, &[(99, 1.0), (70, 1.0), (119, 1.0)]);
            let y = 1.5;
            main_block(&mut k, y, false, 2.75);
            row(
                &mut k,
                y + 4.0,
                0.0,
                &[(29, 1.25), (125, 1.25), (56, 1.25), (57, 6.25), (100, 1.25), (126, 1.25), (127, 1.25), (97, 1.25)],
            );
            row(&mut k, y, 15.25, &[(110, 1.0), (102, 1.0), (104, 1.0)]);
            row(&mut k, y + 1.0, 15.25, &[(111, 1.0), (107, 1.0), (109, 1.0)]);
            at(&mut k, 103, 16.25, y + 3.0, 1.0, 1.0);
            row(&mut k, y + 4.0, 15.25, &[(105, 1.0), (108, 1.0), (106, 1.0)]);
            let nx = 18.5;
            row(&mut k, y, nx, &[(69, 1.0), (98, 1.0), (55, 1.0), (74, 1.0)]);
            row(&mut k, y + 1.0, nx, &[(71, 1.0), (72, 1.0), (73, 1.0)]);
            at(&mut k, 78, nx + 3.0, y + 1.0, 1.0, 2.0);
            row(&mut k, y + 2.0, nx, &[(75, 1.0), (76, 1.0), (77, 1.0)]);
            row(&mut k, y + 3.0, nx, &[(79, 1.0), (80, 1.0), (81, 1.0)]);
            at(&mut k, 96, nx + 3.0, y + 3.0, 1.0, 2.0);
            row(&mut k, y + 4.0, nx, &[(82, 2.0), (83, 1.0)]);
            Layout { keys: k, width: nx + 4.0, height: y + 5.0 }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn layouts_are_consistent() {
        for (ff, count) in [(FormFactor::Sixty, 61), (FormFactor::SeventyFive, 83), (FormFactor::Full, 104)] {
            let l = layout(ff);
            assert_eq!(l.keys.len(), count, "{ff:?}");
            let codes: HashSet<u32> = l.keys.iter().map(|k| k.evdev).collect();
            assert_eq!(codes.len(), l.keys.len(), "{ff:?}: duplicate keys");
            for (i, a) in l.keys.iter().enumerate() {
                assert!(a.x + a.w <= l.width + 1e-9 && a.y + a.h <= l.height + 1e-9, "{ff:?}: key {} out of bounds", a.evdev);
                for b in &l.keys[i + 1..] {
                    let overlap = a.x < b.x + b.w - 1e-9 && b.x < a.x + a.w - 1e-9 && a.y < b.y + b.h - 1e-9 && b.y < a.y + a.h - 1e-9;
                    assert!(!overlap, "{ff:?}: {} overlaps {}", a.evdev, b.evdev);
                }
            }
        }
        // Rows of the alphanumeric block are all 15u wide.
        let l = layout(FormFactor::Full);
        for y in [1.5, 2.5, 3.5, 4.5, 5.5] {
            let w: f64 = l.keys.iter().filter(|k| k.y == y && k.x < 15.0).map(|k| k.w).sum();
            assert!((w - 15.0).abs() < 1e-9, "row {y} is {w}u");
        }
    }
}
