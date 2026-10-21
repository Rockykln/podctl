//! The card in layers, so a frame of the entry moves the products
//! without drawing them again.

use tiny_skia::*;

use crate::anim::Art;
use crate::paint::{Pal, inked};
use crate::render::{self, BUF_H, BUF_W, Part, Snapshot};

/// How much lighter a floor shadow gets per pixel its object rises.
const SHADOW_FADE_PER_PX: f32 = 0.10;
/// The rear bud is drawn at 93 % and tilted 4°; it hops along that axis.
const REAR_SCALE: f32 = 0.93;
const REAR_TILT_DEG: f32 = 4.0;
/// The Max hops as one, lower than a bud.
const MAX_HOP: f32 = 0.7;

/// A layer cut down to where it has ink, placed at `x`, `y` (device px).
struct Sprite {
    pm: Pixmap,
    x: i32,
    y: i32,
}

impl Sprite {
    fn cut(pm: &Pixmap, area: ((usize, usize), (usize, usize))) -> Option<Sprite> {
        let ((x0, x1), (y0, y1)) = area;
        let r = IntRect::from_xywh(x0 as i32, y0 as i32, (x1 - x0) as u32, (y1 - y0) as u32)?;
        Some(Sprite {
            pm: pm.clone_rect(r)?,
            x: x0 as i32,
            y: y0 as i32,
        })
    }

    fn draw(&self, out: &mut Pixmap, dx: f32, dy: f32, opacity: f32) {
        let whole = dx.fract() == 0.0 && dy.fract() == 0.0;
        let paint = PixmapPaint {
            opacity,
            quality: if whole {
                FilterQuality::Nearest
            } else {
                FilterQuality::Bilinear
            },
            ..Default::default()
        };
        let t = Transform::from_translate(self.x as f32 + dx, self.y as f32 + dy);
        out.draw_pixmap(0, 0, self.pm.as_ref(), &paint, t, None);
    }
}

pub struct Layers {
    scale: f32,
    plate: Pixmap,
    parts: Vec<(Part, Sprite)>,
    /// The case's charging light: the lit case where it differs from the
    /// dark one, faded in over it.
    led: Option<Sprite>,
    readouts: Option<Sprite>,
}

fn blank(s: f32) -> Pixmap {
    let (w, h) = render::buf_px(BUF_W, BUF_H, s);
    Pixmap::new(w, h).expect("non-zero card")
}

fn drawn(snap: &Snapshot, pal: &Pal, s: f32, part: Part) -> Pixmap {
    let mut pm = blank(s);
    render::draw_part(&mut pm, snap, pal, s, part);
    pm
}

/// Columns and rows where two layers of one size differ.
fn differ(a: &Pixmap, b: &Pixmap) -> Option<((usize, usize), (usize, usize))> {
    let w = a.width() as usize;
    let (mut x0, mut x1, mut y0, mut y1) = (usize::MAX, 0, usize::MAX, 0);
    for (i, (p, q)) in a
        .data()
        .as_chunks::<4>()
        .0
        .iter()
        .zip(b.data().as_chunks::<4>().0)
        .enumerate()
    {
        if p != q {
            let (x, y) = (i % w, i / w);
            x0 = x0.min(x);
            x1 = x1.max(x + 1);
            y0 = y0.min(y);
            y1 = y1.max(y + 1);
        }
    }
    (x0 < x1).then_some(((x0, x1), (y0, y1)))
}

impl Layers {
    /// Each part on a thread of its own: one after the other they take
    /// longer than the entry waits for its first hop.
    pub fn build(snap: &Snapshot, pal: &Pal, s: f32) -> Layers {
        let mut jobs = Part::products(snap);
        if let Some(i) = jobs.iter().position(|p| *p == Part::Case { led: true }) {
            jobs[i] = Part::Case { led: false };
            jobs.push(Part::Case { led: true });
        }
        jobs.push(Part::Readouts);
        let (plate, drawn) = std::thread::scope(|sc| {
            let plate = sc.spawn(|| render::render_plate(snap, pal, s));
            let parts: Vec<_> = jobs
                .iter()
                .map(|&p| sc.spawn(move || (p, drawn(snap, pal, s, p))))
                .collect();
            let drawn: Vec<(Part, Pixmap)> = parts
                .into_iter()
                .map(|h| h.join().expect("layer thread"))
                .collect();
            (plate.join().expect("plate thread"), drawn)
        });

        let lit = drawn.iter().find(|(p, _)| *p == Part::Case { led: true });
        let dark = drawn.iter().find(|(p, _)| *p == Part::Case { led: false });
        let led = match (lit, dark) {
            (Some((_, on)), Some((_, off))) => differ(on, off).and_then(|a| Sprite::cut(on, a)),
            _ => None,
        };
        let cut = |pm: &Pixmap| inked(pm).and_then(|a| Sprite::cut(pm, a));
        let mut parts = Vec::new();
        let mut readouts = None;
        for (p, pm) in &drawn {
            match p {
                Part::Case { led: true } => {}
                Part::Readouts => readouts = cut(pm),
                _ => parts.extend(cut(pm).map(|sp| (*p, sp))),
            }
        }
        Layers {
            scale: s,
            plate,
            parts,
            led,
            readouts,
        }
    }

    /// The card with the products where `art` has them.
    pub fn compose(&self, art: Art) -> Pixmap {
        let s = self.scale;
        let mut out = self.plate.clone();
        let (sin, cos) = REAR_TILT_DEG.to_radians().sin_cos();
        let max = MAX_HOP * art.left;
        let lighter = |h: f32| (1.0 - SHADOW_FADE_PER_PX * h).clamp(0.0, 1.0);
        for (part, sp) in &self.parts {
            match part {
                Part::BudL => sp.draw(&mut out, 0.0, -art.left * s, 1.0),
                Part::BudR => {
                    let h = REAR_SCALE * art.right * s;
                    sp.draw(&mut out, h * sin, -h * cos, 1.0)
                }
                Part::ShadowL => sp.draw(&mut out, 0.0, 0.0, lighter(art.left)),
                Part::ShadowR => sp.draw(&mut out, 0.0, 0.0, lighter(REAR_SCALE * art.right)),
                Part::MaxBody => sp.draw(&mut out, 0.0, -max * s, 1.0),
                Part::MaxShadow => sp.draw(&mut out, 0.0, 0.0, lighter(max)),
                _ => {
                    sp.draw(&mut out, 0.0, 0.0, 1.0);
                    if let (Part::Case { .. }, Some(led)) = (part, &self.led) {
                        led.draw(&mut out, 0.0, 0.0, art.led);
                    }
                }
            }
        }
        if let Some(ro) = &self.readouts {
            ro.draw(&mut out, 0.0, 0.0, 1.0);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::art::Art as Model;
    use crate::paint::{dark, light};

    /// At rest the layers add up to the card drawn in one go; only the
    /// antialiasing of separately blended edges may differ.
    #[test]
    fn layers_at_rest_are_the_card() {
        for (model, pal, scale) in [
            (Model::Pro2C, dark(), 1.0),
            (Model::Pro2C, light(), 1.75),
            (Model::Gen3, dark(), 2.0),
            (Model::MaxL, dark(), 1.25),
        ] {
            let snap = Snapshot::sample_for(model);
            let want = render::render(&snap, &pal, scale);
            let got = Layers::build(&snap, &pal, scale).compose(Art::REST);
            let mut off = 0usize;
            for (a, b) in got.data().chunks(4).zip(want.data().chunks(4)) {
                if a.iter().zip(b).any(|(x, y)| x.abs_diff(*y) > 5) {
                    off += 1;
                }
            }
            let share = off as f64 / (got.width() * got.height()) as f64;
            assert!(share < 0.005, "{model:?} @{scale}: {:.3} %", share * 100.0);
        }
    }

    #[test]
    fn a_charging_case_has_its_light_apart() {
        let mut snap = Snapshot::sample_for(Model::Pro2C);
        snap.case.charging = true;
        let l = Layers::build(&snap, &dark(), 2.0);
        let led = l.led.as_ref().expect("lit case differs from the dark one");
        assert!(led.pm.width() < 60 && led.pm.height() < 60);
        snap.case.charging = false;
        assert!(Layers::build(&snap, &dark(), 2.0).led.is_none());
    }
}
