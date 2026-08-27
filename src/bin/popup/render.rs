use std::sync::OnceLock;

use fontdue::{Font, FontSettings};
use tiny_skia::{
    Color, FillRule, GradientStop, LineCap, LinearGradient, Paint, PathBuilder, Pixmap, Point,
    RadialGradient, Shader, SpreadMode, Stroke, Transform,
};

use podctl::model::Mode;

use crate::assets;

pub const CARD_W: u32 = 360;
pub const CARD_H: u32 = 180;

const PAD: f32 = 15.0;
const RADIUS: f32 = 22.0;

const DONUT_R: f32 = 20.0;
const DONUT_THICK: f32 = 5.0;
const ART_W: f32 = 128.0;
const MARGIN_R: f32 = 16.0;
const MARGIN_B: f32 = 14.0;

#[derive(Clone, Copy, Default)]
pub struct Pod {
    pub level: Option<u8>,
    pub charging: bool,
}

#[derive(Clone)]
pub struct Snapshot {
    pub model: String,
    pub connected: bool,
    pub mode: Option<Mode>,
    pub left: Pod,
    pub right: Pod,
    pub case: Pod,
}

pub const LOW_PCT: u8 = 25;

impl Snapshot {
    /// Any earbud or the case at or below the low-battery threshold.
    pub fn low(&self) -> bool {
        [self.left, self.right, self.case]
            .iter()
            .any(|p| p.level.is_some_and(|l| l < LOW_PCT))
    }

    /// No component has a level yet — the rings would all read "—".
    pub fn blank(&self) -> bool {
        [self.left, self.right, self.case]
            .iter()
            .all(|p| p.level.is_none())
    }

    pub fn sample() -> Self {
        Snapshot {
            model: "AirPods Pro 2".into(),
            connected: true,
            mode: Some(Mode::NoiseCancellation),
            left: Pod {
                level: Some(82),
                charging: false,
            },
            right: Pod {
                level: Some(79),
                charging: false,
            },
            case: Pod {
                level: Some(64),
                charging: true,
            },
        }
    }
}

fn mode_label(m: Mode) -> &'static str {
    match m {
        Mode::Off => "Off",
        Mode::NoiseCancellation => "Noise Cancellation",
        Mode::Transparency => "Transparency",
        Mode::Adaptive => "Adaptive",
    }
}

#[derive(Clone, Copy)]
pub struct Theme {
    card_top: Color,
    card_bot: Color,
    highlight: Color,
    shadow: Color,
    text: Color,
    sub: Color,
    track: Color,
    good: Color,
    warn: Color,
    bad: Color,
    accent: Color,
    glyph: Color,
    glyph_hi: Color,
    glyph_lo: Color,
    glyph_shade: Color,
    glyph_dark: Color,
    spec: Color,
    dot_on: Color,
    dot_off: Color,
}

impl Theme {
    pub fn dark() -> Self {
        Theme {
            card_top: Color::from_rgba8(46, 46, 52, 238),
            card_bot: Color::from_rgba8(26, 26, 31, 238),
            highlight: Color::from_rgba8(255, 255, 255, 24),
            shadow: Color::from_rgba8(0, 0, 0, 16),
            text: Color::from_rgba8(245, 245, 247, 255),
            sub: Color::from_rgba8(164, 164, 172, 255),
            track: Color::from_rgba8(66, 66, 74, 255),
            good: Color::from_rgba8(52, 199, 108, 255),
            warn: Color::from_rgba8(255, 179, 64, 255),
            bad: Color::from_rgba8(255, 86, 72, 255),
            accent: Color::from_rgba8(250, 192, 70, 255),
            glyph: Color::from_rgba8(228, 228, 234, 255),
            glyph_hi: Color::from_rgba8(255, 255, 255, 255),
            glyph_lo: Color::from_rgba8(188, 188, 198, 255),
            glyph_shade: Color::from_rgba8(184, 184, 194, 255),
            glyph_dark: Color::from_rgba8(140, 140, 151, 255),
            spec: Color::from_rgba8(255, 255, 255, 125),
            dot_on: Color::from_rgba8(52, 199, 108, 255),
            dot_off: Color::from_rgba8(120, 120, 128, 255),
        }
    }

    pub fn light() -> Self {
        Theme {
            card_top: Color::from_rgba8(253, 253, 255, 245),
            card_bot: Color::from_rgba8(236, 236, 241, 245),
            highlight: Color::from_rgba8(0, 0, 0, 14),
            shadow: Color::from_rgba8(0, 0, 0, 12),
            text: Color::from_rgba8(28, 28, 32, 255),
            sub: Color::from_rgba8(110, 110, 120, 255),
            track: Color::from_rgba8(208, 208, 214, 255),
            good: Color::from_rgba8(40, 178, 92, 255),
            warn: Color::from_rgba8(214, 148, 28, 255),
            bad: Color::from_rgba8(222, 64, 55, 255),
            accent: Color::from_rgba8(208, 148, 28, 255),
            glyph: Color::from_rgba8(120, 120, 130, 255),
            glyph_hi: Color::from_rgba8(176, 176, 188, 255),
            glyph_lo: Color::from_rgba8(88, 88, 98, 255),
            glyph_shade: Color::from_rgba8(90, 90, 100, 255),
            glyph_dark: Color::from_rgba8(60, 60, 70, 255),
            spec: Color::from_rgba8(255, 255, 255, 150),
            dot_on: Color::from_rgba8(40, 178, 92, 255),
            dot_off: Color::from_rgba8(176, 176, 184, 255),
        }
    }

    pub fn by_name(name: &str) -> Self {
        if name.eq_ignore_ascii_case("light") {
            Self::light()
        } else {
            Self::dark()
        }
    }

    fn ramp(&self, pod: Pod) -> Color {
        match pod.level {
            _ if pod.charging => self.accent,
            None => self.sub,
            Some(l) if l >= 50 => self.good,
            Some(l) if l >= 20 => self.warn,
            Some(_) => self.bad,
        }
    }
}

fn ui_font() -> &'static Font {
    static F: OnceLock<Font> = OnceLock::new();
    F.get_or_init(|| {
        Font::from_bytes(assets::UI_FONT, FontSettings::default()).expect("bundled font parses")
    })
}

pub fn render(snap: &Snapshot, theme: &Theme) -> Pixmap {
    let mut pm = Pixmap::new(CARD_W, CARD_H).expect("pixmap alloc");
    let font = ui_font();

    let x0 = PAD;
    let y0 = PAD;
    let cw = CARD_W as f32 - 2.0 * PAD;
    let ch = CARD_H as f32 - 2.0 * PAD;

    drop_shadow(&mut pm, x0, y0, cw, ch, theme);
    card(&mut pm, x0, y0, cw, ch, theme);

    earbuds(&mut pm, x0, y0, ch, theme);

    let tx = x0 + ART_W + 8.0;
    let text_right = x0 + cw - MARGIN_R;
    let (label, title) = fit(font, &snap.model, text_right - tx, 19.0, 12.0);
    text(
        &mut pm,
        font,
        &label,
        tx,
        y0 + 42.0,
        &Ink::semibold(title, theme.text),
    );

    let (status, dot) = if snap.connected {
        (
            snap.mode.map(mode_label).unwrap_or("Connected"),
            theme.dot_on,
        )
    } else {
        ("Disconnected", theme.dot_off)
    };
    disc(&mut pm, tx + 4.0, y0 + 62.0, 3.5, dot);
    text(
        &mut pm,
        font,
        status,
        tx + 14.0,
        y0 + 66.0,
        &Ink::new(12.5, theme.sub),
    );

    // Donut row, derived from the card box so it can never overflow:
    // label baseline sits MARGIN_B above the bottom; the ring is stacked
    // above it; columns are spread between the art column and the right
    // margin.
    let outer = DONUT_R + DONUT_THICK * 0.5;
    let label_base = y0 + ch - MARGIN_B;
    let cy = label_base - 12.0 - DONUT_R;
    let xr = text_right - outer;
    let xl = x0 + ART_W + 14.0 + outer;
    let step = (xr - xl) * 0.5;
    donut(&mut pm, font, theme, xl, cy, snap.left, "Left");
    donut(&mut pm, font, theme, xl + step, cy, snap.right, "Right");
    donut(&mut pm, font, theme, xr, cy, snap.case, "Case");

    if snap.low() {
        let ring = rounded_rect(x0, y0, cw, ch, RADIUS);
        let mut p = Paint {
            anti_alias: true,
            ..Default::default()
        };
        p.set_color(theme.bad);
        pm.stroke_path(
            &ring,
            &p,
            &Stroke {
                width: 2.5,
                ..Default::default()
            },
            Transform::identity(),
            None,
        );
    }

    pm
}

fn earbuds(pm: &mut Pixmap, x0: f32, y0: f32, ch: f32, theme: &Theme) {
    let s = 62.0;
    let bud_w = s * 0.64;
    let gap = s * 0.30;
    let ox = x0 + 24.0;
    let oy = y0 + (ch - s * 0.98) * 0.5;
    let e = assets::earbud(s, ox, oy);
    let axis = ox + bud_w + gap * 0.5;
    let mirror = Transform::from_row(-1.0, 0.0, 0.0, 1.0, 2.0 * axis, 0.0);
    let flip = |xa: f32| 2.0 * axis - xa;

    // Soft contact shadows first, so both earbuds sit on the card.
    let sy = oy + s * 0.99 + 3.0;
    contact(pm, ox + 0.40 * s, sy, 0.30 * s, theme);
    contact(pm, flip(ox + 0.40 * s), sy, 0.30 * s, theme);

    draw_earbud(pm, theme, &e, Transform::identity(), ox + 0.32 * s, oy, s);
    draw_earbud(pm, theme, &e, mirror, flip(ox + 0.32 * s), oy, s);
}

fn contact(pm: &mut Pixmap, cx: f32, cy: f32, rx: f32, theme: &Theme) {
    for k in (1..=3).rev() {
        let g = k as f32;
        let mut pb = PathBuilder::new();
        pb.push_oval(
            tiny_skia::Rect::from_xywh(cx - rx - g, cy - 4.0 - g, 2.0 * (rx + g), 8.0 + 2.0 * g)
                .unwrap(),
        );
        if let Some(p) = pb.finish() {
            fill(pm, &p, theme.shadow);
        }
    }
}

/// Render one earbud with 3D shading. `bcx` is the bud centre x in
/// pixmap space; `t` carries the mirror for the right earbud while the
/// gradients are built in pixmap space so the light stays consistent.
fn draw_earbud(
    pm: &mut Pixmap,
    theme: &Theme,
    e: &assets::Earbud,
    t: Transform,
    bcx: f32,
    oy: f32,
    s: f32,
) {
    let bcy = oy + 0.31 * s;
    let br = 0.34 * s;

    // Body: a glossy sphere lit from the upper left.
    // 0.12 RadialGradient takes (start, start_radius, end, end_radius, ...);
    // start_radius = 0 keeps the classic single-point focal behaviour.
    let body = RadialGradient::new(
        Point::from_xy(bcx - 0.36 * br, bcy - 0.42 * br),
        0.0,
        Point::from_xy(bcx, bcy),
        br,
        vec![
            GradientStop::new(0.0, theme.glyph_hi),
            GradientStop::new(0.55, theme.glyph),
            GradientStop::new(1.0, theme.glyph_lo),
        ],
        SpreadMode::Pad,
        Transform::identity(),
    );
    fill_shader(pm, &e.bud, body, theme.glyph, t);

    // Stem: a cylinder, lighter on the lit (left) side.
    let stem = LinearGradient::new(
        Point::from_xy(bcx - 0.20 * s, bcy),
        Point::from_xy(bcx + 0.20 * s, bcy),
        vec![
            GradientStop::new(0.0, theme.glyph),
            GradientStop::new(1.0, theme.glyph_lo),
        ],
        SpreadMode::Pad,
        Transform::identity(),
    );
    fill_shader(pm, &e.stem, stem, theme.glyph, t);
    fill_t(pm, &e.miccap, theme.glyph_dark, t);

    // Matte silicone tip with its own soft dome shading.
    let tcx = bcx + 0.01 * s;
    let tcy = oy + 0.17 * s;
    let tip = RadialGradient::new(
        Point::from_xy(tcx - 0.3 * br, tcy - 0.3 * br),
        0.0,
        Point::from_xy(tcx, tcy),
        0.30 * s,
        vec![
            GradientStop::new(0.0, lighten(theme.glyph_shade, 18)),
            GradientStop::new(1.0, theme.glyph_shade),
        ],
        SpreadMode::Pad,
        Transform::identity(),
    );
    fill_shader(pm, &e.tip, tip, theme.glyph_shade, t);
    fill_t(pm, &e.nozzle, theme.glyph_dark, t);
    fill_t(pm, &e.sensor, theme.glyph_shade, t);

    // Specular: one soft localised hotspot in the upper-left, the way
    // light actually catches a glossy sphere (never a symmetric band).
    let gx = bcx - 0.30 * br;
    let gy = bcy - 0.40 * br;
    let gr = 0.34 * br;
    let mut glint = PathBuilder::new();
    glint.push_oval(tiny_skia::Rect::from_xywh(gx - gr, gy - gr, 2.0 * gr, 2.0 * gr).unwrap());
    if let Some(g) = glint.finish() {
        let hot = RadialGradient::new(
            Point::from_xy(gx, gy),
            0.0,
            Point::from_xy(gx, gy),
            gr,
            vec![
                GradientStop::new(0.0, theme.spec),
                GradientStop::new(0.55, with_alpha(theme.spec, 40)),
                GradientStop::new(1.0, with_alpha(theme.spec, 0)),
            ],
            SpreadMode::Pad,
            Transform::identity(),
        );
        fill_shader(pm, &g, hot, theme.spec, t);
    }
}

fn with_alpha(c: Color, a: u8) -> Color {
    Color::from_rgba8(
        (c.red() * 255.0) as u8,
        (c.green() * 255.0) as u8,
        (c.blue() * 255.0) as u8,
        a,
    )
}

fn lighten(c: Color, by: u8) -> Color {
    let f = |v: f32| ((v * 255.0) as u16 + by as u16).min(255) as u8;
    Color::from_rgba8(
        f(c.red()),
        f(c.green()),
        f(c.blue()),
        (c.alpha() * 255.0) as u8,
    )
}

fn fill_shader(
    pm: &mut Pixmap,
    path: &tiny_skia::Path,
    shader: Option<Shader>,
    fallback: Color,
    t: Transform,
) {
    let mut p = Paint {
        anti_alias: true,
        ..Default::default()
    };
    match shader {
        Some(s) => p.shader = s,
        None => p.set_color(fallback),
    }
    pm.fill_path(path, &p, FillRule::Winding, t, None);
}

fn drop_shadow(pm: &mut Pixmap, x0: f32, y0: f32, cw: f32, ch: f32, theme: &Theme) {
    for i in (1..=6).rev() {
        let d = i as f32;
        let r = rounded_rect(x0 - d, y0 - d + 1.5, cw + 2.0 * d, ch + 2.0 * d, RADIUS + d);
        fill(pm, &r, theme.shadow);
    }
}

fn card(pm: &mut Pixmap, x0: f32, y0: f32, cw: f32, ch: f32, theme: &Theme) {
    let body = rounded_rect(x0, y0, cw, ch, RADIUS);

    let grad = LinearGradient::new(
        Point::from_xy(x0, y0),
        Point::from_xy(x0, y0 + ch),
        vec![
            GradientStop::new(0.0, theme.card_top),
            GradientStop::new(1.0, theme.card_bot),
        ],
        SpreadMode::Pad,
        Transform::identity(),
    );
    let mut p = Paint {
        anti_alias: true,
        ..Default::default()
    };
    if let Some(shader) = grad {
        p.shader = shader;
    } else {
        p.set_color(theme.card_top);
    }
    pm.fill_path(&body, &p, FillRule::Winding, Transform::identity(), None);

    let mut hp = Paint {
        anti_alias: true,
        ..Default::default()
    };
    hp.set_color(theme.highlight);
    pm.stroke_path(
        &body,
        &hp,
        &Stroke {
            width: 1.0,
            ..Default::default()
        },
        Transform::identity(),
        None,
    );
}

fn donut(pm: &mut Pixmap, font: &Font, theme: &Theme, cx: f32, cy: f32, pod: Pod, label: &str) {
    let rr = DONUT_R;
    let thick = DONUT_THICK;

    let track = ring(cx, cy, rr, 0.0, std::f32::consts::TAU, 96);
    let mut p = Paint {
        anti_alias: true,
        ..Default::default()
    };
    p.set_color(theme.track);
    pm.stroke_path(
        &track,
        &p,
        &Stroke {
            width: thick,
            ..Default::default()
        },
        Transform::identity(),
        None,
    );

    match pod.level {
        Some(level) if level > 0 => {
            let frac = (level.min(100) as f32) / 100.0;
            let start = -std::f32::consts::FRAC_PI_2;
            let arc = ring(cx, cy, rr, start, start + frac * std::f32::consts::TAU, 96);
            p.set_color(theme.ramp(pod));
            let s = Stroke {
                width: thick,
                line_cap: LineCap::Round,
                ..Default::default()
            };
            pm.stroke_path(&arc, &p, &s, Transform::identity(), None);

            let pct = format!("{level}");
            if pod.charging {
                text_centered(
                    pm,
                    font,
                    &pct,
                    cx,
                    cy + 1.0,
                    &Ink::semibold(15.0, theme.text),
                );
                let b = assets::bolt(9.0, cx, cy + 11.0);
                fill(pm, &b, theme.accent);
            } else {
                text_centered(
                    pm,
                    font,
                    &pct,
                    cx,
                    cy + 5.0,
                    &Ink::semibold(16.0, theme.text),
                );
            }
        }
        _ => {
            text_centered(pm, font, "—", cx, cy + 5.0, &Ink::new(15.0, theme.sub));
        }
    }

    text_centered(
        pm,
        font,
        label,
        cx,
        cy + rr + 12.0,
        &Ink::new(10.0, theme.sub),
    );
}

fn disc(pm: &mut Pixmap, cx: f32, cy: f32, r: f32, color: Color) {
    let mut pb = PathBuilder::new();
    pb.push_circle(cx, cy, r);
    if let Some(path) = pb.finish() {
        fill(pm, &path, color);
    }
}

fn fill(pm: &mut Pixmap, path: &tiny_skia::Path, color: Color) {
    fill_t(pm, path, color, Transform::identity());
}

fn fill_t(pm: &mut Pixmap, path: &tiny_skia::Path, color: Color, t: Transform) {
    let mut p = Paint {
        anti_alias: true,
        ..Default::default()
    };
    p.set_color(color);
    pm.fill_path(path, &p, FillRule::Winding, t, None);
}

fn ring(cx: f32, cy: f32, r: f32, a0: f32, a1: f32, segs: usize) -> tiny_skia::Path {
    let mut pb = PathBuilder::new();
    for i in 0..=segs {
        let t = a0 + (a1 - a0) * (i as f32 / segs as f32);
        let (x, y) = (cx + r * t.cos(), cy + r * t.sin());
        if i == 0 {
            pb.move_to(x, y);
        } else {
            pb.line_to(x, y);
        }
    }
    pb.finish().expect("ring path")
}

fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> tiny_skia::Path {
    let r = r.min(w * 0.5).min(h * 0.5);
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.quad_to(x + w, y, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.quad_to(x + w, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.quad_to(x, y + h, x, y + h - r);
    pb.line_to(x, y + r);
    pb.quad_to(x, y, x + r, y);
    pb.close();
    pb.finish().expect("rounded rect")
}

struct Ink {
    px: f32,
    color: Color,
    weight: f32,
}

impl Ink {
    fn new(px: f32, color: Color) -> Self {
        Ink {
            px,
            color,
            weight: 0.0,
        }
    }
    fn semibold(px: f32, color: Color) -> Self {
        Ink {
            px,
            color,
            weight: 0.4,
        }
    }
}

fn text_width(font: &Font, s: &str, px: f32) -> f32 {
    s.chars().map(|c| font.metrics(c, px).advance_width).sum()
}

fn fit(font: &Font, s: &str, max_w: f32, ideal: f32, min: f32) -> (String, f32) {
    let mut px = ideal;
    while px > min && text_width(font, s, px) > max_w {
        px -= 0.5;
    }
    if text_width(font, s, px) <= max_w {
        return (s.to_string(), px);
    }
    let ell = text_width(font, "…", px);
    let mut out = String::new();
    let mut w = 0.0;
    for ch in s.chars() {
        let cw = font.metrics(ch, px).advance_width;
        if w + cw + ell > max_w {
            break;
        }
        w += cw;
        out.push(ch);
    }
    out.push('…');
    (out, px)
}

fn text_centered(pm: &mut Pixmap, font: &Font, s: &str, cx: f32, baseline: f32, ink: &Ink) {
    let w = text_width(font, s, ink.px);
    text(pm, font, s, cx - w * 0.5, baseline, ink);
}

fn text(pm: &mut Pixmap, font: &Font, s: &str, x: f32, baseline: f32, ink: &Ink) {
    let mut pen = x;
    for ch in s.chars() {
        let (m, bitmap) = font.rasterize(ch, ink.px);
        if m.width > 0 && m.height > 0 {
            let gx = pen + m.xmin as f32;
            let gy = baseline - m.height as f32 - m.ymin as f32;
            blit(pm, &bitmap, (m.width, m.height), (gx, gy), ink.color);
            if ink.weight > 0.0 {
                blit(
                    pm,
                    &bitmap,
                    (m.width, m.height),
                    (gx + ink.weight, gy),
                    ink.color,
                );
            }
        }
        pen += m.advance_width;
    }
}

fn blit(pm: &mut Pixmap, cov: &[u8], dim: (usize, usize), pos: (f32, f32), color: Color) {
    let (w, h) = dim;
    let pw = pm.width() as i32;
    let ph = pm.height() as i32;
    let (cr, cg, cb, ca) = (color.red(), color.green(), color.blue(), color.alpha());
    let buf = pm.data_mut();
    let bx = pos.0.round() as i32;
    let by = pos.1.round() as i32;
    for gy in 0..h {
        let py = by + gy as i32;
        if py < 0 || py >= ph {
            continue;
        }
        for gx in 0..w {
            let px = bx + gx as i32;
            if px < 0 || px >= pw {
                continue;
            }
            let c = cov[gy * w + gx] as f32 / 255.0;
            if c <= 0.0 {
                continue;
            }
            let a = c * ca;
            let idx = ((py * pw + px) * 4) as usize;
            let inv = 1.0 - a;
            buf[idx] = (cr * a * 255.0 + buf[idx] as f32 * inv).round() as u8;
            buf[idx + 1] = (cg * a * 255.0 + buf[idx + 1] as f32 * inv).round() as u8;
            buf[idx + 2] = (cb * a * 255.0 + buf[idx + 2] as f32 * inv).round() as u8;
            buf[idx + 3] = (a * 255.0 + buf[idx + 3] as f32 * inv).round() as u8;
        }
    }
}

pub fn to_bgra_premul(pm: &Pixmap) -> Vec<u8> {
    let mut out = pm.data().to_vec();
    for px in out.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
    }
    out
}

/// Round-trip premultiplied BGRA back into a PNG — the notify fallback
/// only gets the BGRA frame but the notification daemon wants an image.
pub fn png_from_bgra(bgra: &[u8], w: u32, h: u32) -> Option<Vec<u8>> {
    let mut rgba = bgra.to_vec();
    for px in rgba.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
    }
    let size = tiny_skia::IntSize::from_wh(w, h)?;
    let pm = Pixmap::from_vec(rgba, size)?;
    pm.encode_png().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_name_keeps_ideal_size() {
        let f = ui_font();
        let (s, px) = fit(f, "AirPods Pro 2", 178.0, 19.0, 12.0);
        assert_eq!(s, "AirPods Pro 2");
        assert_eq!(px, 19.0);
        assert!(text_width(f, &s, px) <= 178.0);
    }

    #[test]
    fn long_name_never_overflows() {
        let f = ui_font();
        let long = "AirPods Pro (2nd generation, USB-C, extra long)";
        let (s, px) = fit(f, long, 178.0, 19.0, 12.0);
        assert!(
            text_width(f, &s, px) <= 178.0,
            "fitted text still overflows"
        );
        assert!(s.ends_with('…') || s == long);
        assert!(px >= 12.0);
    }

    fn red_pixels(pm: &Pixmap) -> usize {
        pm.data()
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[0] > 180 && p[1] < 120 && p[2] < 120 && p[3] > 180)
            .count()
    }

    #[test]
    fn blank_tracks_missing_levels() {
        let mut s = Snapshot::sample();
        assert!(!s.blank());
        s.left = Pod::default();
        s.right = Pod::default();
        assert!(!s.blank(), "the case still has a level");
        s.case = Pod::default();
        assert!(s.blank());
        // Charging with no level reported is still blank.
        s.case = Pod {
            level: None,
            charging: true,
        };
        assert!(s.blank());
    }

    #[test]
    fn low_threshold() {
        let mut s = Snapshot::sample();
        assert!(!s.low(), "82/79/64 is not low");
        s.right = Pod {
            level: Some(24),
            charging: false,
        };
        assert!(s.low(), "right 24% must be low");
        s.right = Pod {
            level: Some(25),
            charging: false,
        };
        assert!(!s.low(), "25% is the boundary, not below");
    }

    #[test]
    fn low_battery_paints_red_border() {
        let ok = Snapshot::sample();
        let mut low = Snapshot::sample();
        low.left = Pod {
            level: Some(11),
            charging: false,
        };
        assert!(low.low());
        let t = Theme::dark();
        let n = red_pixels(&render(&ok, &t));
        let r = red_pixels(&render(&low, &t));
        assert!(
            r > n + 200,
            "low render should add a red border (n={n}, r={r})"
        );
    }
}
