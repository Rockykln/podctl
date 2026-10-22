//! The tray's own icons: the pair of buds over a level bar, written out
//! at startup and handed to the host through `IconThemePath`.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The levels there is an icon for; a reading shows as the nearest.
const STEPS: [u8; 6] = [10, 20, 40, 60, 80, 100];
/// At or below this the bar turns red.
const RED_UP_TO: u8 = 20;

const INDEX: &str = "\
[Icon Theme]
Name=Hicolor
Directories=scalable/status

[scalable/status]
Size=16
MinSize=8
MaxSize=512
Type=Scalable
Context=Status
";

/// How the pair and the bar are drawn on the 16 px grid.
struct Shape {
    head_r: f64,
    head_y: f64,
    /// Centre of each head, and the left edge of its stem.
    buds: [(f64, f64); 2],
    stem_w: f64,
    stem_h: f64,
    bar_x: f64,
    bar_y: f64,
    bar_w: f64,
    bar_h: f64,
    /// The bar while charging, shortened to make room for the bolt.
    bar_w_charging: f64,
    /// Heads and stems as shapes of their own. In one path their
    /// opposite windings cut a hole where they overlap.
    solid: bool,
}

/// For panels that draw the icon at 22 px and up, like Plasma's.
const FINE: Shape = Shape {
    head_r: 2.9,
    head_y: 3.6,
    buds: [(4.2, 5.0), (11.8, 9.0)],
    stem_w: 2.0,
    stem_h: 7.4,
    bar_x: 1.0,
    bar_y: 12.5,
    bar_w: 14.0,
    bar_h: 3.0,
    bar_w_charging: 10.5,
    solid: false,
};

/// GNOME draws every tray icon at 16 px next to its solid system icons,
/// where the fine lines all but vanish.
const BOLD: Shape = Shape {
    head_r: 3.4,
    head_y: 3.9,
    buds: [(4.0, 4.6), (12.0, 8.6)],
    stem_w: 2.8,
    stem_h: 7.3,
    bar_x: 0.5,
    bar_y: 12.0,
    bar_w: 15.0,
    bar_h: 4.0,
    bar_w_charging: 11.0,
    solid: true,
};

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
    let shape = if podctl::xdg::on_gnome() {
        &BOLD
    } else {
        &FINE
    };
    let path = install(&dir, shape)?;
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
fn install(dir: &Path, shape: &Shape) -> std::io::Result<PathBuf> {
    let status = dir.join("hicolor/scalable/status");
    std::fs::create_dir_all(&status)?;
    // GTK and St only look into directories a theme index names; Plasma
    // finds the files without it.
    std::fs::write(dir.join("hicolor/index.theme"), INDEX)?;
    let mut all = vec![
        (PLAIN.to_string(), svg(shape, None, false, false)),
        (DISCONNECTED.to_string(), svg(shape, None, false, true)),
    ];
    for step in STEPS {
        for charging in [false, true] {
            all.push((
                level(step, charging),
                svg(shape, Some(step), charging, false),
            ));
        }
    }
    for (name, body) in all {
        std::fs::write(status.join(format!("{name}.svg")), body)?;
    }
    Ok(dir.to_path_buf())
}

fn svg(s: &Shape, level: Option<u8>, charging: bool, faded: bool) -> String {
    let mut buds = Vec::new();
    for (cx, sx) in s.buds {
        buds.push(circle(cx, s.head_y, s.head_r));
        buds.push(rounded(sx, s.head_y, s.stem_w, s.stem_h, s.stem_w / 2.0));
    }
    let mut paths = if s.solid {
        buds.iter()
            .map(|d| path("ColorScheme-Text", d, None))
            .collect()
    } else {
        vec![path("ColorScheme-Text", &buds.concat(), None)]
    };
    if level.is_some() || faded {
        // Charging shortens the bar to make room for the bolt beside it.
        let width = if charging { s.bar_w_charging } else { s.bar_w };
        let bar = |w: f64| rounded(s.bar_x, s.bar_y, w, s.bar_h, s.bar_h / 2.0);
        paths.push(path("ColorScheme-Text", &bar(width), Some(0.3)));
        if let Some(pct) = level {
            let class = if pct <= RED_UP_TO && !charging {
                "ColorScheme-NegativeText error"
            } else {
                "ColorScheme-PositiveText success"
            };
            paths.push(path(class, &bar(width * f64::from(pct) / 100.0), None));
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
    fn the_bold_heads_are_shapes_of_their_own() {
        let body = svg(&BOLD, Some(100), false, false);
        // Two heads, two stems, the bar's track and its fill.
        assert_eq!(body.matches("<path").count(), 6, "{body}");
        assert_eq!(
            svg(&FINE, Some(100), false, false).matches("<path").count(),
            3
        );
    }

    #[test]
    fn a_reading_shows_as_the_nearest_step() {
        assert_eq!(level(3, false), "podctl-airpods-10-symbolic");
        assert_eq!(level(31, false), "podctl-airpods-40-symbolic");
        assert_eq!(level(95, true), "podctl-airpods-100-charging-symbolic");
    }

    #[test]
    fn every_name_handed_out_is_written() {
        for (i, shape) in [&FINE, &BOLD].into_iter().enumerate() {
            let dir = std::env::temp_dir().join(format!("podctl-icons-{}-{i}", std::process::id()));
            install(&dir, shape).unwrap();
            check_written(&dir);
        }
    }

    fn check_written(dir: &Path) {
        assert!(
            std::fs::read_to_string(dir.join("hicolor/index.theme"))
                .unwrap()
                .contains("scalable/status")
        );
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
