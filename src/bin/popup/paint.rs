//! Drawing primitives for the bubble: palette, text as outlines, shapes, blur.

use std::cell::Cell;

use tiny_skia::*;

use crate::assets::UI_FONT;

#[derive(Clone, Copy)]
pub struct Pal {
    pub dark: bool,
    pub card: Color,
    pub border: Color,
    pub shadow: f32,
    pub text: Color,
    pub sub: Color,
    pub fill3: Color,
    pub track: Color,
    pub ring: Color,
    pub green: Color,
    pub red: Color,
    pub line: Color,
    pub floor: f32,
}

pub fn hex(v: u32, a: u8) -> Color {
    Color::from_rgba8((v >> 16) as u8, (v >> 8) as u8, v as u8, a)
}

pub fn dark() -> Pal {
    Pal {
        dark: true,
        card: hex(0x1F1F22, 246),
        border: hex(0xFFFFFF, 22),
        shadow: 0.45,
        text: hex(0xF5F5F7, 255),
        sub: hex(0x9A9AA1, 255),
        fill3: hex(0x3A3A3E, 255),
        track: hex(0xFFFFFF, 38),
        ring: hex(0xF5F5F7, 255),
        green: hex(0x30D158, 255),
        red: hex(0xFF453A, 255),
        line: hex(0x000000, 0),
        floor: 0.55,
    }
}

pub fn light() -> Pal {
    Pal {
        dark: false,
        card: hex(0xFFFFFF, 250),
        border: hex(0x000000, 18),
        shadow: 0.16,
        text: hex(0x1D1D1F, 255),
        sub: hex(0x6E6E73, 255),
        fill3: hex(0xEDEDF0, 255),
        track: hex(0x000000, 26),
        ring: hex(0x1D1D1F, 255),
        green: hex(0x248A3D, 255),
        red: hex(0xD70015, 255),
        line: hex(0x000000, 34),
        floor: 0.14,
    }
}

pub struct Ob<'a> {
    pb: &'a mut PathBuilder,
    x: f32,
    y: f32,
    k: f32,
}
impl ttf_parser::OutlineBuilder for Ob<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        self.pb.move_to(self.x + x * self.k, self.y - y * self.k)
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.pb.line_to(self.x + x * self.k, self.y - y * self.k)
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.pb.quad_to(
            self.x + x1 * self.k,
            self.y - y1 * self.k,
            self.x + x * self.k,
            self.y - y * self.k,
        )
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.pb.cubic_to(
            self.x + x1 * self.k,
            self.y - y1 * self.k,
            self.x + x2 * self.k,
            self.y - y2 * self.k,
            self.x + x * self.k,
            self.y - y * self.k,
        )
    }
    fn close(&mut self) {
        self.pb.close()
    }
}

pub fn face() -> ttf_parser::Face<'static> {
    ttf_parser::Face::parse(UI_FONT, 0).expect("bundled font parses")
}

pub fn text_w(s: &str, px: f32, track: f32) -> f32 {
    let f = face();
    let k = px / f.units_per_em() as f32;
    s.chars()
        .map(|c| {
            f.glyph_index(c)
                .and_then(|g| f.glyph_hor_advance(g))
                .unwrap_or(0) as f32
                * k
                + track
        })
        .sum::<f32>()
        - track
}

/// weight: 0 = regular, ~0.5 = semibold (outline stroke in px at 16px).
#[allow(clippy::too_many_arguments)]
pub fn text(
    pm: &mut Pixmap,
    t: Transform,
    s: &str,
    x: f32,
    base: f32,
    px: f32,
    weight: f32,
    color: Color,
    track: f32,
) {
    let f = face();
    let k = px / f.units_per_em() as f32;
    let mut pb = PathBuilder::new();
    let mut pen = x;
    for c in s.chars() {
        if let Some(g) = f.glyph_index(c) {
            let mut ob = Ob {
                pb: &mut pb,
                x: pen,
                y: base,
                k,
            };
            f.outline_glyph(g, &mut ob);
            pen += f.glyph_hor_advance(g).unwrap_or(0) as f32 * k + track;
        }
    }
    if let Some(p) = pb.finish() {
        let paint = paint(color);
        pm.fill_path(&p, &paint, FillRule::Winding, t, None);
        if weight > 0.0 {
            let st = Stroke {
                width: weight * px / 16.0,
                line_join: LineJoin::Round,
                ..Default::default()
            };
            pm.stroke_path(&p, &paint, &st, t, None);
        }
    }
}

pub fn paint(c: Color) -> Paint<'static> {
    let mut p = Paint {
        anti_alias: true,
        ..Default::default()
    };
    p.set_color(c);
    p
}

pub const KAPPA: f32 = 0.552_284_8;

pub fn rrect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Path {
    let r = r.min(w / 2.0).min(h / 2.0);
    let c = r * (1.0 - KAPPA);
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - c, y, x + w, y + c, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h - c, x + w - c, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + c, y + h, x, y + h - c, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + c, x + c, y, x + r, y);
    pb.close();
    pb.finish().unwrap()
}

pub fn oval(cx: f32, cy: f32, rx: f32, ry: f32) -> Path {
    PathBuilder::from_oval(Rect::from_xywh(cx - rx, cy - ry, 2.0 * rx, 2.0 * ry).unwrap()).unwrap()
}

pub fn arc(cx: f32, cy: f32, r: f32, a0: f32, a1: f32) -> Option<Path> {
    let n = (((a1 - a0).abs() / std::f32::consts::TAU) * 96.0)
        .ceil()
        .max(2.0) as usize;
    let mut pb = PathBuilder::new();
    for i in 0..=n {
        let t = a0 + (a1 - a0) * i as f32 / n as f32;
        let (x, y) = (cx + r * t.cos(), cy + r * t.sin());
        if i == 0 {
            pb.move_to(x, y)
        } else {
            pb.line_to(x, y)
        }
    }
    pb.finish()
}

pub fn fill(pm: &mut Pixmap, p: &Path, c: Color, t: Transform) {
    pm.fill_path(p, &paint(c), FillRule::Winding, t, None);
}

pub fn stroke(pm: &mut Pixmap, p: &Path, c: Color, w: f32, cap: LineCap, t: Transform) {
    pm.stroke_path(
        p,
        &paint(c),
        &Stroke {
            width: w,
            line_cap: cap,
            line_join: LineJoin::Round,
            ..Default::default()
        },
        t,
        None,
    );
}

pub fn lin(p0: (f32, f32), p1: (f32, f32), a: Color, b: Color) -> Shader<'static> {
    LinearGradient::new(
        Point::from_xy(p0.0, p0.1),
        Point::from_xy(p1.0, p1.1),
        vec![GradientStop::new(0.0, a), GradientStop::new(1.0, b)],
        SpreadMode::Pad,
        Transform::identity(),
    )
    .unwrap()
}

pub fn fill_sh(pm: &mut Pixmap, p: &Path, sh: Shader, t: Transform) {
    let mut pt = Paint {
        anti_alias: true,
        ..Default::default()
    };
    pt.shader = sh;
    pm.fill_path(p, &pt, FillRule::Winding, t, None);
}

/// Columns and rows that hold any ink, as half-open ranges.
pub fn inked(pm: &Pixmap) -> Option<((usize, usize), (usize, usize))> {
    let w = pm.width() as usize;
    let (mut x0, mut x1, mut y0, mut y1) = (usize::MAX, 0, usize::MAX, 0);
    for (y, row) in pm.data().chunks_exact(w * 4).enumerate() {
        let px = row.as_chunks::<4>().0;
        let Some(first) = px.iter().position(|p| p[3] != 0) else {
            continue;
        };
        let last = px.iter().rposition(|p| p[3] != 0).unwrap_or(first);
        x0 = x0.min(first);
        x1 = x1.max(last + 1);
        y0 = y0.min(y);
        y1 = y1.max(y + 1);
    }
    (x0 < x1).then_some(((x0, x1), (y0, y1)))
}

/// Separable box blur, 3 passes ~ gaussian. Operates on all channels.
/// Only the inked area plus the reach of the passes is touched: outside
/// it every pixel is zero before and after.
pub fn blur(pm: &mut Pixmap, r: usize) {
    let (w, h) = (pm.width() as usize, pm.height() as usize);
    let Some(((x0, x1), (y0, y1))) = inked(pm) else {
        return;
    };
    let reach = 3 * r;
    let (x0, x1) = (x0.saturating_sub(reach), (x1 + reach).min(w));
    let (y0, y1) = (y0.saturating_sub(reach), (y1 + reach).min(h));
    for _ in 0..3 {
        for horiz in [true, false] {
            let src = pm.data().to_vec();
            let dst = pm.data_mut();
            let (outer, inner) = if horiz {
                (y0..y1, x1 - x0)
            } else {
                (x0..x1, y1 - y0)
            };
            for o in outer {
                let idx = |i: usize| {
                    if horiz {
                        (o * w + x0 + i) * 4
                    } else {
                        ((y0 + i) * w + o) * 4
                    }
                };
                let mut acc = [0u32; 4];
                let win = 2 * r + 1;
                for i in 0..(r + 1).min(inner) {
                    for c in 0..4 {
                        acc[c] += src[idx(i) + c] as u32;
                    }
                }
                for i in 0..inner {
                    for c in 0..4 {
                        dst[idx(i) + c] = (acc[c] / win as u32) as u8;
                    }
                    if i + r + 1 < inner {
                        for c in 0..4 {
                            acc[c] += src[idx(i + r + 1) + c] as u32;
                        }
                    }
                    if i >= r {
                        for c in 0..4 {
                            acc[c] -= src[idx(i - r) + c] as u32;
                        }
                    }
                }
            }
        }
    }
}

pub fn soft_shadow(
    pm: &mut Pixmap,
    s: f32,
    path: &Path,
    dy: f32,
    radius: f32,
    alpha: f32,
    t: Transform,
) {
    let mut sh = Pixmap::new(pm.width(), pm.height()).unwrap();
    fill(
        &mut sh,
        path,
        Color::from_rgba(0.0, 0.0, 0.0, alpha).unwrap(),
        t.post_translate(0.0, dy * s),
    );
    blur(&mut sh, (radius * s / 2.0).max(1.0) as usize);
    pm.draw_pixmap(
        0,
        0,
        sh.as_ref(),
        &PixmapPaint::default(),
        Transform::identity(),
        None,
    );
}

thread_local! {
    /// Device pixels per logical pixel of the card being drawn. Hairlines
    /// are sized against it; per thread, as tests draw in parallel.
    static SCALE: Cell<f32> = const { Cell::new(2.0) };
}

pub fn set_scale(s: f32) {
    SCALE.with(|c| c.set(s));
}

pub fn scale() -> f32 {
    SCALE.with(Cell::get)
}
