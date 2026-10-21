//! The products on the card, drawn in true proportions: buds and case on
//! one millimetre scale, AirPods Max on its own.

// A drawing call takes where, how big, which palette and what state.
#![allow(clippy::too_many_arguments)]

use tiny_skia::*;

use super::paint::*;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Art {
    Gen1,
    Gen2,
    Gen3,
    Gen4,
    Gen4Anc,
    Gen5,  // AirPods 5 (Charging Case USB-C, no speaker)
    Gen5W, // AirPods 5 with Wireless Charging Case (speaker)
    Pro1,
    Pro2L,
    Pro2C,
    Pro3,
    MaxL,
    MaxC,
    Max2,
}

#[derive(Clone, Copy, PartialEq)]
pub enum BudKind {
    Classic,
    Gen3,
    Gen4,
    Pro,
    Pro3,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Port {
    Lightning,
    UsbC,
}

pub struct CaseSpec {
    w: f32,
    h: f32,
    r: f32,
    seam: f32,
    led: Option<f32>, // y of front LED in mm, None = LED inside lid
    speaker: bool,
    lanyard: bool,
    notch: bool, // finger notch across the lid seam (all Pro cases)
    port: Port,
}

impl Art {
    /// The drawing for a detected model. An unknown one gets the most
    /// common case rather than none.
    pub fn of(m: podctl::caps::Model) -> Self {
        use podctl::caps::Model as M;
        match m {
            M::Gen1 => Art::Gen1,
            M::Gen2 => Art::Gen2,
            M::Gen3 => Art::Gen3,
            M::Gen4 => Art::Gen4,
            M::Gen4Anc => Art::Gen4Anc,
            M::Pro => Art::Pro1,
            M::Pro2 => Art::Pro2L,
            M::Pro2UsbC | M::Unknown => Art::Pro2C,
            M::Pro3 => Art::Pro3,
            M::Max => Art::MaxL,
            M::MaxUsbC => Art::MaxC,
            M::Max2 => Art::Max2,
        }
    }

    /// How far the buds stand above the case's floor, in mm: their
    /// middle level with the middle of the case.
    pub fn bud_lift(self) -> f32 {
        ((self.case().h - bud_geo(self.bud()).h) / 2.0).max(0.0)
    }

    pub fn by_slug(slug: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.slug() == slug)
    }

    pub const ALL: [Art; 14] = {
        use Art::*;
        [
            Gen1, Gen2, Gen3, Gen4, Gen4Anc, Gen5, Gen5W, Pro1, Pro2L, Pro2C, Pro3, MaxL, MaxC,
            Max2,
        ]
    };
    pub fn slug(self) -> &'static str {
        use Art::*;
        match self {
            Gen1 => "airpods1",
            Gen2 => "airpods2",
            Gen3 => "airpods3",
            Gen4 => "airpods4",
            Gen4Anc => "airpods4-anc",
            Gen5 => "airpods5",
            Gen5W => "airpods5-wireless",
            Pro1 => "pro1",
            Pro2L => "pro2-lightning",
            Pro2C => "pro2-usbc",
            Pro3 => "pro3",
            MaxL => "max-lightning",
            MaxC => "max-usbc",
            Max2 => "max2",
        }
    }
    pub fn is_max(self) -> bool {
        matches!(self, Art::MaxL | Art::MaxC | Art::Max2)
    }
    pub fn bud(self) -> BudKind {
        use Art::*;
        match self {
            Gen1 | Gen2 => BudKind::Classic,
            Gen3 => BudKind::Gen3,
            // AirPods 5: Apple lists identical bud dimensions (30.2 x 18.3 x 18.1 mm, 4.3 g) and reviews
            // describe the buds as visually unchanged -> same geometry as AirPods 4.
            Gen4 | Gen4Anc | Gen5 | Gen5W => BudKind::Gen4,
            Pro3 => BudKind::Pro3,
            _ => BudKind::Pro,
        }
    }
    #[rustfmt::skip]
    pub fn case(self) -> CaseSpec {
        use Art::*;
        match self {
            Gen1 => CaseSpec { w: 44.3, h: 53.5, r: 9.0, seam: 14.2, led: None, speaker: false, lanyard: false, notch: false, port: Port::Lightning },
            Gen2 => CaseSpec { w: 44.3, h: 53.5, r: 9.0, seam: 14.2, led: Some(27.0), speaker: false, lanyard: false, notch: false, port: Port::Lightning },
            Gen3 => CaseSpec { w: 54.4, h: 46.4, r: 13.0, seam: 15.0, led: Some(27.5), speaker: false, lanyard: false, notch: false, port: Port::Lightning },
            Gen4 => CaseSpec { w: 50.1, h: 46.2, r: 16.5, seam: 14.6, led: Some(27.5), speaker: false, lanyard: false, notch: false, port: Port::UsbC },
            Gen4Anc => CaseSpec { w: 50.1, h: 46.2, r: 16.5, seam: 14.6, led: Some(27.5), speaker: true, lanyard: false, notch: false, port: Port::UsbC },
            Gen5 => CaseSpec { w: 50.1, h: 46.2, r: 16.5, seam: 14.6, led: Some(27.5), speaker: false, lanyard: false, notch: false, port: Port::UsbC },
            Gen5W => CaseSpec { w: 50.1, h: 46.2, r: 16.5, seam: 14.6, led: Some(27.5), speaker: true, lanyard: false, notch: false, port: Port::UsbC },
            Pro1 => CaseSpec { w: 60.6, h: 45.2, r: 13.0, seam: 15.6, led: Some(27.0), speaker: false, lanyard: false, notch: true, port: Port::Lightning },
            Pro2L => CaseSpec { w: 60.6, h: 45.2, r: 13.0, seam: 15.6, led: Some(27.0), speaker: true, lanyard: true, notch: true, port: Port::Lightning },
            Pro2C => CaseSpec { w: 60.6, h: 45.2, r: 13.0, seam: 15.6, led: Some(27.0), speaker: true, lanyard: true, notch: true, port: Port::UsbC },
            Pro3 => CaseSpec { w: 62.2, h: 47.2, r: 15.0, seam: 16.0, led: Some(28.0), speaker: true, lanyard: true, notch: true, port: Port::UsbC },
            _ => unreachable!(),
        }
    }
}

/// Closed Catmull-Rom spline through the given points.
pub fn spline(pts: &[(f32, f32)]) -> Path {
    let n = pts.len();
    let mut pb = PathBuilder::new();
    pb.move_to(pts[0].0, pts[0].1);
    for i in 0..n {
        let p0 = pts[(i + n - 1) % n];
        let p1 = pts[i];
        let p2 = pts[(i + 1) % n];
        let p3 = pts[(i + 2) % n];
        let c1 = (p1.0 + (p2.0 - p0.0) / 6.0, p1.1 + (p2.1 - p0.1) / 6.0);
        let c2 = (p2.0 - (p3.0 - p1.0) / 6.0, p2.1 - (p3.1 - p1.1) / 6.0);
        pb.cubic_to(c1.0, c1.1, c2.0, c2.1, p2.0, p2.1);
    }
    pb.close();
    pb.finish().unwrap()
}

pub fn rot(p: Path, deg: f32, cx: f32, cy: f32) -> Path {
    p.transform(Transform::from_rotate_at(deg, cx, cy)).unwrap()
}

pub fn pill(cx: f32, cy: f32, w: f32, h: f32, deg: f32) -> Path {
    rot(
        rrect(cx - w / 2.0, cy - h / 2.0, w, h, h / 2.0),
        deg,
        cx,
        cy,
    )
}

pub fn ell(cx: f32, cy: f32, rx: f32, ry: f32, deg: f32) -> Path {
    rot(oval(cx, cy, rx, ry), deg, cx, cy)
}

/// Tapered stem with rounded bottom corners.
pub fn stem(x0t: f32, x1t: f32, yt: f32, x0b: f32, x1b: f32, yb: f32, rb: f32) -> Path {
    let c = rb * KAPPA;
    let mut pb = PathBuilder::new();
    pb.move_to(x0t, yt);
    pb.line_to(x1t, yt);
    pb.line_to(x1b, yb - rb);
    pb.cubic_to(x1b, yb - rb + c, x1b - rb + c, yb, x1b - rb, yb);
    pb.line_to(x0b + rb, yb);
    pb.cubic_to(x0b + rb - c, yb, x0b, yb - rb + c, x0b, yb - rb);
    pb.close();
    pb.finish().unwrap()
}

pub fn mix(a: Color, b: Color, t: f32) -> Color {
    Color::from_rgba(
        a.red() + (b.red() - a.red()) * t,
        a.green() + (b.green() - a.green()) * t,
        a.blue() + (b.blue() - a.blue()) * t,
        a.alpha() + (b.alpha() - a.alpha()) * t,
    )
    .unwrap()
}

pub fn lin3(
    p0: (f32, f32),
    p1: (f32, f32),
    a: Color,
    b: Color,
    c: Color,
    mid: f32,
) -> Shader<'static> {
    LinearGradient::new(
        Point::from_xy(p0.0, p0.1),
        Point::from_xy(p1.0, p1.1),
        vec![
            GradientStop::new(0.0, a),
            GradientStop::new(mid, b),
            GradientStop::new(1.0, c),
        ],
        SpreadMode::Pad,
        Transform::identity(),
    )
    .unwrap()
}

pub fn mask_of(pm: &Pixmap, parts: &[&Path], t: Transform) -> Mask {
    let mut m = Mask::new(pm.width(), pm.height()).unwrap();
    for p in parts {
        m.fill_path(p, FillRule::Winding, true, t);
    }
    m
}

/// Material for soft plastic / metal volumes.
#[derive(Clone, Copy)]
pub struct Mat {
    hi: Color,
    lo: Color,
    edge: f32, // blur in mm that rounds off the silhouette
}

pub fn shell(pal: &Pal) -> Mat {
    if pal.dark {
        Mat {
            hi: hex(0xFCFCFD, 255),
            lo: hex(0x8E8E96, 255),
            edge: 2.0,
        }
    } else {
        Mat {
            hi: hex(0xFFFFFF, 255),
            lo: hex(0xB4B4BC, 255),
            edge: 2.0,
        }
    }
}

/// Pillow-shaded volume: solid `lo`, then the blurred silhouette in `hi`, nudged towards the
/// light (upper left, device space), clipped back to the silhouette. Edges fall off softly.
/// `grad` (mm, object space) darkens the hi layer from top to bottom.
pub fn volume(
    pm: &mut Pixmap,
    parts: &[&Path],
    t: Transform,
    kdev: f32,
    m: Mat,
    grad: ((f32, f32), (f32, f32)),
    line: Color,
) {
    if line.alpha() > 0.0 {
        for p in parts {
            stroke(pm, p, line, 1.2 * scale() / kdev, LineCap::Round, t);
        }
    }
    let mask = mask_of(pm, parts, t);
    for p in parts {
        fill(pm, p, m.lo, t);
    }
    let mut layer = Pixmap::new(pm.width(), pm.height()).unwrap();
    let off = (-0.30 * m.edge * kdev, -0.45 * m.edge * kdev);
    let lt = t.post_translate(off.0, off.1);
    let hi2 = mix(m.hi, m.lo, 0.28);
    for p in parts {
        fill_sh(&mut layer, p, lin(grad.0, grad.1, m.hi, hi2), lt);
    }
    blur(&mut layer, ((m.edge * kdev) / 2.0).max(1.0) as usize);
    pm.draw_pixmap(
        0,
        0,
        layer.as_ref(),
        &PixmapPaint::default(),
        Transform::identity(),
        Some(&mask),
    );
}

/// Soft specular blob (mm, object space), clipped to `clip`.
pub fn glint(
    pm: &mut Pixmap,
    clip: &[&Path],
    blob: &Path,
    t: Transform,
    kdev: f32,
    a: f32,
    r_mm: f32,
) {
    let mask = mask_of(pm, clip, t);
    let mut layer = Pixmap::new(pm.width(), pm.height()).unwrap();
    fill(
        &mut layer,
        blob,
        Color::from_rgba(1.0, 1.0, 1.0, a).unwrap(),
        t,
    );
    blur(&mut layer, ((r_mm * kdev) / 2.0).max(1.0) as usize);
    pm.draw_pixmap(
        0,
        0,
        layer.as_ref(),
        &PixmapPaint::default(),
        Transform::identity(),
        Some(&mask),
    );
}

/// Shadow/darkening blob clipped to `clip`.
pub fn shade(
    pm: &mut Pixmap,
    clip: &[&Path],
    blob: &Path,
    t: Transform,
    kdev: f32,
    a: f32,
    r_mm: f32,
) {
    let mask = mask_of(pm, clip, t);
    let mut layer = Pixmap::new(pm.width(), pm.height()).unwrap();
    fill(
        &mut layer,
        blob,
        Color::from_rgba(0.0, 0.0, 0.0, a).unwrap(),
        t,
    );
    blur(&mut layer, ((r_mm * kdev) / 2.0).max(1.0) as usize);
    pm.draw_pixmap(
        0,
        0,
        layer.as_ref(),
        &PixmapPaint::default(),
        Transform::identity(),
        Some(&mask),
    );
}

/// Floor shadow for an object standing at (cx, y), logical px. Light comes from the upper left, so a
/// faint, longer cast shadow trails to the right. Light theme gets stronger grounding (white on white).
pub fn floor_shadow(pm: &mut Pixmap, pal: &Pal, s: f32, cx: f32, y: f32, rx: f32) {
    let t = Transform::from_scale(s, s);
    let g = if pal.dark { 1.0 } else { 1.45 };
    let f = (pal.floor * g).min(1.0);
    soft_shadow(
        pm,
        s,
        &oval(cx + rx * 0.40, y + 0.3, rx * 1.30, 1.8),
        0.0,
        7.0,
        f * if pal.dark { 0.30 } else { 0.20 },
        t,
    );
    soft_shadow(pm, s, &oval(cx, y, rx, 2.2), 0.0, 5.0, f * 0.75, t);
    soft_shadow(
        pm,
        s,
        &oval(cx, y - 0.6, rx * 0.8, 1.0),
        0.0,
        1.6,
        f * 0.5,
        t,
    );
}

/// Rim light: thin bright fringe on the silhouette edge facing away from the light (lower right in
/// screen space), so white objects separate from the background. Offsets are device-space.
pub fn rim_light(pm: &mut Pixmap, pal: &Pal, parts: &[&Path], t: Transform, kdev: f32, w_mm: f32) {
    let a = if pal.dark { 0.42 } else { 0.0 };
    if a == 0.0 {
        return;
    }
    let mask = mask_of(pm, parts, t);
    let mut l = Pixmap::new(pm.width(), pm.height()).unwrap();
    for p in parts {
        fill(&mut l, p, Color::from_rgba(1.0, 1.0, 1.0, a).unwrap(), t);
    }
    let mut cut = Paint::default();
    cut.set_color(Color::from_rgba(0.0, 0.0, 0.0, 1.0).unwrap());
    cut.blend_mode = BlendMode::DestinationOut;
    cut.anti_alias = true;
    let d = w_mm * kdev;
    let ct = t.post_translate(-d * 0.75, -d);
    for p in parts {
        l.fill_path(p, &cut, FillRule::Winding, ct, None);
    }
    blur(&mut l, ((0.35 * kdev) / 2.0).max(1.0) as usize);
    pm.draw_pixmap(
        0,
        0,
        l.as_ref(),
        &PixmapPaint::default(),
        Transform::identity(),
        Some(&mask),
    );
}

pub fn mesh_col(pal: &Pal) -> Color {
    if pal.dark {
        hex(0x2A2A2E, 255)
    } else {
        hex(0x36363A, 255)
    }
}

pub fn mesh(pm: &mut Pixmap, pal: &Pal, p: &Path, t: Transform, kdev: f32) {
    fill(pm, p, mesh_col(pal), t);
    // thin lighter lip so the grille reads as recessed
    stroke(
        pm,
        p,
        hex(0xFFFFFF, 60),
        0.35 / kdev * 2.0,
        LineCap::Round,
        t.post_translate(0.0, 0.5),
    );
}

// Buds (mm, canonical = LEFT bud, inner side = +x).

/// Silicone ear tip seen at an angle: a flat base disc (radius r) sitting on the shell with a dome of
/// height hd on top. The tip axis is turned `th` degrees from the viewer towards +x and rolled by `rot`.
#[derive(Clone, Copy)]
pub struct TipSpec {
    c: (f32, f32), // base centre, mm
    r: f32,
    hd: f32,
    th: f32,
    rot: f32,
    nr: f32, // nozzle opening radius
}

impl TipSpec {
    fn cs(&self) -> (f32, f32) {
        let a = self.th.to_radians();
        (a.cos(), a.sin())
    }
    fn tr(&self) -> Transform {
        Transform::from_translate(self.c.0, self.c.1).pre_rotate(self.rot)
    }
    /// Silhouette: back half = projected base rim (rx = r cos th), front half = projected dome
    /// (rx = sqrt(r^2 cos^2 + hd^2 sin^2)). Both halves share the vertical extent r, so they meet
    /// tangentially at top and bottom.
    fn outline(&self) -> Path {
        let (co, si) = self.cs();
        let rb = self.r * co;
        let rf = (self.r * self.r * co * co + self.hd * self.hd * si * si).sqrt();
        let r = self.r;
        let k = KAPPA;
        let mut pb = PathBuilder::new();
        pb.move_to(0.0, -r);
        pb.cubic_to(rf * k, -r, rf, -r * k, rf, 0.0);
        pb.cubic_to(rf, r * k, rf * k, r, 0.0, r);
        pb.cubic_to(-rb * k, r, -rb, r * k, -rb, 0.0);
        pb.cubic_to(-rb, -r * k, -rb * k, -r, 0.0, -r);
        pb.close();
        pb.finish().unwrap().transform(self.tr()).unwrap()
    }
    fn base(&self) -> Path {
        let (co, _) = self.cs();
        oval(0.0, 0.0, self.r * co, self.r)
            .transform(self.tr())
            .unwrap()
    }
    /// Apex of the dome (centre of the nozzle), object space.
    fn apex(&self) -> (f32, f32) {
        let (_, si) = self.cs();
        self.tr().map_point_xy(self.hd * si, 0.0)
    }
    /// Ellipse around the apex, `dr` added to the nozzle radius, foreshortened like the nozzle.
    fn ring(&self, dr: f32, dx: f32, dy: f32) -> Path {
        let (co, si) = self.cs();
        let rr = self.nr + dr;
        oval(self.hd * si + dx, dy, rr * co, rr)
            .transform(self.tr())
            .unwrap()
    }
    /// Point in the tip's local (unrotated) frame -> object space.
    fn at(&self, x: f32, y: f32) -> (f32, f32) {
        self.tr().map_point_xy(x, y)
    }
}

trait MapXY {
    fn map_point_xy(&self, x: f32, y: f32) -> (f32, f32);
}
impl MapXY for Transform {
    fn map_point_xy(&self, x: f32, y: f32) -> (f32, f32) {
        let mut p = [Point::from_xy(x, y)];
        self.map_points(&mut p);
        (p[0].x, p[0].y)
    }
}

pub struct BudGeo {
    h: f32,
    head: Path,
    stem: Path,
    tip: Option<Path>, // silicone ear tip silhouette (Pro), drawn in front of the shell
    tip_spec: Option<TipSpec>,
    meshes: Vec<Path>,
    sensor: Option<Path>,
    cap: Option<Path>,    // metal contact cap (classic)
    bottom: Option<Path>, // dark bottom mic
    pivot: (f32, f32),    // stem bottom centre
    spec: Path,           // head specular (object space, for the unmirrored bud)
}

pub fn bud_geo(k: BudKind) -> BudGeo {
    match k {
        // Three-quarter view: the ear tip is turned ~18 deg from the viewer towards +x, i.e. towards
        // the other bud of the pair (canonical = left bud, unmirrored).
        BudKind::Pro => BudGeo {
            h: 30.9,
            head: spline(&[
                (9.6, 0.3),
                (13.6, 1.0),
                (16.7, 3.3),
                (18.2, 6.9),
                (17.8, 10.9),
                (15.6, 14.3),
                (12.2, 16.3),
                (8.2, 16.8),
                (4.6, 15.8),
                (1.9, 13.2),
                (0.5, 9.5),
                (0.8, 5.5),
                (2.8, 2.3),
                (5.8, 0.7),
            ]),
            stem: stem(2.7, 8.5, 12.0, 3.1, 7.9, 30.9, 1.6),
            tip: Some(
                TipSpec {
                    c: (13.4, 10.4),
                    r: 6.3,
                    hd: 4.6,
                    th: 18.0,
                    rot: 8.0,
                    nr: 2.25,
                }
                .outline(),
            ),
            tip_spec: Some(TipSpec {
                c: (13.4, 10.4),
                r: 6.3,
                hd: 4.6,
                th: 18.0,
                rot: 8.0,
                nr: 2.25,
            }),
            meshes: vec![pill(7.2, 2.5, 5.4, 1.45, -12.0)],
            sensor: Some(rrect(4.0, 19.2, 2.6, 8.6, 1.3)),
            cap: None,
            bottom: Some(rrect(3.6, 30.0, 3.6, 0.9, 0.45)),
            pivot: (5.5, 30.9),
            spec: ell(6.4, 5.0, 3.8, 2.6, -22.0),
        },
        BudKind::Pro3 => BudGeo {
            h: 30.9,
            head: spline(&[
                (8.8, 0.4),
                (12.4, 1.0),
                (15.2, 3.2),
                (16.5, 6.7),
                (16.1, 10.6),
                (14.0, 13.9),
                (10.8, 16.0),
                (7.6, 17.0),
                (4.4, 16.4),
                (1.9, 13.9),
                (0.6, 10.0),
                (0.8, 5.8),
                (2.6, 2.5),
                (5.4, 0.8),
            ]),
            stem: stem(2.6, 7.9, 12.6, 3.1, 7.4, 30.9, 1.5),
            tip: Some(
                TipSpec {
                    c: (13.0, 10.6),
                    r: 6.8,
                    hd: 5.2,
                    th: 18.0,
                    rot: 8.0,
                    nr: 2.2,
                }
                .outline(),
            ),
            tip_spec: Some(TipSpec {
                c: (13.0, 10.6),
                r: 6.8,
                hd: 5.2,
                th: 18.0,
                rot: 8.0,
                nr: 2.2,
            }),
            meshes: vec![pill(6.6, 2.4, 4.0, 1.25, -12.0)],
            sensor: Some(rrect(3.9, 20.4, 2.4, 7.6, 1.2)),
            cap: None,
            bottom: Some(rrect(3.5, 30.0, 3.3, 0.9, 0.45)),
            pivot: (5.25, 30.9),
            spec: ell(6.0, 5.2, 3.4, 2.4, -22.0),
        },
        // Side profile, speaker faces +x. Head 17.6 wide (between Apple's 16.5 width and 18.0 depth),
        // stem 6.0 -> 5.3 mm, overall height 38.0 mm (optical correction: at the true 40.5 mm they read too long).
        BudKind::Classic => BudGeo {
            h: 38.0,
            head: spline(&[
                (8.2, 0.2),
                (12.3, 1.0),
                (15.5, 3.4),
                (17.5, 7.4),
                (17.2, 11.8),
                (14.6, 15.2),
                (10.6, 16.9),
                (6.3, 16.6),
                (2.8, 14.5),
                (0.6, 10.9),
                (0.3, 6.6),
                (1.9, 3.0),
                (4.7, 0.9),
            ]),
            stem: stem(2.4, 8.4, 11.5, 2.75, 8.05, 38.0, 2.5),
            tip: None,
            tip_spec: None,
            meshes: vec![
                ell(15.6, 10.6, 1.15, 3.2, 32.0),
                pill(6.6, 2.0, 3.6, 1.0, -16.0),
            ],
            sensor: None,
            cap: Some(stem(2.75, 8.05, 35.7, 2.75, 8.05, 38.0, 2.5)),
            bottom: Some(oval(5.4, 37.1, 0.9, 0.35)),
            pivot: (5.4, 38.0),
            spec: ell(6.2, 5.4, 3.6, 2.7, -25.0),
        },
        BudKind::Gen3 => BudGeo {
            h: 30.79,
            head: spline(&[
                (7.6, 0.2),
                (11.4, 0.9),
                (14.6, 2.9),
                (17.0, 5.8),
                (18.2, 8.8),
                (17.2, 11.8),
                (14.0, 13.8),
                (10.2, 15.4),
                (6.6, 16.6),
                (3.0, 15.8),
                (0.8, 12.6),
                (0.2, 7.8),
                (1.4, 3.4),
                (4.2, 0.9),
            ]),
            stem: stem(1.6, 6.8, 12.0, 1.9, 6.4, 30.79, 1.6),
            tip: None,
            tip_spec: None,
            meshes: vec![
                ell(16.9, 9.0, 0.95, 2.7, 22.0),
                pill(6.0, 2.1, 4.4, 1.3, -18.0),
            ],
            sensor: Some(rrect(2.9, 19.0, 2.4, 8.0, 1.2)),
            cap: None,
            bottom: Some(rrect(2.6, 29.9, 3.0, 0.9, 0.45)),
            pivot: (4.1, 30.79),
            spec: ell(5.6, 5.0, 3.4, 2.4, -25.0),
        },
        BudKind::Gen4 => BudGeo {
            h: 30.2,
            head: spline(&[
                (7.8, 0.2),
                (11.6, 0.9),
                (14.8, 3.0),
                (17.1, 6.0),
                (18.2, 9.0),
                (17.2, 11.9),
                (14.2, 13.9),
                (10.4, 15.4),
                (7.0, 16.4),
                (3.2, 15.6),
                (0.9, 12.6),
                (0.2, 7.8),
                (1.4, 3.4),
                (4.3, 0.9),
            ]),
            stem: stem(1.6, 6.7, 12.0, 1.9, 6.3, 30.2, 1.6),
            tip: None,
            tip_spec: None,
            meshes: vec![
                ell(17.0, 9.2, 0.85, 2.5, 22.0),
                pill(6.1, 2.1, 3.8, 1.15, -18.0),
            ],
            sensor: Some(rrect(2.9, 18.8, 2.3, 7.6, 1.15)),
            cap: None,
            bottom: Some(rrect(2.6, 29.3, 2.9, 0.9, 0.45)),
            pivot: (4.1, 30.2),
            spec: ell(5.8, 5.0, 3.4, 2.4, -25.0),
        },
    }
}

/// Feathered mask: `blob` filled and blurred by r_mm, so masked strokes fade out instead of ending hard.
pub fn soft_mask(pm: &Pixmap, blob: &Path, t: Transform, kdev: f32, r_mm: f32) -> Mask {
    let mut l = Pixmap::new(pm.width(), pm.height()).unwrap();
    fill(&mut l, blob, hex(0xFFFFFF, 255), t);
    blur(&mut l, ((r_mm * kdev) / 2.0).max(1.0) as usize);
    Mask::from_pixmap(l.as_ref(), MaskType::Alpha)
}

/// Reflect an object-space path about x = cx when the bud is drawn mirrored, so that
/// light-dependent details (specular, occlusion) stay on the screen-space light side.
pub fn lit(p: &Path, mirror: bool, cx: f32) -> Path {
    if !mirror {
        return p.clone();
    }
    p.clone()
        .transform(Transform::from_translate(2.0 * cx, 0.0).pre_scale(-1.0, 1.0))
        .unwrap()
}

pub fn silicone(pal: &Pal) -> Mat {
    if pal.dark {
        Mat {
            hi: hex(0xEDEDF0, 255),
            lo: hex(0x9C9CA4, 255),
            edge: 3.2,
        }
    } else {
        Mat {
            hi: hex(0xF3F3F5, 255),
            lo: hex(0xB2B2BA, 255),
            edge: 3.2,
        }
    }
}

/// Fine mesh grain inside a grille path (lighter dots on the dark mesh).
pub fn mesh_tex(pm: &mut Pixmap, p: &Path, t: Transform, pitch: f32, a: u8) {
    let b = p.bounds();
    let mut pb = PathBuilder::new();
    let mut y = b.top() + pitch * 0.5;
    let mut row = 0;
    while y < b.bottom() {
        let mut x = b.left() + pitch * if row % 2 == 0 { 0.5 } else { 1.0 };
        while x < b.right() {
            pb.push_circle(x, y, pitch * 0.22);
            x += pitch;
        }
        y += pitch * 0.87;
        row += 1;
    }
    if let Some(d) = pb.finish() {
        let mask = mask_of(pm, &[p], t);
        let mut l = Pixmap::new(pm.width(), pm.height()).unwrap();
        fill(&mut l, &d, hex(0xFFFFFF, a), t);
        pm.draw_pixmap(
            0,
            0,
            l.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            Some(&mask),
        );
    }
}

pub fn hline(x0: f32, x1: f32, y: f32) -> Path {
    let mut pb = PathBuilder::new();
    pb.move_to(x0, y);
    pb.line_to(x1, y);
    pb.finish().unwrap()
}

/// Hairline seam clipped to `clip`: dark line with a catch-light just below.
pub fn seam_line(
    pm: &mut Pixmap,
    pal: &Pal,
    clip: &[&Path],
    x0: f32,
    x1: f32,
    y: f32,
    t: Transform,
) {
    let mask = mask_of(pm, clip, t);
    let mut l = Pixmap::new(pm.width(), pm.height()).unwrap();
    stroke(
        &mut l,
        &hline(x0, x1, y),
        hex(0x000000, if pal.dark { 60 } else { 40 }),
        0.16,
        LineCap::Butt,
        t,
    );
    stroke(
        &mut l,
        &hline(x0, x1, y + 0.18),
        hex(0xFFFFFF, 120),
        0.12,
        LineCap::Butt,
        t,
    );
    pm.draw_pixmap(
        0,
        0,
        l.as_ref(),
        &PixmapPaint::default(),
        Transform::identity(),
        Some(&mask),
    );
}

/// Draw one bud. `t` maps bud mm space to device px; kdev = device px per mm.
/// `mirror`: t contains a horizontal flip. Light always comes from the screen's upper left: blobs are
/// reflected back with `lit()`, offsets aligned to geometry use `lx` (+1 / -1 in object space).
pub fn draw_bud(pm: &mut Pixmap, pal: &Pal, g: &BudGeo, t: Transform, kdev: f32, mirror: bool) {
    let lx = if mirror { -1.0 } else { 1.0 };
    let m = shell(pal);
    let grad = ((0.0, 0.0), (0.0, g.h));
    let hb = g.head.bounds();
    let hc = (hb.left() + hb.right()) / 2.0;
    volume(pm, &[&g.head, &g.stem], t, kdev, m, grad, pal.line);
    glint(
        pm,
        &[&g.head],
        &lit(&g.spec, mirror, hc),
        t,
        kdev,
        0.85,
        2.0,
    );
    // stem cylinder highlight on the lit side
    let sb = g.stem.bounds();
    let scx = (sb.left() + sb.right()) / 2.0;
    let sx = sb.left() + sb.width() * 0.24;
    let band = rrect(
        sx,
        sb.top() + 3.0,
        sb.width() * 0.22,
        sb.height() - 5.0,
        0.6,
    );
    glint(pm, &[&g.stem], &lit(&band, mirror, scx), t, kdev, 0.55, 0.9);
    // head shades the stem just below the joint
    shade(
        pm,
        &[&g.stem],
        &ell(scx, hb.bottom() - 0.4, sb.width() * 0.7, 1.4, 0.0),
        t,
        kdev,
        0.10,
        1.2,
    );
    // stem end turns away from the light and towards the floor (stronger on dark: a bright stem end
    // on a dark card reads longer than it is)
    let end_a = if pal.dark { 0.20 } else { 0.07 };
    shade(
        pm,
        &[&g.stem],
        &rrect(
            sb.left() - 1.0,
            sb.bottom() - 5.5,
            sb.width() + 2.0,
            8.0,
            2.0,
        ),
        t,
        kdev,
        end_a,
        2.6,
    );
    if let Some(s) = &g.sensor {
        // force sensor flat: slightly recessed -> shadowed upper edge, lit lower edge
        shade(pm, &[&g.stem], s, t, kdev, 0.07, 0.5);
        let b = s.bounds();
        let r = rrect(b.left(), b.top(), b.width(), b.height(), b.width() / 2.0);
        let mask = mask_of(pm, &[s], t);
        let mut l = Pixmap::new(pm.width(), pm.height()).unwrap();
        stroke(
            &mut l,
            &r,
            hex(0x000000, 40),
            0.22,
            LineCap::Round,
            t.pre_translate(-0.10 * lx, -0.12),
        );
        stroke(
            &mut l,
            &r,
            hex(0xFFFFFF, 150),
            0.18,
            LineCap::Round,
            t.pre_translate(0.12 * lx, 0.16),
        );
        pm.draw_pixmap(
            0,
            0,
            l.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            Some(&mask),
        );
        stroke(pm, &r, hex(0x000000, 14), 0.12, LineCap::Round, t);
    }
    if let Some(c) = &g.cap {
        let b = c.bounds();
        let (a, z) = if mirror {
            (hex(0x8A8A90, 255), hex(0x9C9CA2, 255))
        } else {
            (hex(0x9C9CA2, 255), hex(0x8A8A90, 255))
        };
        let mid = if mirror { 0.6 } else { 0.4 };
        let hi = if pal.dark {
            hex(0xD6D6DA, 255)
        } else {
            hex(0xE6E6EA, 255)
        };
        fill_sh(
            pm,
            c,
            lin3((b.left(), 0.0), (b.right(), 0.0), a, hi, z, mid),
            t,
        );
        // joint between plastic stem and metal cap
        seam_line(
            pm,
            pal,
            &[&g.stem],
            b.left() - 1.0,
            b.right() + 1.0,
            b.top(),
            t,
        );
    } else {
        // bottom grille seam a little above the stem end
        seam_line(
            pm,
            pal,
            &[&g.stem],
            sb.left() - 1.0,
            sb.right() + 1.0,
            sb.bottom() - 1.9,
            t,
        );
    }
    rim_light(pm, pal, &[&g.head, &g.stem], t, kdev, 0.55);
    for me in &g.meshes {
        mesh(pm, pal, me, t, kdev);
        mesh_tex(pm, me, t, 0.34, 34);
    }
    if let Some(bm) = &g.bottom {
        fill(pm, bm, with_a(mesh_col(pal), 220), t);
        mesh_tex(pm, bm, t, 0.3, 30);
    }
    if let (Some(tip), Some(ts)) = (&g.tip, &g.tip_spec) {
        let tb = tip.bounds();
        let tc = (
            (tb.left() + tb.right()) / 2.0,
            (tb.top() + tb.bottom()) / 2.0,
        );
        let (rx, ry) = (tb.width() / 2.0, tb.height() / 2.0);
        let (co, si) = ts.cs();
        // contact occlusion where the silicone base sits on the shell, offset away from the light
        let ao = tip
            .clone()
            .transform(Transform::from_translate(0.45 * lx, 0.65))
            .unwrap();
        shade(pm, &[&g.head, &g.stem], &ao, t, kdev, 0.30, 1.5);
        // silicone dome: matte, lower contrast, wider falloff than the glossy shell
        volume(
            pm,
            &[tip],
            t,
            kdev,
            silicone(pal),
            ((0.0, tb.top()), (0.0, tb.bottom())),
            pal.line,
        );
        rim_light(pm, pal, &[tip], t, kdev, 0.5);
        // base skirt: the back half of the silhouette is the base rim; a faint crease just inside it
        // separates the flat skirt from the dome
        let skirt = ts.base();
        let mut sl = Pixmap::new(pm.width(), pm.height()).unwrap();
        stroke(&mut sl, &skirt, hex(0x000000, 26), 0.35, LineCap::Round, t);
        blur(&mut sl, ((0.3 * kdev) / 2.0).max(1.0) as usize);
        let back = ts.at(-ts.r * co * 0.55, 0.0);
        let smask = soft_mask(
            pm,
            &ell(back.0, back.1, ts.r * co * 0.75, ts.r * 1.1, 0.0),
            t,
            kdev,
            1.6,
        );
        pm.draw_pixmap(
            0,
            0,
            sl.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            Some(&smask),
        );
        // broad matte sheen on the lit shoulder, no hard specular
        let sheen = ell(
            tc.0 - rx * 0.30,
            tc.1 - ry * 0.42,
            rx * 0.42,
            ry * 0.30,
            -30.0,
        );
        glint(pm, &[tip], &lit(&sheen, mirror, tc.0), t, kdev, 0.55, 2.4);
        // rolled outer lip: faint catch-light just inside the rim, lit side only
        let lip_o = tip
            .clone()
            .transform(
                Transform::from_translate(tc.0, tc.1)
                    .pre_scale((rx - 0.55) / rx, (ry - 0.55) / ry)
                    .pre_translate(-tc.0, -tc.1),
            )
            .unwrap();
        let mut ol = Pixmap::new(pm.width(), pm.height()).unwrap();
        stroke(&mut ol, &lip_o, hex(0xFFFFFF, 150), 0.45, LineCap::Round, t);
        blur(&mut ol, ((0.25 * kdev) / 2.0).max(1.0) as usize);
        let lo_blob = ell(
            tc.0 - rx * 0.75,
            tc.1 - ry * 0.75,
            rx * 0.95,
            ry * 0.95,
            0.0,
        );
        let lo_mask = soft_mask(pm, &lit(&lo_blob, mirror, tc.0), t, kdev, 2.4);
        pm.draw_pixmap(
            0,
            0,
            ol.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            Some(&lo_mask),
        );
        // lower / far rim turns away from the light
        let mut cut = Paint::default();
        cut.set_color(Color::from_rgba(0.0, 0.0, 0.0, 1.0).unwrap());
        cut.blend_mode = BlendMode::DestinationOut;
        cut.anti_alias = true;
        let rim = ell(
            tc.0 + rx * 0.30,
            tc.1 + ry * 0.36,
            rx * 0.95,
            ry * 0.85,
            0.0,
        );
        let rim_cut = ell(
            tc.0 - rx * 0.10,
            tc.1 - ry * 0.10,
            rx * 0.95,
            ry * 0.95,
            0.0,
        );
        let mut rl = Pixmap::new(pm.width(), pm.height()).unwrap();
        let tmask = mask_of(pm, &[tip], t);
        fill(
            &mut rl,
            &lit(&rim, mirror, tc.0),
            Color::from_rgba(0.0, 0.0, 0.0, 0.16).unwrap(),
            t,
        );
        rl.fill_path(
            &lit(&rim_cut, mirror, tc.0),
            &cut,
            FillRule::Winding,
            t,
            None,
        );
        blur(&mut rl, ((1.4 * kdev) / 2.0).max(1.0) as usize);
        pm.draw_pixmap(
            0,
            0,
            rl.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            Some(&tmask),
        );

        // nozzle at the dome apex, foreshortened like the dome
        let nz = ts.ring(0.0, 0.0, 0.0);
        let collar = ts.ring(0.55, 0.0, 0.0);
        shade(pm, &[tip], &collar, t, kdev, 0.22, 0.7);
        fill(pm, &nz, mesh_col(pal), t);
        let core = ts.ring(-0.55, -0.22 * lx, -0.30);
        fill(pm, &core, with_a(hex(0x56565C, 255), 150), t);
        // mesh dots on the nozzle plane
        let (ax, _) = (ts.hd * si, 0.0);
        let mut dots = PathBuilder::new();
        let lim = ts.nr - 0.45;
        for iy in -4..=4 {
            for ix in -4..=4 {
                let u = ix as f32 * 0.5 + (iy & 1) as f32 * 0.25;
                let v = iy as f32 * 0.45;
                if u * u + v * v < lim * lim {
                    let p = ts.at(ax + u * co, v);
                    dots.push_oval(
                        Rect::from_xywh(p.0 - 0.11 * co, p.1 - 0.11, 0.22 * co, 0.22).unwrap(),
                    );
                }
            }
        }
        if let Some(d) = dots.finish() {
            fill(pm, &d, hex(0x000000, 70), t);
        }
        // lit far wall inside the opening (lower edge, on the side away from the light)
        let wall = ts.ring(-0.15, 0.30 * lx, 0.50);
        let wall_cut = ts.ring(-0.10, 0.05 * lx, 0.05);
        let mut wl = Pixmap::new(pm.width(), pm.height()).unwrap();
        fill(
            &mut wl,
            &wall,
            hex(0xFFFFFF, if pal.dark { 70 } else { 90 }),
            t,
        );
        wl.fill_path(&wall_cut, &cut, FillRule::Winding, t, None);
        let nmask = mask_of(pm, &[&nz], t);
        pm.draw_pixmap(
            0,
            0,
            wl.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            Some(&nmask),
        );
        // crisp silicone lip around the opening, catch-light towards the light
        let lip = ts.ring(0.12, 0.0, 0.0);
        let mut ll = Pixmap::new(pm.width(), pm.height()).unwrap();
        stroke(&mut ll, &lip, hex(0xFFFFFF, 200), 0.26, LineCap::Round, t);
        let ap = ts.apex();
        let lmask = soft_mask(
            pm,
            &ell(ap.0 - 1.2 * lx, ap.1 - 1.5, 2.4, 2.4, 0.0),
            t,
            kdev,
            1.2,
        );
        pm.draw_pixmap(
            0,
            0,
            ll.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            Some(&lmask),
        );
    }
}

/// Bud pair arrangement.
#[derive(Clone, Copy)]
pub struct Pose {
    gap: f32,        // mm between the two silhouettes at their closest (bounding boxes)
    rear_scale: f32, // right bud drawn this much smaller (depth)
    rear_lift: f32,  // mm: right bud stands further back, its floor contact sits higher
    rear_tilt: f32,  // deg, right bud only, top leans outward (away from the front bud)
}

pub fn pose_b() -> Pose {
    let tilt = std::env::var("TILT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(REAR_TILT);
    Pose {
        gap: 4.8,
        rear_scale: 0.93,
        rear_lift: 2.2,
        rear_tilt: tilt,
    }
}

pub const REAR_TILT: f32 = 4.0;

/// Pair of buds standing on `floor`, centred at `cx` (logical px), k px/mm.
/// sel bits: 1 left shadow, 2 right shadow, 4 left bud, 8 right bud.
pub fn draw_pair(
    pm: &mut Pixmap,
    pal: &Pal,
    s: f32,
    kind: BudKind,
    cx: f32,
    floor: f32,
    k: f32,
    sel: u8,
) {
    draw_pair_pose(pm, pal, s, kind, cx, floor, k, sel, pose_b())
}

pub fn bounds_of(parts: &[&Path], t: Transform) -> Rect {
    let (mut l, mut tp, mut r, mut b) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in parts {
        let bb = (*p).clone().transform(t).unwrap().bounds();
        l = l.min(bb.left());
        tp = tp.min(bb.top());
        r = r.max(bb.right());
        b = b.max(bb.bottom());
    }
    Rect::from_ltrb(l, tp, r, b).unwrap()
}

pub fn draw_pair_pose(
    pm: &mut Pixmap,
    pal: &Pal,
    s: f32,
    kind: BudKind,
    cx: f32,
    floor: f32,
    k: f32,
    sel: u8,
    pose: Pose,
) {
    let g = bud_geo(kind);
    let parts: Vec<&Path> = [&g.head, &g.stem]
        .into_iter()
        .chain(g.tip.as_ref())
        .collect();
    let rs = pose.rear_scale;
    let sw = g.stem.bounds().width();
    let to_pivot = Transform::from_translate(-g.pivot.0, -g.pivot.1);
    // Canonical bud = left/front, unmirrored: speaker / ear tip faces +x, i.e. towards the other bud.
    // Logical-space transforms with the stem end at x = 0 on the floor line.
    let tl0 = Transform::from_translate(0.0, floor)
        .pre_scale(k, k)
        .pre_concat(to_pivot);
    // Rear bud: mirrored, smaller, standing on a higher floor line, tilted clockwise (top to the
    // right, away from the front bud) about the outer corner of its stem end, which stays on the floor.
    let fr = floor - pose.rear_lift * k;
    let corner = sw / 2.0 * k * rs;
    let tr0 = Transform::from_translate(0.0, fr)
        .pre_concat(Transform::from_rotate_at(pose.rear_tilt, corner, 0.0))
        .pre_scale(-k * rs, k * rs)
        .pre_concat(to_pivot);
    let bl = bounds_of(&parts, tl0);
    let br = bounds_of(&parts, tr0);
    // Gap between the bounding boxes. The box edges are the speaker / ear tip (head height) on both
    // buds, also after the tilt, so the visible gap between the heads stays at `gap`.
    // profile buds (speakers face each other) need more air than the Pro tips
    let gap = if g.tip.is_some() {
        pose.gap
    } else {
        pose.gap + 2.7
    };
    let dxr = bl.right() + gap * k - br.left();
    let span_l = bl.left();
    let span_r = br.right() + dxr;
    let ox = cx - (span_l + span_r) / 2.0;
    let tl = tl0.post_translate(ox, 0.0);
    let tr = tr0.post_translate(ox + dxr, 0.0);
    let kdev = k * s;
    let contact = |pm: &mut Pixmap, x: f32, y: f32, sc: f32| {
        let t = Transform::from_scale(s, s);
        soft_shadow(
            pm,
            s,
            &oval(x, y - 0.2, sw * 0.62 * k * sc, 0.55 * sc),
            0.0,
            0.9,
            pal.floor * 0.8,
            t,
        );
    };
    if sel & 2 != 0 {
        // shadow follows the tilt: centred under the tilted silhouette, contact under the stem end
        let b = bounds_of(&parts, tr);
        let rx = (bl.width()) * 0.42 * rs;
        floor_shadow(pm, pal, s, (b.left() + b.right()) / 2.0, fr, rx);
        let end = tr.map_point_xy(g.pivot.0, g.pivot.1 - 0.4);
        contact(pm, end.0, fr, rs);
    }
    if sel & 1 != 0 {
        let b = bounds_of(&parts, tl);
        floor_shadow(
            pm,
            pal,
            s,
            (b.left() + b.right()) / 2.0,
            floor,
            bl.width() * 0.42,
        );
        contact(pm, ox, floor, 1.0);
    }
    // rear first so the front bud overlaps it
    if sel & 8 != 0 {
        draw_bud(pm, pal, &g, tr.post_scale(s, s), kdev * rs, true);
    }
    if sel & 4 != 0 {
        draw_bud(pm, pal, &g, tl.post_scale(s, s), kdev, false);
    }
}

// Cases (front view, mm).

// Status LED (charging), mm / alpha. Also used by the LED glow-in animation.
pub const LED_CORE: u32 = 0x58C078; // slightly desaturated green
pub const LED_R: f32 = 0.68; // dot radius
pub const LED_HALO_R: f32 = 1.35; // halo disc radius before blur
pub const LED_HALO_BLUR: f32 = 0.9; // halo blur radius
pub const LED_HALO_A_DARK: f32 = 0.16;
pub const LED_HALO_A_LIGHT: f32 = 0.11;

// Finger notch (all Pro cases): a long pill across the lid seam, measured on a Pro 2 case photo
// ~49 % of the case width, ~2.5 mm high, the seam a little above its middle.
pub const NOTCH_W_FRAC: f32 = 0.49;
pub const NOTCH_H: f32 = 2.5;
pub const NOTCH_ABOVE: f32 = 0.4; // share of the height above the seam

pub fn draw_case(
    pm: &mut Pixmap,
    pal: &Pal,
    s: f32,
    c: &CaseSpec,
    cx: f32,
    floor: f32,
    k: f32,
    charging: bool,
) {
    let ox = cx - c.w * k / 2.0;
    let oy = floor - c.h * k;
    let t = Transform::from_translate(ox, oy)
        .pre_scale(k, k)
        .post_scale(s, s);
    let kdev = k * s;
    floor_shadow(pm, pal, s, cx, floor, c.w * k * 0.46);
    // contact occlusion along the flat bottom
    soft_shadow(
        pm,
        s,
        &oval(cx, floor - 0.3, (c.w / 2.0 - c.r * 0.55) * k, 0.6),
        0.0,
        1.0,
        pal.floor * if pal.dark { 0.9 } else { 1.0 },
        Transform::from_scale(s, s),
    );
    let body = rrect(0.0, 0.0, c.w, c.h, c.r);
    let mut m = shell(pal);
    m.edge = 4.2;
    volume(pm, &[&body], t, kdev, m, ((0.0, 0.0), (0.0, c.h)), pal.line);
    // soft top-left sheen
    glint(
        pm,
        &[&body],
        &ell(c.w * 0.30, c.h * 0.18, c.w * 0.26, c.h * 0.09, -6.0),
        t,
        kdev,
        0.75,
        3.5,
    );
    // rounded top edge of the lid (rolling back towards the hinge): faint catch-light, then a soft
    // darker band just behind the crest
    {
        let mask = mask_of(pm, &[&body], t);
        let mut l = Pixmap::new(pm.width(), pm.height()).unwrap();
        let top = rrect(0.9, 0.9, c.w - 1.8, c.h - 1.8, (c.r - 0.9).max(0.5));
        stroke(&mut l, &top, hex(0xFFFFFF, 170), 0.35, LineCap::Round, t);
        blur(&mut l, ((0.3 * kdev) / 2.0).max(1.0) as usize);
        let band = soft_mask(
            pm,
            &rrect(c.r * 0.6, -2.0, c.w - c.r * 1.2, 4.0, 0.0),
            t,
            kdev,
            3.0,
        );
        let mut m2 = band;
        m2.intersect_path(&body, FillRule::Winding, true, t);
        let _ = mask;
        pm.draw_pixmap(
            0,
            0,
            l.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            Some(&m2),
        );
    }
    rim_light(pm, pal, &[&body], t, kdev, 0.8);
    // lid gap: shadow under the lid lip, hairline seam, catch-light below
    let mut pb = PathBuilder::new();
    pb.move_to(-1.0, c.seam);
    pb.line_to(c.w + 1.0, c.seam);
    let seam = pb.finish().unwrap();
    let mask = mask_of(pm, &[&body], t);
    let mut layer = Pixmap::new(pm.width(), pm.height()).unwrap();
    shade(
        pm,
        &[&body],
        &rrect(-2.0, c.seam, c.w + 4.0, 2.2, 0.0),
        t,
        kdev,
        0.10,
        1.6,
    );
    stroke(
        &mut layer,
        &seam,
        hex(0x000000, if pal.dark { 120 } else { 80 }),
        0.32,
        LineCap::Butt,
        t,
    );
    stroke(
        &mut layer,
        &seam,
        hex(0xFFFFFF, 200),
        0.22,
        LineCap::Butt,
        t.pre_translate(0.0, 0.34),
    );
    pm.draw_pixmap(
        0,
        0,
        layer.as_ref(),
        &PixmapPaint::default(),
        Transform::identity(),
        Some(&mask),
    );
    // finger notch: a shallow pill-shaped recess across the lid seam, straight top and bottom edges,
    // fully rounded ends. Lit from the upper left: the upper inner wall faces down (shadow), the lower
    // inner wall faces up (catch-light). The seam keeps running through it.
    if c.notch {
        let (w, h) = (c.w * NOTCH_W_FRAC, NOTCH_H);
        let (x0, top) = (c.w / 2.0 - w / 2.0, c.seam - h * NOTCH_ABOVE);
        let (x1, r) = (x0 + w, h / 2.0);
        let k = 0.5523 * r;
        let pill = rrect(x0, top, w, h, r);
        // upper and lower rim, each from one end centre to the other over the top or the bottom
        let rim = |up: bool| {
            let (ye, sgn) = (top + r, if up { -1.0 } else { 1.0 });
            let mut pb = PathBuilder::new();
            pb.move_to(x0, ye);
            pb.cubic_to(
                x0,
                ye + sgn * k,
                x0 + r - k,
                ye + sgn * r,
                x0 + r,
                ye + sgn * r,
            );
            pb.line_to(x1 - r, ye + sgn * r);
            pb.cubic_to(x1 - r + k, ye + sgn * r, x1, ye + sgn * k, x1, ye);
            pb.finish().unwrap()
        };
        let (up, lo) = (rim(true), rim(false));
        let dk = if pal.dark { 1.0 } else { 0.8 };
        shade(pm, &[&pill], &pill, t, kdev, 0.015 * dk, 0.4);
        shade(
            pm,
            &[&pill],
            &rrect(x0 - 1.0, top - 1.0, w + 2.0, h * 0.35 + 1.0, 0.0),
            t,
            kdev,
            0.07 * dk,
            0.8,
        );
        let mut l = Pixmap::new(pm.width(), pm.height()).unwrap();
        stroke(
            &mut l,
            &up,
            hex(0x000000, if pal.dark { 60 } else { 42 }),
            0.12,
            LineCap::Round,
            t,
        );
        stroke(
            &mut l,
            &lo,
            hex(0xFFFFFF, 220),
            0.22,
            LineCap::Round,
            t.pre_translate(0.0, -0.18),
        );
        stroke(
            &mut l,
            &lo,
            hex(0x000000, 26),
            0.10,
            LineCap::Round,
            t.pre_translate(0.0, 0.05),
        );
        pm.draw_pixmap(
            0,
            0,
            l.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            None,
        );
    }
    // lanyard loop: stainless insert flush in the right flank, centred on the case height. Seen from
    // the front only its edge shows: a slightly recessed slot with a brushed steel bar.
    if c.lanyard {
        // Seen from the front only the edge of the side insert shows: a thin vertical steel sliver
        // (~10.5 mm, about 23 % of the case height), centred on the full case height, with the oval
        // slot of the loop as a dark slit in its middle.
        let ly = c.h * 0.5;
        let (sx, sw, sh) = (c.w - 1.55, 1.15, 10.5);
        let recess = rrect(sx, ly - sh / 2.0, sw, sh, sw / 2.0);
        shade(pm, &[&body], &recess, t, kdev, 0.50, 0.25);
        let (bx, bw, bh) = (sx + 0.2, sw - 0.36, sh - 0.6);
        let bar = rrect(bx, ly - bh / 2.0, bw, bh, bw / 2.0);
        fill_sh(
            pm,
            &bar,
            lin3(
                (bx, 0.0),
                (bx + bw, 0.0),
                hex(0x8A8A92, 255),
                hex(0xEDEDF0, 255),
                hex(0x5A5A62, 255),
                0.36,
            ),
            t,
        );
        // ends dive into the shell
        shade(
            pm,
            &[&bar],
            &rrect(bx - 1.0, ly - bh / 2.0 - 1.0, bw + 2.0, 1.7, 0.0),
            t,
            kdev,
            0.28,
            0.6,
        );
        shade(
            pm,
            &[&bar],
            &rrect(bx - 1.0, ly + bh / 2.0 - 0.7, bw + 2.0, 1.7, 0.0),
            t,
            kdev,
            0.28,
            0.6,
        );
        // oval slot of the loop
        let slit_h = sh * 0.52;
        let slit = rrect(
            bx + bw * 0.55,
            ly - slit_h / 2.0,
            bw * 0.30,
            slit_h,
            bw * 0.15,
        );
        fill(pm, &slit, hex(0x2A2A2E, 200), t);
        // brushed specular streak along the sliver, upper part
        glint(
            pm,
            &[&bar],
            &rrect(
                bx + bw * 0.22,
                ly - bh * 0.42,
                bw * 0.22,
                bh * 0.38,
                bw * 0.11,
            ),
            t,
            kdev,
            0.8,
            0.3,
        );
        stroke(pm, &recess, hex(0x000000, 28), 0.1, LineCap::Round, t);
    }
    // bottom edge: port and (if any) speaker holes, seen just above the lower rim
    let by = c.h - 1.05;
    let (pw, ph) = match c.port {
        Port::Lightning => (6.6, 0.9),
        Port::UsbC => (7.8, 1.25),
    };
    let port = rrect(c.w / 2.0 - pw / 2.0, by - ph / 2.0, pw, ph, ph / 2.0);
    fill(pm, &port, with_a(mesh_col(pal), 190), t);
    match c.port {
        // USB-C: centre tongue
        Port::UsbC => fill(
            pm,
            &rrect(c.w / 2.0 - pw * 0.32, by - 0.14, pw * 0.64, 0.28, 0.14),
            hex(0x6E6E74, 200),
            t,
        ),
        // Lightning: shallow slot, lit lower lip
        Port::Lightning => stroke(
            pm,
            &hline(c.w / 2.0 - pw * 0.4, c.w / 2.0 + pw * 0.4, by + ph * 0.32),
            hex(0x5A5A60, 200),
            0.12,
            LineCap::Round,
            t,
        ),
    }
    // chamfer catch-light under the opening
    stroke(
        pm,
        &hline(
            c.w / 2.0 - pw / 2.0 + ph * 0.4,
            c.w / 2.0 + pw / 2.0 - ph * 0.4,
            by + ph / 2.0 + 0.12,
        ),
        hex(0xFFFFFF, 150),
        0.12,
        LineCap::Round,
        t,
    );
    if c.speaker {
        for side in [-1.0f32, 1.0] {
            for i in 0..4 {
                let x = c.w / 2.0 + side * (pw / 2.0 + 1.8 + i as f32 * 1.35);
                fill(pm, &oval(x, by, 0.38, 0.38), with_a(mesh_col(pal), 165), t);
                fill(
                    pm,
                    &oval(x + 0.06, by + 0.12, 0.2, 0.14),
                    hex(0x8A8A90, 90),
                    t,
                );
            }
        }
    }
    // status LED
    if let Some(ly) = c.led {
        // LED window: hairline bezel
        stroke(
            pm,
            &oval(c.w / 2.0, ly, 1.25, 1.25),
            hex(0x000000, 16),
            0.14,
            LineCap::Round,
            t,
        );
        stroke(
            pm,
            &oval(c.w / 2.0, ly + 0.15, 1.3, 1.3),
            hex(0xFFFFFF, 90),
            0.1,
            LineCap::Round,
            t,
        );
        if charging {
            // calm status LED: small dot, faint soft halo, tiny hot centre
            let led = hex(LED_CORE, 255);
            let halo_a = if pal.dark {
                LED_HALO_A_DARK
            } else {
                LED_HALO_A_LIGHT
            };
            let mut hl = Pixmap::new(pm.width(), pm.height()).unwrap();
            fill(
                &mut hl,
                &oval(c.w / 2.0, ly, LED_HALO_R, LED_HALO_R),
                with_a(led, (halo_a * 255.0) as u8),
                t,
            );
            blur(&mut hl, ((LED_HALO_BLUR * kdev) / 2.0).max(1.0) as usize);
            pm.draw_pixmap(
                0,
                0,
                hl.as_ref(),
                &PixmapPaint::default(),
                Transform::identity(),
                None,
            );
            fill(pm, &oval(c.w / 2.0, ly, LED_R, LED_R), led, t);
            fill(
                pm,
                &oval(c.w / 2.0 - 0.12, ly - 0.14, LED_R * 0.38, LED_R * 0.38),
                hex(0xD8F5E0, 150),
                t,
            );
        } else {
            fill(pm, &oval(c.w / 2.0, ly, 0.95, 0.95), hex(0x000000, 40), t);
            fill(
                pm,
                &oval(c.w / 2.0, ly + 0.15, 0.8, 0.8),
                hex(0x8E8E93, 60),
                t,
            );
        }
    }
}

// AirPods Max (front view, mm: 168.6 x 187.3).

/// sel bits: 1 floor shadows, 2 the headphones.
pub fn draw_max(pm: &mut Pixmap, pal: &Pal, s: f32, cx: f32, floor: f32, k: f32, sel: u8) {
    let (w, h) = (168.6f32, 187.3f32);
    let ox = cx - w * k / 2.0;
    let oy = floor - h * k;
    let t = Transform::from_translate(ox, oy)
        .pre_scale(k, k)
        .post_scale(s, s);
    let kdev = k * s;
    if sel & 1 != 0 {
        floor_shadow(pm, pal, s, cx - (w / 2.0 - 24.0) * k, floor, 23.0 * k);
        floor_shadow(pm, pal, s, cx + (w / 2.0 - 24.0) * k, floor, 23.0 * k);
    }
    if sel & 2 == 0 {
        return;
    }

    let alu = if pal.dark {
        Mat {
            hi: hex(0xF4F4F6, 255),
            lo: hex(0x8A8A92, 255),
            edge: 9.0,
        }
    } else {
        Mat {
            hi: hex(0xF8F8FA, 255),
            lo: hex(0xA2A2AA, 255),
            edge: 9.0,
        }
    };
    let knit = if pal.dark {
        Mat {
            hi: hex(0xD9D9DE, 255),
            lo: hex(0x75757C, 255),
            edge: 7.0,
        }
    } else {
        Mat {
            hi: hex(0xDADADF, 255),
            lo: hex(0x8C8C94, 255),
            edge: 7.0,
        }
    };
    let steel = |x0: f32, x1: f32| {
        lin3(
            (x0, 0.0),
            (x1, 0.0),
            hex(0x8C8C94, 255),
            hex(0xF2F2F5, 255),
            hex(0x7A7A82, 255),
            0.42,
        )
    };

    // headband frame (stainless) and knit canopy
    let mut pb = PathBuilder::new();
    pb.move_to(14.5, 58.0);
    pb.cubic_to(14.5, 8.0, 40.0, 2.0, 84.3, 2.0);
    pb.cubic_to(128.6, 2.0, 154.1, 8.0, 154.1, 58.0);
    let frame = pb.finish().unwrap();
    let mut pb = PathBuilder::new();
    pb.move_to(21.0, 56.0);
    pb.cubic_to(23.0, 26.0, 46.0, 19.0, 84.3, 19.0);
    pb.cubic_to(122.6, 19.0, 145.6, 26.0, 147.6, 56.0);
    let canopy = pb.finish().unwrap();
    let line = pal.line;
    if line.alpha() > 0.0 {
        stroke(
            pm,
            &frame,
            line,
            3.6 + 1.2 * scale() / kdev,
            LineCap::Round,
            t,
        );
        stroke(
            pm,
            &canopy,
            line,
            11.0 + 1.2 * scale() / kdev,
            LineCap::Butt,
            t,
        );
    }
    stroke(pm, &canopy, knit.lo, 11.0, LineCap::Butt, t);
    stroke(
        pm,
        &canopy,
        knit.hi,
        6.0,
        LineCap::Butt,
        t.pre_translate(-0.3, -1.6),
    );
    // knit texture: two fine weave lines
    stroke(
        pm,
        &canopy,
        with_a(knit.lo, 110),
        0.6,
        LineCap::Butt,
        t.pre_translate(0.0, 2.6),
    );
    stroke(
        pm,
        &canopy,
        with_a(knit.lo, 70),
        0.5,
        LineCap::Butt,
        t.pre_translate(0.0, -2.0),
    );
    let mut st = Paint {
        anti_alias: true,
        ..Default::default()
    };
    st.shader = lin(
        (0.0, -1.0),
        (0.0, 8.0),
        hex(0xF4F4F7, 255),
        hex(0x8E8E96, 255),
    );
    pm.stroke_path(
        &frame,
        &st,
        &Stroke {
            width: 3.4,
            line_cap: LineCap::Round,
            ..Default::default()
        },
        t,
        None,
    );
    stroke(
        pm,
        &frame,
        hex(0xFFFFFF, 170),
        0.8,
        LineCap::Round,
        t.pre_translate(-0.4, -0.8),
    );

    for side in [0, 1] {
        // mirror helper for x
        let rx = |x0: f32, ww: f32| if side == 0 { x0 } else { w - x0 - ww };
        // telescoping arm
        let ax = rx(12.6, 3.8);
        let arm = rrect(ax, 54.0, 3.8, 42.0, 1.0);
        if line.alpha() > 0.0 {
            stroke(pm, &arm, line, 1.2 * scale() / kdev, LineCap::Round, t);
        }
        fill_sh(pm, &arm, steel(ax, ax + 3.8), t);
        // frame/canopy joint block
        let jx = rx(10.6, 7.8);
        let joint = rrect(jx, 50.0, 7.8, 9.0, 2.6);
        if line.alpha() > 0.0 {
            stroke(pm, &joint, line, 1.2 * scale() / kdev, LineCap::Round, t);
        }
        fill_sh(pm, &joint, steel(jx, jx + 7.8), t);
        // cushion (knit) behind the shell
        let cush = rrect(rx(25.0, 23.0), 100.0, 23.0, 85.0, 7.5);
        volume(
            pm,
            &[&cush],
            t,
            kdev,
            knit,
            ((0.0, 101.0), (0.0, 184.0)),
            line,
        );
        // inner face of the cushion (mesh towards the ear)
        let inner = rrect(rx(44.4, 3.4), 106.0, 3.4, 73.0, 1.7);
        fill(
            pm,
            &inner,
            with_a(hex(0x000000, 255), if pal.dark { 60 } else { 40 }),
            t,
        );
        // aluminium cup shell
        let cup = rrect(rx(0.0, 29.0), 96.0, 29.0, 91.3, 9.0);
        volume(pm, &[&cup], t, kdev, alu, ((0.0, 96.0), (0.0, 187.3)), line);
        glint(
            pm,
            &[&cup],
            &rrect(rx(5.5, 7.0), 104.0, 7.0, 70.0, 3.5),
            t,
            kdev,
            0.6,
            3.0,
        );
        // yoke on top of the cup
        let yx = rx(8.8, 11.4);
        let yoke = rrect(yx, 91.5, 11.4, 6.5, 2.4);
        if line.alpha() > 0.0 {
            stroke(pm, &yoke, line, 1.2 * scale() / kdev, LineCap::Round, t);
        }
        fill_sh(pm, &yoke, steel(yx, yx + 11.4), t);
    }
    // right cup: digital crown (outer edge) and noise control button (inner edge)
    let crx = w - 6.8;
    let crown = rrect(crx - 5.6, 85.0, 11.2, 11.0, 2.0);
    if line.alpha() > 0.0 {
        stroke(pm, &crown, line, 1.2 * scale() / kdev, LineCap::Round, t);
    }
    fill_sh(
        pm,
        &crown,
        lin3(
            (crx - 5.6, 0.0),
            (crx + 5.6, 0.0),
            hex(0x9A9AA2, 255),
            hex(0xF4F4F7, 255),
            hex(0x85858D, 255),
            0.4,
        ),
        t,
    );
    for i in 0..6 {
        let x = crx - 4.2 + i as f32 * 1.7;
        let mut pb = PathBuilder::new();
        pb.move_to(x, 86.6);
        pb.line_to(x, 94.6);
        stroke(
            pm,
            &pb.finish().unwrap(),
            hex(0x000000, 70),
            0.7,
            LineCap::Butt,
            t,
        );
    }
    let btn = rrect(w - 28.4, 92.6, 7.0, 4.2, 2.1);
    if line.alpha() > 0.0 {
        stroke(pm, &btn, line, 1.2 * scale() / kdev, LineCap::Round, t);
    }
    fill_sh(
        pm,
        &btn,
        lin(
            (0.0, 92.6),
            (0.0, 96.8),
            hex(0xF0F0F3, 255),
            hex(0x9A9AA2, 255),
        ),
        t,
    );
}

pub fn with_a(c: Color, a: u8) -> Color {
    let mut c = c;
    c.set_alpha(a as f32 / 255.0);
    c
}
