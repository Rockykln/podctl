//! The card the bubble shows: device name, mode, the products and their
//! levels.

use podctl::model::Mode;
use tiny_skia::*;

use crate::art::{Art, draw_case, draw_max, draw_pair};
use crate::paint::*;

/// Logical size of the card, and the shadow margin drawn around it.
pub const CARD_W: f32 = 360.0;
pub const CARD_H: f32 = 184.0;
const RADIUS: f32 = 26.0;
pub const MARGIN: f32 = 24.0;
/// The buffer a backend gets at scale 1: card plus shadow margin.
pub const BUF_W: u32 = (CARD_W + 2.0 * MARGIN) as u32;
pub const BUF_H: u32 = (CARD_H + 2.0 * MARGIN) as u32;

/// Logical y the products stand on, and px per mm for buds and case
/// (one scale, so they keep their true proportions) and for the Max.
const FLOOR: f32 = 141.0;
const K_BUDS: f32 = 1.5;
const K_MAX: f32 = 0.45;
const TITLE_MAX_W: f32 = 290.0;

/// One threshold for "low": the ring and the number turn red at or
/// below it, and the bubble pops up once on the way down.
pub const LOW_PCT: u8 = 20;

pub type Theme = Pal;

impl Pal {
    pub fn by_name(name: &str) -> Self {
        if name.eq_ignore_ascii_case("light") {
            light()
        } else {
            dark()
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct Pod {
    pub level: Option<u8>,
    pub charging: bool,
}

#[derive(Clone)]
pub struct Snapshot {
    /// What the user called the buds; the model label when they did not.
    pub name: String,
    pub art: Art,
    pub connected: bool,
    pub mode: Option<Mode>,
    pub left: Pod,
    pub right: Pod,
    pub case: Pod,
}

impl Snapshot {
    /// Any earbud or the case at or below the low-battery threshold.
    pub fn low(&self) -> bool {
        [self.left, self.right, self.case]
            .iter()
            .any(|p| p.level.is_some_and(|l| l <= LOW_PCT))
    }

    /// No component has a level yet — the rings would all read "—".
    pub fn blank(&self) -> bool {
        [self.left, self.right, self.case]
            .iter()
            .all(|p| p.level.is_none())
    }

    /// The line under the name: the listening mode, nothing else.
    pub fn status(&self) -> &'static str {
        if self.blank() {
            return "Connecting…";
        }
        self.mode.map(mode_label).unwrap_or("Connected")
    }

    pub fn sample() -> Self {
        Self::sample_for(Art::Pro2C)
    }

    /// The states the reference images were drawn with.
    #[rustfmt::skip]
    pub fn sample_for(art: Art) -> Self {
        use Art::*;
        use Mode::*;
        let pod = |l, c| Pod { level: Some(l), charging: c };
        let none = Pod::default();
        let (name, mode, left, right, case) = match art {
            Gen1 => ("AirPods (1st generation)", None, pod(72, false), pod(70, false), pod(48, false)),
            Gen2 => ("AirPods (2nd generation)", None, pod(100, true), pod(97, true), pod(55, true)),
            Gen3 => ("AirPods (3rd generation)", None, pod(12, false), pod(64, false), pod(38, false)),
            Gen4 => ("AirPods 4", None, pod(88, false), pod(86, false), pod(71, true)),
            Gen4Anc => ("AirPods 4 (ANC)", Some(NoiseCancellation), pod(91, false), pod(90, false), pod(42, false)),
            Gen5 => ("AirPods 5", Some(NoiseCancellation), pod(84, false), pod(83, false), pod(67, true)),
            Gen5W => ("AirPods 5 (Wireless Charging)", Some(Adaptive), pod(77, false), pod(75, false), pod(52, true)),
            Pro1 => ("AirPods Pro", Some(Transparency), pod(54, false), pod(62, false), pod(80, false)),
            Pro2L => ("AirPods Pro 2 (Lightning)", Some(Adaptive), pod(82, false), pod(79, false), pod(64, true)),
            Pro2C => ("AirPods Pro 2 (USB-C)", Some(NoiseCancellation), pod(82, false), pod(79, false), pod(64, true)),
            Pro3 => ("AirPods Pro 3", Some(NoiseCancellation), pod(95, false), pod(93, false), pod(58, false)),
            MaxL => ("AirPods Max", Some(Transparency), pod(74, false), none, none),
            MaxC => ("AirPods Max (USB-C)", Some(NoiseCancellation), pod(41, true), none, none),
            Max2 => ("AirPods Max 2", Some(Adaptive), pod(88, false), none, none),
        };
        Snapshot { name: name.into(), art, connected: true, mode, left, right, case }
    }
}

pub fn mode_label(m: Mode) -> &'static str {
    match m {
        Mode::Off => "Off",
        Mode::NoiseCancellation => "Noise Cancellation",
        Mode::Transparency => "Transparency",
        Mode::Adaptive => "Adaptive Audio",
    }
}

/// The card with its shadow margin, `scale` device pixels per logical one.
pub fn render(snap: &Snapshot, pal: &Theme, scale: f32) -> Pixmap {
    let s = scale;
    set_scale(s);
    let bw = ((CARD_W + 2.0 * MARGIN) * s).ceil() as u32;
    let bh = ((CARD_H + 2.0 * MARGIN) * s).ceil() as u32;
    let mut pm = Pixmap::new(bw, bh).expect("non-zero card");
    let t = Transform::from_scale(s, s);
    let (x0, y0) = (MARGIN, MARGIN);
    let body = rrect(x0, y0, CARD_W, CARD_H, RADIUS);
    soft_shadow(&mut pm, s, &body, 1.0, 3.0, pal.shadow * 0.5, t);
    soft_shadow(&mut pm, s, &body, 8.0, 18.0, pal.shadow, t);
    fill(&mut pm, &body, pal.card, t);
    let edge = rrect(x0 + 0.5, y0 + 0.5, CARD_W - 1.0, CARD_H - 1.0, RADIUS - 0.5);
    stroke(&mut pm, &edge, pal.border, 1.0, LineCap::Butt, t);

    let title = ellipsize(&snap.name, 17.0, -0.1, TITLE_MAX_W);
    text(
        &mut pm,
        t,
        &title,
        x0 + 20.0,
        y0 + 35.0,
        17.0,
        0.55,
        pal.text,
        -0.1,
    );
    text(
        &mut pm,
        t,
        snap.status(),
        x0 + 20.0,
        y0 + 53.0,
        12.5,
        0.0,
        pal.sub,
        0.0,
    );
    close_btn(&mut pm, pal, s, x0 + CARD_W - 30.0, y0 + 30.0);

    let base = y0 + CARD_H - 18.0;
    if snap.art.is_max() {
        draw_max(&mut pm, pal, s, x0 + CARD_W / 2.0, y0 + FLOOR, K_MAX);
        readout(&mut pm, pal, s, x0 + CARD_W / 2.0, base, None, snap.left);
        return pm;
    }
    let ca = x0 + 104.0;
    let cb = x0 + 266.0;
    let lift = snap.art.bud_lift() * K_BUDS;
    draw_pair(&mut pm, pal, s, snap.art.bud(), ca, y0 + FLOOR - lift, K_BUDS, 15);
    draw_case(
        &mut pm,
        pal,
        s,
        &snap.art.case(),
        cb,
        y0 + FLOOR,
        K_BUDS,
        snap.case.charging,
    );

    let (l, r) = (snap.left, snap.right);
    if let Some(both) = merged(l, r) {
        readout(&mut pm, pal, s, ca, base, None, both);
    } else {
        readout(&mut pm, pal, s, ca - 40.0, base, Some("L"), l);
        readout(&mut pm, pal, s, ca + 40.0, base, Some("R"), r);
    }
    readout(&mut pm, pal, s, cb, base, None, snap.case);
    pm
}

/// One value for both buds when they are within 5 points and charge
/// alike; the lower one is shown.
fn merged(l: Pod, r: Pod) -> Option<Pod> {
    match (l.level, r.level) {
        (Some(a), Some(b)) if a.abs_diff(b) <= 5 && l.charging == r.charging => Some(Pod {
            level: Some(a.min(b)),
            charging: l.charging,
        }),
        (None, None) => Some(Pod {
            level: None,
            charging: l.charging,
        }),
        _ => None,
    }
}

fn ellipsize(s: &str, px: f32, track: f32, max_w: f32) -> String {
    if text_w(s, px, track) <= max_w {
        return s.to_string();
    }
    let mut chars: Vec<char> = s.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let cut: String = chars.iter().collect::<String>().trim_end().to_string() + "…";
        if text_w(&cut, px, track) <= max_w {
            return cut;
        }
    }
    "…".into()
}

fn ring_color(pal: &Pal, p: Pod) -> Color {
    match p.level {
        _ if p.charging => pal.green,
        Some(l) if l <= LOW_PCT => pal.red,
        _ => pal.ring,
    }
}

fn bolt(cx: f32, cy: f32, h: f32) -> Path {
    let w = h * 0.62;
    let (x, y) = (cx - w / 2.0, cy - h / 2.0);
    let mut pb = PathBuilder::new();
    pb.move_to(x + w * 0.62, y);
    pb.line_to(x, y + h * 0.58);
    pb.line_to(x + w * 0.45, y + h * 0.58);
    pb.line_to(x + w * 0.36, y + h);
    pb.line_to(x + w, y + h * 0.40);
    pb.line_to(x + w * 0.55, y + h * 0.40);
    pb.close();
    pb.finish().expect("closed bolt")
}

/// `[label] (ring) 82% [bolt]`, centred on `cx` at baseline `base`.
#[allow(clippy::too_many_arguments)]
fn readout(pm: &mut Pixmap, pal: &Pal, s: f32, cx: f32, base: f32, label: Option<&str>, p: Pod) {
    let t = Transform::from_scale(s, s);
    let ring_d = 15.0;
    let pct = match p.level {
        Some(l) => format!("{l}%"),
        None => "—".into(),
    };
    let pct_px = 13.0;
    let pct_w = text_w(&pct, pct_px, 0.0);
    let lab_px = 11.0;
    let lab_w = label.map(|l| text_w(l, lab_px, 0.0) + 5.0).unwrap_or(0.0);
    let bolt_w = if p.charging { 9.0 } else { 0.0 };
    let total = lab_w + ring_d + 6.0 + pct_w + bolt_w;
    let mut x = cx - total / 2.0;
    if let Some(l) = label {
        text(pm, t, l, x, base, lab_px, 0.5, pal.sub, 0.0);
        x += lab_w;
    }
    let rc = (x + ring_d / 2.0, base - 4.6);
    let rr = ring_d / 2.0 - 1.3;
    if let Some(full) = arc(rc.0, rc.1, rr, 0.0, std::f32::consts::TAU) {
        stroke(pm, &full, pal.track, 2.6, LineCap::Butt, t);
    }
    if let Some(l) = p.level {
        let a0 = -std::f32::consts::FRAC_PI_2;
        let f = l as f32 / 100.0;
        if f > 0.005
            && let Some(a) = arc(rc.0, rc.1, rr, a0, a0 + f * std::f32::consts::TAU)
        {
            stroke(pm, &a, ring_color(pal, p), 2.6, LineCap::Round, t);
        }
    }
    x += ring_d + 6.0;
    let tc = match p.level {
        Some(l) if l <= LOW_PCT && !p.charging => pal.red,
        Some(_) => pal.text,
        None => pal.sub,
    };
    text(pm, t, &pct, x, base, pct_px, 0.55, tc, 0.0);
    x += pct_w;
    if p.charging {
        fill(pm, &bolt(x + 5.5, base - 4.6, 10.0), pal.green, t);
    }
}

fn close_btn(pm: &mut Pixmap, pal: &Pal, s: f32, cx: f32, cy: f32) {
    let t = Transform::from_scale(s, s);
    fill(pm, &oval(cx, cy, 11.0, 11.0), pal.fill3, t);
    let d = 3.6;
    let mut pb = PathBuilder::new();
    pb.move_to(cx - d, cy - d);
    pb.line_to(cx + d, cy + d);
    pb.move_to(cx + d, cy - d);
    pb.line_to(cx - d, cy + d);
    if let Some(x) = pb.finish() {
        stroke(pm, &x, pal.sub, 1.7, LineCap::Round, t);
    }
}

/// The soft gradient the reference images put behind the card.
pub fn on_backdrop(card: &Pixmap, dark: bool) -> Pixmap {
    let (w, h) = (card.width(), card.height());
    let mut bg = Pixmap::new(w, h).expect("non-zero card");
    let (a, b) = if dark {
        (hex(0x2B3442, 255), hex(0x151A22, 255))
    } else {
        (hex(0xC9D6E3, 255), hex(0xE9E4DA, 255))
    };
    if let Some(r) = Rect::from_xywh(0.0, 0.0, w as f32, h as f32) {
        let p = Paint {
            shader: lin((0.0, 0.0), (w as f32 * 0.6, h as f32), a, b),
            ..Default::default()
        };
        bg.fill_rect(r, &p, Transform::identity(), None);
    }
    bg.draw_pixmap(
        0,
        0,
        card.as_ref(),
        &PixmapPaint::default(),
        Transform::identity(),
        None,
    );
    bg
}

pub fn to_bgra_premul(pm: &Pixmap) -> Vec<u8> {
    let mut out = pm.data().to_vec();
    for px in out.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_levels_show_as_one() {
        let p = |l| Pod {
            level: Some(l),
            charging: false,
        };
        assert_eq!(merged(p(82), p(79)).and_then(|m| m.level), Some(79));
        assert_eq!(merged(p(82), p(70)), None);
        let charging = Pod {
            level: Some(80),
            charging: true,
        };
        assert_eq!(merged(p(80), charging), None, "one charges, one does not");
        assert_eq!(merged(p(80), Pod::default()), None);
    }

    #[test]
    fn a_long_name_is_cut_with_an_ellipsis() {
        let long = "AirPods Pro (2nd generation, USB-C) of somebody with a long name";
        let cut = ellipsize(long, 17.0, -0.1, TITLE_MAX_W);
        assert!(cut.ends_with('…'));
        assert!(text_w(&cut, 17.0, -0.1) <= TITLE_MAX_W);
        assert_eq!(
            ellipsize("AirPods Pro", 17.0, -0.1, TITLE_MAX_W),
            "AirPods Pro"
        );
    }

    #[test]
    fn the_subtitle_is_the_mode_or_connecting() {
        let mut s = Snapshot::sample();
        assert_eq!(s.status(), "Noise Cancellation");
        s.mode = None;
        assert_eq!(s.status(), "Connected");
        s.left = Pod::default();
        s.right = Pod::default();
        s.case = Pod::default();
        assert_eq!(s.status(), "Connecting…");
    }

    #[test]
    fn low_is_at_or_below_the_threshold() {
        let mut s = Snapshot::sample();
        assert!(!s.low());
        s.right.level = Some(LOW_PCT + 1);
        assert!(!s.low());
        s.right.level = Some(LOW_PCT);
        assert!(s.low());
    }
}
