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

#[cfg(test)]
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
