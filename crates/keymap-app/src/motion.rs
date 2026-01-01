//! The opening ripple's timing (keys appear from the keyboard's centre
//! outward) and the session marker that picks its speed. No GTK here, so it
//! can be tested directly.

use std::path::{Path, PathBuf};

/// Whole ripple on the first visible opening of a desktop session (~1.47 s).
pub const FIRST_OPEN_SECS: f64 = 2.2 / 1.5;
/// Every later opening in the same session: exactly twice as fast.
pub const LATER_OPEN_SECS: f64 = FIRST_OPEN_SECS / 2.0;

/// Each key's own fade lasts 7/11 of the ripple...
pub fn key_secs(total: f64) -> f64 {
    total * 7.0 / 11.0
}

/// ...and the outermost key starts 4/11 in, so the last key finishes at `total`.
pub fn max_stagger_secs(total: f64) -> f64 {
    total * 4.0 / 11.0
}

/// Normalized distance `[0, 1]` of each key centre from the keyboard centre.
/// Horizontal distance is measured in half-widths and vertical in whole
/// heights, which keeps the spread outward rather than top-to-bottom.
pub fn distances(centers: &[(f64, f64)], width: f64, height: f64) -> Vec<f64> {
    if width <= 0.0 || height <= 0.0 {
        return vec![0.0; centers.len()];
    }
    let raw: Vec<f64> = centers
        .iter()
        .map(|&(x, y)| {
            let dx = (x - width / 2.0) / (width / 2.0);
            let dy = (y - height / 2.0) / height;
            (dx * dx + dy * dy).sqrt()
        })
        .collect();
    let max = raw.iter().copied().fold(0.0, f64::max);
    if max <= 0.0 { vec![0.0; raw.len()] } else { raw.into_iter().map(|d| d / max).collect() }
}

/// `cubic-bezier(0.16, 1, 0.3, 1)`: a smooth ease-out with no overshoot.
pub fn ease_out(p: f64) -> f64 {
    const X1: f64 = 0.16;
    const Y1: f64 = 1.0;
    const X2: f64 = 0.3;
    const Y2: f64 = 1.0;
    if p <= 0.0 {
        return 0.0;
    }
    if p >= 1.0 {
        return 1.0;
    }
    let bez = |a: f64, b: f64, s: f64| 3.0 * a * s * (1.0 - s).powi(2) + 3.0 * b * s * s * (1.0 - s) + s.powi(3);
    // x(s) is monotonic for these control points: bisect for s with x(s) = p.
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..40 {
        let mid = (lo + hi) / 2.0;
        if bez(X1, X2, mid) < p { lo = mid } else { hi = mid }
    }
    bez(Y1, Y2, (lo + hi) / 2.0)
}

/// Opacity of a key at `t` seconds into a ripple of `total` seconds, for a
/// key at normalized distance `d`.
pub fn key_opacity(t: f64, d: f64, total: f64) -> f64 {
    let dur = key_secs(total);
    let p = if dur > 0.0 { (t - d * max_stagger_secs(total)) / dur } else { 1.0 };
    ease_out(p)
}

/// Where the first-opening markers live: per user, cleared at logout/reboot.
pub fn marker_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty()).map(|d| PathBuf::from(d).join("keymap"))
}

/// Identifies the running Niri session by its IPC socket file, which each
/// Niri instance creates anew: a new desktop session gets a new key, while
/// restarting Keymap inside the same session keeps it.
pub fn session_key() -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    let sock = std::env::var_os("NIRI_SOCKET")?;
    let m = std::fs::metadata(&sock).ok()?;
    Some(format!("{}-{}-{}", m.dev(), m.ino(), m.ctime_nsec() as i64 + m.ctime() * 1_000_000_000))
}

/// Records the first visible opening of session `key`, returning whether this
/// call was it. Creating the marker is atomic, so of two racing instances only
/// one gets `true`. Markers of earlier sessions are removed.
pub fn claim_first_open(dir: &Path, key: &str) -> bool {
    let name = format!("opened-{key}");
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let created = std::fs::OpenOptions::new().write(true).create_new(true).open(dir.join(&name)).is_ok();
    if created && let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let n = e.file_name();
            let n = n.to_string_lossy();
            if n.starts_with("opened-") && n != name {
                std::fs::remove_file(e.path()).ok();
            }
        }
    }
    created
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_match_the_chosen_preview() {
        assert!((FIRST_OPEN_SECS - 1.4667).abs() < 1e-3);
        assert!((LATER_OPEN_SECS * 2.0 - FIRST_OPEN_SECS).abs() < 1e-12);
        let t = FIRST_OPEN_SECS;
        assert!((key_secs(t) + max_stagger_secs(t) - t).abs() < 1e-12);
        assert!((key_secs(t) - 0.9333).abs() < 1e-3 && (max_stagger_secs(t) - 0.5333).abs() < 1e-3);
    }

    #[test]
    fn easing_is_monotonic_without_overshoot() {
        assert_eq!(ease_out(0.0), 0.0);
        assert!((ease_out(1.0) - 1.0).abs() < 1e-9);
        let mut prev = 0.0;
        for i in 1..=100 {
            let v = ease_out(i as f64 / 100.0);
            assert!(v >= prev - 1e-12 && v <= 1.0 + 1e-9, "step {i}: {v}");
            prev = v;
        }
        // Ease-out: most of the change happens early.
        assert!(ease_out(0.3) > 0.7);
    }

    #[test]
    fn centre_keys_lead_and_the_last_key_finishes_on_time() {
        let d = distances(&[(50.0, 10.0), (0.0, 0.0), (100.0, 20.0), (75.0, 10.0)], 100.0, 20.0);
        assert_eq!(d[0], 0.0);
        assert_eq!(d[1], 1.0);
        assert_eq!(d[2], 1.0);
        assert!(d[3] > 0.0 && d[3] < 1.0);
        let t = FIRST_OPEN_SECS;
        assert_eq!(key_opacity(0.0, 0.0, t), 0.0);
        assert_eq!(key_opacity(max_stagger_secs(t), 1.0, t), 0.0);
        assert!(key_opacity(max_stagger_secs(t), 0.0, t) > 0.9);
        assert!((key_opacity(t, 1.0, t) - 1.0).abs() < 1e-9);
        // Long after the end, keys stay exactly at rest.
        assert_eq!(key_opacity(10.0, 0.5, t), 1.0);
    }

    #[test]
    fn empty_or_degenerate_layouts_are_safe() {
        assert!(distances(&[], 100.0, 20.0).is_empty());
        assert_eq!(distances(&[(1.0, 1.0)], 0.0, 0.0), vec![0.0]);
        assert_eq!(distances(&[(50.0, 10.0)], 100.0, 20.0), vec![0.0]);
        assert_eq!(key_opacity(0.0, 0.0, 0.0), 1.0);
    }

    #[test]
    fn first_open_is_claimed_once_per_session() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("keymap");
        assert!(claim_first_open(&dir, "a"));
        // Later openings, a restarted or a racing second instance: fast.
        assert!(!claim_first_open(&dir, "a"));
        assert!(!claim_first_open(&dir, "a"));
        // A new session starts slow again and clears the old marker.
        assert!(claim_first_open(&dir, "b"));
        assert!(!dir.join("opened-a").exists());
        assert!(!claim_first_open(&dir, "b"));
    }

    #[test]
    fn unusable_marker_dir_never_claims() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("not-a-dir");
        std::fs::write(&file, "").unwrap();
        assert!(!claim_first_open(&file.join("keymap"), "a"));
    }
}
