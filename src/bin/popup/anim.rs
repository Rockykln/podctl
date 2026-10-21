//! How the card moves, as a function of the time since a phase began.

use std::time::Duration;

/// Fallback frame pace where the server sends no frame callbacks.
pub const FRAME: Duration = Duration::from_millis(16);

/// The entry rises this far, logical px. The buffer keeps the room below
/// the card, so the window itself never moves.
pub const RISE: f32 = 20.0;
const SINK: f32 = 10.0;

const FADE_IN_MS: f32 = 180.0;
const SPRING_ZETA: f32 = 0.86;
const SPRING_RESPONSE_S: f32 = 0.42;
/// The spring is within a hundredth of a pixel by 700 ms; the right
/// bud's hop ends at 740.
pub const ENTRY_MS: f32 = 760.0;
pub const EXIT_MS: f32 = 220.0;
pub const DISMISS_MS: f32 = 150.0;
const DISMISS_SCALE: f32 = 0.97;

const HOP_PX: f32 = 3.5;
const HOP_MS: f32 = 560.0;
const HOP_L_FROM: f32 = 110.0;
const HOP_R_FROM: f32 = 180.0;
const LED_FROM: f32 = 260.0;
const LED_MS: f32 = 320.0;

/// The products during the entry: how high each bud hops (logical px)
/// and how far the case's charging light has come on.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Art {
    pub left: f32,
    pub right: f32,
    pub led: f32,
}

impl Art {
    pub const REST: Art = Art {
        left: 0.0,
        right: 0.0,
        led: 1.0,
    };
}

pub fn ease_out_cubic(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

/// Up and down again, a little lower towards the end.
fn hop(ms: f32) -> f32 {
    if ms <= 0.0 || ms >= HOP_MS {
        return 0.0;
    }
    let u = ms / HOP_MS;
    HOP_PX * (std::f32::consts::PI * ease_out_cubic(u)).sin() * (1.0 - 0.25 * u)
}

pub fn art(ms: f32) -> Art {
    Art {
        left: hop(ms - HOP_L_FROM),
        right: hop(ms - HOP_R_FROM),
        led: ease_out_cubic((ms - LED_FROM) / LED_MS),
    }
}

/// With the values there when the card maps, the rings start this late.
pub const RINGS_AFTER_MAP_MS: f32 = 120.0;
const RING_MS: f32 = 700.0;
const RING_STAGGER_MS: f32 = 70.0;
const SWAP_MS: f32 = 260.0;
const BOLT_FROM_MS: f32 = 500.0;
/// The bolt comes in at its full size in the room the row kept for it,
/// in four steps of 100 ms. The design's spring overshot and moved the
/// row, and a smooth fade looked unsteady.
const BOLT_STEPS: f32 = 4.0;
const BOLT_STEP_MS: f32 = 100.0;
const BOLT_MS: f32 = BOLT_STEPS * BOLT_STEP_MS;
const PULSE_MS: f32 = 480.0;
pub const STROKE: f32 = 2.6;
const PULSE_PX: f32 = 1.0;
const DRIFT_PX: f32 = 3.0;

/// One readout while its value comes in: how much of the ring is drawn,
/// how far the "—" has given way to the number (0 to 1), how far the
/// bolt has come in, and the ring's stroke.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Gauge {
    pub fill: f32,
    pub text: f32,
    pub bolt: f32,
    pub stroke: f32,
}

impl Gauge {
    pub const REST: Gauge = Gauge {
        fill: 1.0,
        text: 1.0,
        bolt: 1.0,
        stroke: STROKE,
    };
}

/// Readout `i` (left to right) `ms` after the values arrived. A low one
/// pulses once after it has filled; nothing blinks after that.
pub fn gauge(ms: f32, i: usize, low: bool) -> Gauge {
    let t = ms - i as f32 * RING_STAGGER_MS;
    let pulse = (t - RING_MS) / PULSE_MS;
    Gauge {
        fill: ease_out_quint(t / RING_MS),
        text: (t / SWAP_MS).clamp(0.0, 1.0),
        bolt: ((((t - BOLT_FROM_MS) / BOLT_STEP_MS).floor() + 1.0) / BOLT_STEPS).clamp(0.0, 1.0),
        stroke: if low && (0.0..1.0).contains(&pulse) {
            STROKE + PULSE_PX * (std::f32::consts::PI * pulse).sin()
        } else {
            STROKE
        },
    }
}

/// Whether `n` readouts are still moving `ms` after the values arrived.
pub fn gauges_moving(ms: f32, n: usize) -> bool {
    let last = (n.max(1) - 1) as f32 * RING_STAGGER_MS;
    ms < last + (RING_MS + PULSE_MS).max(BOLT_FROM_MS + BOLT_MS)
}

/// The text "—" or the number, `u` into the swap: how visible it is and
/// how far below its place. The dash goes in the first 30 %, the number
/// comes up 3 px in the rest; the two are never both legible.
pub fn swap_text(u: f32, number: bool) -> (f32, f32) {
    if number {
        let v = ((u - 0.3) / 0.7).clamp(0.0, 1.0);
        (v, DRIFT_PX * (1.0 - ease_out_cubic(v)))
    } else {
        (1.0 - (u / 0.3).clamp(0.0, 1.0), 0.0)
    }
}

const STATUS_MS: f32 = 260.0;

/// The status line changing `ms` in: the old text drifts up and out in
/// the first 40 %, the new one comes up from below in the rest. Returns
/// (old alpha, old dy, new alpha, new dy); `None` once it is over.
pub fn status_swap(ms: f32) -> Option<(f32, f32, f32, f32)> {
    if ms >= STATUS_MS {
        return None;
    }
    let u = (ms / STATUS_MS).max(0.0);
    let out = (u / 0.4).clamp(0.0, 1.0);
    let inn = ((u - 0.4) / 0.6).clamp(0.0, 1.0);
    Some((
        1.0 - out,
        -DRIFT_PX * ease_out_cubic(out),
        inn,
        DRIFT_PX * (1.0 - ease_out_cubic(inn)),
    ))
}

/// Where the card is: offset down, opacity, and scale about its centre.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Pose {
    pub dy: f32,
    pub alpha: f32,
    pub scale: f32,
}

impl Pose {
    pub const REST: Pose = Pose {
        dy: 0.0,
        alpha: 1.0,
        scale: 1.0,
    };
}

pub fn ease_out_quint(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(5)
}

pub fn ease_in_cubic(t: f32) -> f32 {
    t.clamp(0.0, 1.0).powi(3)
}

/// Step response of a damped oscillator, as SwiftUI's
/// `.spring(response:dampingFraction:)`: 0 at t = 0, settling at 1.
pub fn spring(t_s: f32, zeta: f32, response_s: f32) -> f32 {
    if t_s <= 0.0 {
        return 0.0;
    }
    let w0 = std::f32::consts::TAU / response_s;
    if zeta >= 1.0 {
        return 1.0 - (-w0 * t_s).exp() * (1.0 + w0 * t_s);
    }
    let wd = w0 * (1.0 - zeta * zeta).sqrt();
    1.0 - (-zeta * w0 * t_s).exp() * ((wd * t_s).cos() + zeta * w0 / wd * (wd * t_s).sin())
}

/// The card rising into place. `None` once it rests.
pub fn entry(ms: f32) -> Option<Pose> {
    (ms < ENTRY_MS).then(|| Pose {
        dy: RISE * (1.0 - spring(ms / 1000.0, SPRING_ZETA, SPRING_RESPONSE_S)),
        alpha: ease_out_quint(ms / FADE_IN_MS),
        scale: 1.0,
    })
}

/// Leaving after the hold: down and out, starting from `from`.
pub fn exit(ms: f32, from: Pose) -> Option<Pose> {
    (ms < EXIT_MS).then(|| {
        let e = ease_in_cubic(ms / EXIT_MS);
        Pose {
            dy: from.dy + SINK * e,
            alpha: from.alpha * (1.0 - e),
            scale: from.scale,
        }
    })
}

/// Closed by a click: a little smaller and out.
pub fn dismiss(ms: f32, from: Pose) -> Option<Pose> {
    (ms < DISMISS_MS).then(|| {
        let e = ease_in_cubic(ms / DISMISS_MS);
        Pose {
            dy: from.dy,
            alpha: from.alpha * (1.0 - e),
            scale: from.scale + (DISMISS_SCALE - from.scale) * e,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curves_start_and_end_in_place() {
        assert_eq!(ease_out_quint(0.0), 0.0);
        assert_eq!(ease_out_quint(1.0), 1.0);
        assert_eq!(ease_in_cubic(2.0), 1.0);
        assert_eq!(spring(0.0, 0.86, 0.42), 0.0);
        assert!((spring(2.0, 0.86, 0.42) - 1.0).abs() < 1e-4);
        assert!((spring(2.0, 1.0, 0.5) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn the_entry_spring_barely_overshoots() {
        let most = (0..700)
            .map(|ms| spring(ms as f32 / 1000.0, SPRING_ZETA, SPRING_RESPONSE_S))
            .fold(0.0f32, f32::max);
        // The design: 0.5 % overshoot, 0.1 px over a 20 px rise.
        assert!(most > 1.0 && most < 1.006, "{most}");
    }

    #[test]
    fn the_entry_settles_within_its_time() {
        let p = entry(0.0).unwrap();
        assert_eq!((p.dy, p.alpha), (RISE, 0.0));
        let late = entry(ENTRY_MS - 1.0).unwrap();
        assert!(late.dy.abs() < 0.01 && late.alpha == 1.0, "{late:?}");
        // 2 % of the rise is left after about 310 ms.
        assert!(entry(330.0).unwrap().dy.abs() < 0.02 * RISE);
        assert!(entry(ENTRY_MS).is_none());
    }

    #[test]
    fn the_buds_hop_once_in_turn() {
        assert_eq!(
            art(0.0),
            Art {
                left: 0.0,
                right: 0.0,
                led: 0.0
            }
        );
        let top = (0..800)
            .map(|ms| art(ms as f32).left)
            .fold(0.0f32, f32::max);
        assert!(top > 3.0 && top <= HOP_PX, "{top}");
        assert!(art(150.0).left > 0.0 && art(150.0).right == 0.0);
        assert_eq!(art(ENTRY_MS), Art::REST);
        assert!(art(400.0).led > 0.0 && art(400.0).led < 1.0);
    }

    #[test]
    fn rings_fill_in_turn_and_a_low_one_pulses_once() {
        let g = |ms, i| gauge(ms, i, false);
        assert_eq!(g(0.0, 0).fill, 0.0);
        assert!(g(100.0, 0).fill > 0.0 && g(60.0, 1).fill == 0.0);
        assert!((g(RING_MS, 0).fill - 1.0).abs() < 1e-6);
        assert!(g(0.0, 0).bolt == 0.0 && g(2000.0, 0).bolt == 1.0);
        let most = (0..2000)
            .map(|ms| g(ms as f32, 0).bolt)
            .fold(0.0f32, f32::max);
        assert_eq!(most, 1.0, "the bolt never grows past its size");
        let steps: std::collections::BTreeSet<u32> = (0..2000)
            .map(|ms| (g(ms as f32, 0).bolt * 100.0) as u32)
            .collect();
        assert_eq!(steps.into_iter().collect::<Vec<_>>(), [0, 25, 50, 75, 100]);
        let widest = (0..2000)
            .map(|ms| gauge(ms as f32, 0, true).stroke)
            .fold(0.0f32, f32::max);
        assert!((widest - STROKE - PULSE_PX).abs() < 0.01);
        assert_eq!(gauge(1500.0, 0, true).stroke, STROKE);
        assert_eq!(gauge(900.0, 0, false).stroke, STROKE);
        assert!(gauges_moving(1000.0, 3) && !gauges_moving(1400.0, 1));
        assert_eq!(gauge(1400.0, 0, true), Gauge::REST);
    }

    #[test]
    fn two_texts_are_never_both_legible() {
        for i in 0..=100 {
            let u = i as f32 / 100.0;
            let (dash, _) = swap_text(u, false);
            let (num, _) = swap_text(u, true);
            assert!(dash == 0.0 || num == 0.0, "{u}");
            if let Some((a, _, b, _)) = status_swap(u * 260.0) {
                assert!(a == 0.0 || b == 0.0, "{u}");
            }
        }
        assert_eq!(swap_text(1.0, true), (1.0, 0.0));
        assert!(status_swap(260.0).is_none());
    }

    #[test]
    fn exits_end_invisible() {
        let e = exit(EXIT_MS - 0.01, Pose::REST).unwrap();
        assert!(e.alpha < 1e-3 && (e.dy - SINK).abs() < 1e-2);
        let d = dismiss(DISMISS_MS - 0.01, Pose::REST).unwrap();
        assert!(d.alpha < 1e-3 && (d.scale - DISMISS_SCALE).abs() < 1e-3);
        assert!(exit(EXIT_MS, Pose::REST).is_none());
        // Cut short mid-entry, the exit leaves from where the card was.
        let mid = entry(60.0).unwrap();
        assert_eq!(exit(0.0, mid), Some(mid));
    }
}
