//! The card the bubble shows: device name, mode, the products and their
//! levels.

use podctl::model::Mode;
use tiny_skia::*;

use crate::anim::{Gauge, swap_text};
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
    pub fn of(is_dark: bool) -> Self {
        if is_dark { dark() } else { light() }
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

/// Device pixels of a `w`×`h` logical buffer at `scale`. Rounded, as the fractional
/// scale protocol sizes buffers.
pub fn buf_px(w: u32, h: u32, scale: f32) -> (u32, u32) {
    (
        (w as f32 * scale).round() as u32,
        (h as f32 * scale).round() as u32,
    )
}

/// Centre and radius of the close button, in logical buffer coordinates.
const CLOSE_X: f32 = MARGIN + CARD_W - 30.0;
const CLOSE_Y: f32 = MARGIN + 30.0;
const CLOSE_R: f32 = 11.0;
/// A little more than the drawn circle answers to the pointer.
const CLOSE_HIT_R: f32 = 14.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hit {
    Outside,
    Card,
    Close,
}

/// What lies under a point given in logical buffer coordinates.
pub fn hit(x: f32, y: f32) -> Hit {
    let (dx, dy) = (x - CLOSE_X, y - CLOSE_Y);
    if dx * dx + dy * dy <= CLOSE_HIT_R * CLOSE_HIT_R {
        Hit::Close
    } else if (MARGIN..MARGIN + CARD_W).contains(&x) && (MARGIN..MARGIN + CARD_H).contains(&y) {
        Hit::Card
    } else {
        Hit::Outside
    }
}

/// The part of the buffer that takes clicks, logical `(x, y, w, h)`.
pub const INPUT_RECT: (i32, i32, i32, i32) =
    (MARGIN as i32, MARGIN as i32, CARD_W as i32, CARD_H as i32);

/// The square around the close button that answers to the pointer,
/// logical `(x, y, side)`.
pub const CLOSE_HIT_BOX: (f32, f32, f32) = (
    CLOSE_X - CLOSE_HIT_R,
    CLOSE_Y - CLOSE_HIT_R,
    2.0 * CLOSE_HIT_R,
);

/// A rendered card, plus what lies under its close button so the
/// button alone can be drawn again on hover.
pub struct Drawn {
    pub pm: Pixmap,
    under: Pixmap,
    at: (u32, u32),
}

impl Drawn {
    /// Device-pixel origin of `close_patch` in the card.
    pub fn close_at(&self) -> (u32, u32) {
        self.at
    }

    /// The close button area, lit or not.
    pub fn close_patch(&self, pal: &Pal, hover: bool) -> Pixmap {
        let s = scale_of(&self.pm);
        set_scale(s);
        let mut pm = self.under.clone();
        let t =
            Transform::from_scale(s, s).post_translate(-(self.at.0 as f32), -(self.at.1 as f32));
        close_btn(&mut pm, pal, t, hover);
        pm
    }
}

fn scale_of(pm: &Pixmap) -> f32 {
    pm.width() as f32 / BUF_W as f32
}

pub fn render(snap: &Snapshot, pal: &Theme, scale: f32) -> Pixmap {
    render_card(snap, pal, scale).pm
}

/// The card with its shadow margin, `scale` device pixels per logical one.
pub fn render_card(snap: &Snapshot, pal: &Theme, scale: f32) -> Drawn {
    let s = scale;
    set_scale(s);
    let (bw, bh) = buf_px(BUF_W, BUF_H, s);
    let mut pm = Pixmap::new(bw, bh).expect("non-zero card");
    let (under, at) = plate(&mut pm, snap, pal, s, true);
    for part in Part::products(snap) {
        draw_part(&mut pm, snap, pal, s, part);
    }
    draw_part(&mut pm, snap, pal, s, Part::Readouts);
    Drawn { pm, under, at }
}

/// The pieces the card is drawn from, in drawing order after the plate.
/// The motion draws each into a layer of its own.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Part {
    ShadowR,
    ShadowL,
    BudR,
    BudL,
    Case { led: bool },
    MaxShadow,
    MaxBody,
    Readouts,
}

impl Part {
    pub fn products(snap: &Snapshot) -> Vec<Part> {
        if snap.art.is_max() {
            vec![Part::MaxShadow, Part::MaxBody]
        } else {
            vec![
                Part::ShadowR,
                Part::ShadowL,
                Part::BudR,
                Part::BudL,
                Part::Case {
                    led: snap.case.charging,
                },
            ]
        }
    }
}

/// The card without status line, products and readouts, for the layers
/// of the motion.
pub fn render_plate(snap: &Snapshot, pal: &Pal, s: f32) -> Pixmap {
    set_scale(s);
    let (bw, bh) = buf_px(BUF_W, BUF_H, s);
    let mut pm = Pixmap::new(bw, bh).expect("non-zero card");
    plate(&mut pm, snap, pal, s, false);
    pm
}

/// Shadow, card, edge, title, status line and close button; returns what
/// lies under the close button and where.
fn plate(
    pm: &mut Pixmap,
    snap: &Snapshot,
    pal: &Pal,
    s: f32,
    with_status: bool,
) -> (Pixmap, (u32, u32)) {
    let t = Transform::from_scale(s, s);
    let (x0, y0) = (MARGIN, MARGIN);
    let body = rrect(x0, y0, CARD_W, CARD_H, RADIUS);
    soft_shadow(pm, s, &body, 1.0, 3.0, pal.shadow * 0.5, t);
    soft_shadow(pm, s, &body, 8.0, 18.0, pal.shadow, t);
    fill(pm, &body, pal.card, t);
    let edge = rrect(x0 + 0.5, y0 + 0.5, CARD_W - 1.0, CARD_H - 1.0, RADIUS - 0.5);
    stroke(pm, &edge, pal.border, 1.0, LineCap::Butt, t);

    let title = ellipsize(&snap.name, 17.0, -0.1, TITLE_MAX_W);
    text(
        pm,
        t,
        &title,
        x0 + 20.0,
        y0 + 35.0,
        17.0,
        0.55,
        pal.text,
        -0.1,
    );
    if with_status {
        draw_status(pm, snap.status(), pal, s, 1.0, 0.0);
    }
    let r = (CLOSE_R + 1.0) * s;
    let at = (
        ((CLOSE_X * s - r).floor().max(0.0)) as u32,
        ((CLOSE_Y * s - r).floor().max(0.0)) as u32,
    );
    let side = (2.0 * r).ceil() as u32 + 1;
    let under = pm
        .clone_rect(IntRect::from_xywh(at.0 as i32, at.1 as i32, side, side).expect("non-empty"))
        .expect("inside the card");
    close_btn(pm, pal, t, false);
    (under, at)
}

/// The line under the title, `alpha` visible and `dy` below its place.
pub fn draw_status(pm: &mut Pixmap, status: &str, pal: &Pal, s: f32, alpha: f32, dy: f32) {
    if alpha <= 0.0 {
        return;
    }
    let t = Transform::from_scale(s, s);
    text(
        pm,
        t,
        status,
        MARGIN + 20.0,
        MARGIN + 53.0 + dy,
        12.5,
        0.0,
        faded(pal.sub, alpha),
        0.0,
    );
}

fn faded(c: Color, alpha: f32) -> Color {
    if alpha >= 1.0 {
        return c;
    }
    let mut c = c;
    c.apply_opacity(alpha);
    c
}

/// How many readouts the card shows, left to right.
pub fn readout_count(snap: &Snapshot) -> usize {
    if snap.art.is_max() {
        1
    } else if merged(snap.left, snap.right).is_some() {
        2
    } else {
        3
    }
}

/// The readouts, readout `i` as `gauge(i, low)` has it.
pub fn draw_readouts(
    pm: &mut Pixmap,
    snap: &Snapshot,
    pal: &Pal,
    s: f32,
    gauge: &dyn Fn(usize, bool) -> Gauge,
) {
    let low = |p: Pod| p.level.is_some_and(|l| l <= LOW_PCT) && !p.charging;
    let g = |i: usize, p: Pod| gauge(i, low(p));
    set_scale(s);
    let (x0, y0) = (MARGIN, MARGIN);
    let base = y0 + CARD_H - 18.0;
    let (ca, cb) = (x0 + 104.0, x0 + 266.0);
    if snap.art.is_max() {
        readout(
            pm,
            pal,
            s,
            x0 + CARD_W / 2.0,
            base,
            None,
            snap.left,
            g(0, snap.left),
        );
        return;
    }
    let (l, r) = (snap.left, snap.right);
    if let Some(both) = merged(l, r) {
        readout(pm, pal, s, ca, base, None, both, g(0, both));
        readout(pm, pal, s, cb, base, None, snap.case, g(1, snap.case));
    } else {
        readout(pm, pal, s, ca - 40.0, base, Some("L"), l, g(0, l));
        readout(pm, pal, s, ca + 40.0, base, Some("R"), r, g(1, r));
        readout(pm, pal, s, cb, base, None, snap.case, g(2, snap.case));
    }
}

/// Draw `part` of the card into `pm` at scale `s`.
pub fn draw_part(pm: &mut Pixmap, snap: &Snapshot, pal: &Pal, s: f32, part: Part) {
    set_scale(s);
    let (x0, y0) = (MARGIN, MARGIN);
    let (ca, cb) = (x0 + 104.0, x0 + 266.0);
    let pair = |pm: &mut Pixmap, sel: u8| {
        let lift = snap.art.bud_lift() * K_BUDS;
        draw_pair(
            pm,
            pal,
            s,
            snap.art.bud(),
            ca,
            y0 + FLOOR - lift,
            K_BUDS,
            sel,
        );
    };
    match part {
        Part::ShadowR => pair(pm, 2),
        Part::ShadowL => pair(pm, 1),
        Part::BudR => pair(pm, 8),
        Part::BudL => pair(pm, 4),
        Part::Case { led } => draw_case(pm, pal, s, &snap.art.case(), cb, y0 + FLOOR, K_BUDS, led),
        Part::MaxShadow => draw_max(pm, pal, s, x0 + CARD_W / 2.0, y0 + FLOOR, K_MAX, 1),
        Part::MaxBody => draw_max(pm, pal, s, x0 + CARD_W / 2.0, y0 + FLOOR, K_MAX, 2),
        Part::Readouts => draw_readouts(pm, snap, pal, s, &|_, _| Gauge::REST),
    }
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
fn readout(
    pm: &mut Pixmap,
    pal: &Pal,
    s: f32,
    cx: f32,
    base: f32,
    label: Option<&str>,
    p: Pod,
    g: Gauge,
) {
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
    // The row keeps the bolt's room from the start, so only the bolt
    // itself changes as it comes in.
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
        stroke(pm, &full, pal.track, g.stroke, LineCap::Butt, t);
    }
    if let Some(l) = p.level {
        let a0 = -std::f32::consts::FRAC_PI_2;
        let f = l as f32 / 100.0 * g.fill;
        if f > 0.005
            && let Some(a) = arc(rc.0, rc.1, rr, a0, a0 + f * std::f32::consts::TAU)
        {
            stroke(pm, &a, ring_color(pal, p), g.stroke, LineCap::Round, t);
        }
    }
    x += ring_d + 6.0;
    let tc = match p.level {
        Some(l) if l <= LOW_PCT && !p.charging => pal.red,
        Some(_) => pal.text,
        None => pal.sub,
    };
    if g.text >= 1.0 || p.level.is_none() {
        text(pm, t, &pct, x, base, pct_px, 0.55, tc, 0.0);
    } else {
        let (dash, _) = swap_text(g.text, false);
        text(pm, t, "—", x, base, pct_px, 0.55, faded(pal.sub, dash), 0.0);
        let (num, dy) = swap_text(g.text, true);
        if num > 0.0 {
            text(pm, t, &pct, x, base + dy, pct_px, 0.55, faded(tc, num), 0.0);
        }
    }
    x += pct_w;
    if p.charging && g.bolt > 0.0 {
        fill(
            pm,
            &bolt(x + 5.5, base - 4.6, 10.0),
            faded(pal.green, g.bolt),
            t,
        );
    }
}

fn close_btn(pm: &mut Pixmap, pal: &Pal, t: Transform, hover: bool) {
    let (cx, cy) = (CLOSE_X, CLOSE_Y);
    let (face, mark) = if hover {
        let face = if pal.dark {
            hex(0x4A4A4F, 255)
        } else {
            hex(0xDCDCE1, 255)
        };
        (face, mix(pal.sub, pal.text, 0.5))
    } else {
        (pal.fill3, pal.sub)
    };
    fill(pm, &oval(cx, cy, CLOSE_R, CLOSE_R), face, t);
    let d = 3.6;
    let mut pb = PathBuilder::new();
    pb.move_to(cx - d, cy - d);
    pb.line_to(cx + d, cy + d);
    pb.move_to(cx + d, cy - d);
    pb.line_to(cx - d, cy + d);
    if let Some(x) = pb.finish() {
        stroke(pm, &x, mark, 1.7, LineCap::Round, t);
    }
}

fn mix(a: Color, b: Color, k: f32) -> Color {
    let l = |x: f32, y: f32| x + (y - x) * k;
    Color::from_rgba(
        l(a.red(), b.red()),
        l(a.green(), b.green()),
        l(a.blue(), b.blue()),
        l(a.alpha(), b.alpha()),
    )
    .unwrap_or(b)
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
    fn hits_the_card_and_its_close_button() {
        assert_eq!(hit(CLOSE_X, CLOSE_Y), Hit::Close);
        assert_eq!(hit(CLOSE_X + 13.0, CLOSE_Y), Hit::Close);
        assert_eq!(hit(MARGIN + 20.0, MARGIN + 20.0), Hit::Card);
        assert_eq!(hit(MARGIN - 1.0, MARGIN + 20.0), Hit::Outside);
        assert_eq!(hit(MARGIN + CARD_W, MARGIN + 20.0), Hit::Outside);
        assert_eq!(hit(10.0, 10.0), Hit::Outside);
    }

    #[test]
    fn an_unlit_close_patch_is_the_card_itself() {
        for scale in [1.0, 1.25, 1.75, 2.0] {
            for pal in [dark(), light()] {
                let d = render_card(&Snapshot::sample(), &pal, scale);
                let patch = d.close_patch(&pal, false);
                let (x0, y0) = d.close_at();
                let w = d.pm.width() as usize;
                let row = patch.width() as usize * 4;
                for (y, line) in patch.data().chunks_exact(row).enumerate() {
                    let at = ((y0 as usize + y) * w + x0 as usize) * 4;
                    assert_eq!(line, &d.pm.data()[at..at + row], "scale {scale}, row {y}");
                }
                assert_ne!(d.close_patch(&pal, true).data(), patch.data());
            }
        }
    }

    #[test]
    fn buffer_sizes_round_like_the_compositor() {
        assert_eq!(buf_px(BUF_W, BUF_H, 1.0), (408, 232));
        assert_eq!(buf_px(BUF_W, BUF_H, 1.25), (510, 290));
        assert_eq!(buf_px(BUF_W, BUF_H, 1.5), (612, 348));
        assert_eq!(buf_px(BUF_W, BUF_H, 1.75), (714, 406));
        assert_eq!(buf_px(BUF_W, BUF_H, 2.0), (816, 464));
        let pm = render(&Snapshot::sample(), &dark(), 1.75);
        assert_eq!((pm.width(), pm.height()), (714, 406));
    }

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
