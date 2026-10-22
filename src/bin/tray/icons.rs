//! The tray's own icons: the pair of buds over a level bar, written out
//! at startup and handed to the host through `IconThemePath`.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The levels there is an icon for; a reading shows as the nearest.
const STEPS: [u8; 6] = [10, 20, 40, 60, 80, 100];
/// At or below this the bar turns red.
const RED_UP_TO: u8 = 20;

pub const PLAIN: &str = "podctl-airpods-symbolic";
pub const DISCONNECTED: &str = "podctl-airpods-disconnected-symbolic";

static THEME: OnceLock<PathBuf> = OnceLock::new();

/// Where the icons went, once [`setup`] managed to write them.
pub fn theme_path() -> Option<&'static Path> {
    THEME.get().map(PathBuf::as_path)
}

/// Write the icons for this session. systemd hands the unit its own
/// runtime directory; started by hand, the tray makes one.
pub fn setup() -> std::io::Result<()> {
    let dir = match std::env::var_os("RUNTIME_DIRECTORY") {
        Some(d) => PathBuf::from(d),
        None => podctl::xdg::runtime_home()
            .ok_or_else(|| std::io::Error::other("XDG_RUNTIME_DIR is not set"))?
            .join("podctl-tray"),
    };
    let path = install(&dir)?;
    let _ = THEME.set(path);
    Ok(())
}

pub fn level(pct: u8, charging: bool) -> String {
    let step = STEPS
        .into_iter()
        .min_by_key(|&s| s.abs_diff(pct))
        .unwrap_or(100);
    if charging {
        format!("podctl-airpods-{step}-charging-symbolic")
    } else {
        format!("podctl-airpods-{step}-symbolic")
    }
}

/// Write every icon below `dir` and return the path for
/// `IconThemePath`. Hosts look the names up in a `hicolor` theme there.
pub fn install(dir: &Path) -> std::io::Result<PathBuf> {
    let status = dir.join("hicolor/scalable/status");
    std::fs::create_dir_all(&status)?;
    let mut all = vec![
        (PLAIN.to_string(), svg(None, false, false)),
        (DISCONNECTED.to_string(), svg(None, false, true)),
    ];
    for step in STEPS {
        for charging in [false, true] {
            all.push((level(step, charging), svg(Some(step), charging, false)));
        }
    }
    for (name, body) in all {
        std::fs::write(status.join(format!("{name}.svg")), body)?;
    }
    Ok(dir.to_path_buf())
}

fn svg(level: Option<u8>, charging: bool, faded: bool) -> String {
    let mut buds = String::new();
    for (cx, sx) in [(4.2, 5.0), (11.8, 9.0)] {
        buds += &circle(cx, 3.6, 2.9);
        buds += &rounded(sx, 3.6, 2.0, 7.4, 1.0);
    }
    let mut paths = vec![path("ColorScheme-Text", &buds, None)];
    if level.is_some() || faded {
        // Charging shortens the bar to make room for the bolt beside it.
        let width = if charging { 10.5 } else { 14.0 };
        paths.push(path(
            "ColorScheme-Text",
            &rounded(1.0, 12.5, width, 3.0, 1.5),
            Some(0.3),
        ));
        if let Some(pct) = level {
            let class = if pct <= RED_UP_TO && !charging {
                "ColorScheme-NegativeText error"
            } else {
                "ColorScheme-PositiveText success"
            };
            let fill = rounded(1.0, 12.5, width * f64::from(pct) / 100.0, 3.0, 1.5);
            paths.push(path(class, &fill, None));
        }
        if charging {
            paths.push(path(
                "ColorScheme-Text",
                "M14.8 11.2L12.3 14.1H13.7L13.1 16L15.6 12.9H14.2Z",
                None,
            ));
        }
    }
    let opacity = if faded { r#" opacity="0.45""# } else { "" };
    format!(
        concat!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16">"#,
            r#"<style id="current-color-scheme" type="text/css">"#,
            ".ColorScheme-Text{{color:#232629}}.ColorScheme-PositiveText{{color:#27ae60}}",
            ".ColorScheme-NegativeText{{color:#da4453}}</style>",
            "<g{}>{}</g></svg>\n"
        ),
        opacity,
        paths.concat()
    )
}

fn path(class: &str, d: &str, opacity: Option<f64>) -> String {
    let opacity = opacity
        .map(|o| format!(r#" opacity="{o}""#))
        .unwrap_or_default();
    format!(r#"<path class="{class}" fill="currentColor"{opacity} d="{d}"/>"#)
}

fn circle(cx: f64, cy: f64, r: f64) -> String {
    let arc = format!("A{} {} 0 1 0", n(r), n(r));
    format!(
        "M{} {cy}{arc} {} {cy}{arc} {} {cy}Z",
        n(cx - r),
        n(cx + r),
        n(cx - r),
        cy = n(cy)
    )
}

fn rounded(x: f64, y: f64, w: f64, h: f64, r: f64) -> String {
    let r = r.min(w / 2.0).min(h / 2.0);
    let arc = format!("A{} {} 0 0 1", n(r), n(r));
    let (l, t, right, bottom) = (n(x + r), n(y + r), n(x + w - r), n(y + h - r));
    format!(
        "M{l} {}H{right}{arc} {} {t}V{bottom}{arc} {right} {}H{l}{arc} {} {bottom}V{t}{arc} {l} {}Z",
        n(y),
        n(x + w),
        n(y + h),
        n(x),
        n(y),
    )
}

/// A coordinate with at most two decimals, and none it does not need.
fn n(v: f64) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reading_shows_as_the_nearest_step() {
        assert_eq!(level(3, false), "podctl-airpods-10-symbolic");
        assert_eq!(level(31, false), "podctl-airpods-40-symbolic");
        assert_eq!(level(95, true), "podctl-airpods-100-charging-symbolic");
    }

    #[test]
    fn every_name_handed_out_is_written() {
        let dir = std::env::temp_dir().join(format!("podctl-icons-{}", std::process::id()));
        install(&dir).unwrap();
        let status = dir.join("hicolor/scalable/status");
        for name in STEPS
            .iter()
            .flat_map(|&s| [level(s, false), level(s, true)])
            .chain([PLAIN.into(), DISCONNECTED.into()])
        {
            let body = std::fs::read_to_string(status.join(format!("{name}.svg"))).unwrap();
            assert!(body.starts_with("<svg") && body.contains("ColorScheme-Text"));
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
