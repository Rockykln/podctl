//! One table of every model: product codes, name, Apple's A-numbers,
//! the drawing in the bubble and what the model can do. Nothing about a
//! model is written down anywhere else.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Model {
    Gen1,
    Gen2,
    Gen3,
    Gen4,
    Gen4Anc,
    Gen5,
    Gen5Wireless,
    Pro,
    Pro2,
    Pro2UsbC,
    Pro3,
    Max,
    MaxUsbC,
    Max2,
    Unknown,
}

impl Model {
    /// Every model with a row, in table order. `Unknown` is not one of
    /// them: it is the absence of a row, and a row for it would make
    /// `refine` start working on it.
    pub const ALL: [Model; 14] = [
        Model::Gen1,
        Model::Gen2,
        Model::Gen3,
        Model::Gen4,
        Model::Gen4Anc,
        Model::Gen5,
        Model::Gen5Wireless,
        Model::Pro,
        Model::Pro2,
        Model::Pro2UsbC,
        Model::Pro3,
        Model::Max,
        Model::MaxUsbC,
        Model::Max2,
    ];
}

/// Everything known about one model.
pub struct ModelSpec {
    pub model: Model,
    pub label: &'static str,
    /// Bluetooth product codes (the Modalias `p` field) for this model.
    /// Two variants can share one, which is what `refine` is for.
    pub pids: &'static [u16],
    /// Apple's A-numbers as the buds report them in the 0x1D device
    /// info. Only needed where they tell variants apart.
    pub model_numbers: &'static [&'static str],
    /// Drawing in the bubble, by slug — see `popup/art.rs`.
    pub art: &'static str,
    /// Checked against the hardware. `podctl debug` says so for the
    /// models where nobody has confirmed the capability row yet.
    pub verified: bool,
    /// Capabilities without the `model` field; read them through
    /// `caps()`, which fills it in.
    pub flags: Capabilities,
}

impl ModelSpec {
    pub fn caps(&self) -> Capabilities {
        Capabilities {
            model: self.model,
            ..self.flags
        }
    }
}

/// Nothing at all, so each entry below lists exactly what its model has.
const NONE: Capabilities = Capabilities {
    model: Model::Unknown,
    has_anc: false,
    has_transparency: false,
    has_adaptive: false,
    has_conv_awareness: false,
    has_case_battery: false,
    has_in_ear_detection: false,
    has_ear_detection_setting: false,
    has_loud_sound_reduction: false,
    has_spatial_audio: false,
    has_press_volume: false,
    has_press_and_hold: false,
    has_mic_selection: false,
    has_rename: false,
    has_tone_on_press: false,
};

/// Every bud reports a case battery, has in-ear detection and takes a
/// new name. Spatial audio and the rest are per model.
const BUDS: Capabilities = Capabilities {
    has_case_battery: true,
    has_in_ear_detection: true,
    has_ear_detection_setting: true,
    has_mic_selection: true,
    has_rename: true,
    ..NONE
};

/// Noise control, as the AirPods 4 ANC have it.
const ANC_BUDS: Capabilities = Capabilities {
    has_anc: true,
    has_transparency: true,
    has_adaptive: true,
    has_conv_awareness: true,
    has_spatial_audio: true,
    has_press_and_hold: true,
    has_tone_on_press: true,
    ..BUDS
};

/// Product codes: 0x2024 (Pro 2 USB-C) is verified against the owner's
/// hardware. The rest come from The Apple Wiki's Bluetooth PID page and
/// OpenPods' proximity-pair table, and 0x2030 / 0x2036 (AirPods 5) from
/// AirPodsDesktop — read off their code, not copied from it.
pub const MODELS: &[ModelSpec] = &[
    ModelSpec {
        model: Model::Gen1,
        label: "AirPods (1st gen)",
        pids: &[0x2002],
        model_numbers: &[],
        art: "airpods1",
        verified: false,
        flags: BUDS,
    },
    ModelSpec {
        model: Model::Gen2,
        label: "AirPods (2nd gen)",
        pids: &[0x200F],
        model_numbers: &[],
        art: "airpods2",
        verified: false,
        flags: BUDS,
    },
    ModelSpec {
        model: Model::Gen3,
        label: "AirPods (3rd gen)",
        pids: &[0x2013],
        model_numbers: &[],
        art: "airpods3",
        verified: false,
        flags: Capabilities {
            has_spatial_audio: true,
            ..BUDS
        },
    },
    ModelSpec {
        model: Model::Gen4,
        label: "AirPods 4",
        pids: &[0x2019],
        model_numbers: &[],
        art: "airpods4",
        verified: false,
        flags: Capabilities {
            has_spatial_audio: true,
            ..BUDS
        },
    },
    ModelSpec {
        model: Model::Gen4Anc,
        label: "AirPods 4 (ANC)",
        pids: &[0x201B],
        model_numbers: &[],
        art: "airpods4-anc",
        verified: false,
        flags: Capabilities {
            has_loud_sound_reduction: true,
            ..ANC_BUDS
        },
    },
    // The two AirPods 5 share their product codes; the A-number from
    // 0x1D says which case is in the box. Apple names noise control,
    // adaptive audio and conversation awareness, and does not name
    // loud-sound reduction — so that stays off until a capture says
    // otherwise. Volume swipes are the wireless case's alone.
    ModelSpec {
        model: Model::Gen5,
        label: "AirPods 5",
        pids: &[0x2030, 0x2036],
        model_numbers: &["A3531", "A3532", "A3533"],
        art: "airpods5",
        verified: false,
        flags: ANC_BUDS,
    },
    ModelSpec {
        model: Model::Gen5Wireless,
        label: "AirPods 5 with Wireless Charging Case",
        pids: &[0x2030, 0x2036],
        model_numbers: &["A3439", "A3440", "A3441"],
        art: "airpods5-wireless",
        verified: false,
        flags: Capabilities {
            has_press_volume: true,
            ..ANC_BUDS
        },
    },
    ModelSpec {
        model: Model::Pro,
        label: "AirPods Pro",
        pids: &[0x200E],
        model_numbers: &[],
        art: "pro1",
        verified: false,
        flags: Capabilities {
            has_anc: true,
            has_transparency: true,
            has_spatial_audio: true,
            has_press_and_hold: true,
            has_tone_on_press: true,
            ..BUDS
        },
    },
    ModelSpec {
        model: Model::Pro2,
        label: "AirPods Pro (2nd gen)",
        pids: &[0x2014],
        model_numbers: &[],
        art: "pro2-lightning",
        verified: false,
        flags: Capabilities {
            has_loud_sound_reduction: true,
            has_press_volume: true,
            ..ANC_BUDS
        },
    },
    ModelSpec {
        model: Model::Pro2UsbC,
        label: "AirPods Pro (2nd gen, USB-C)",
        pids: &[0x2024],
        model_numbers: &["A3047", "A3048", "A2968"],
        art: "pro2-usbc",
        verified: true,
        flags: Capabilities {
            has_loud_sound_reduction: true,
            has_press_volume: true,
            ..ANC_BUDS
        },
    },
    // Product code and both A-numbers read off a connected pair on
    // 27.11.2026. Set and read back on the device: all four listening
    // modes, conversation awareness, one-bud ANC, the auto-ANC level,
    // the chime volume, the microphone choice and the ear detection
    // setting. Loud-sound reduction, spatial audio, the press actions
    // and the press tone are the ones podctl cannot write at all yet,
    // so the row stays unverified.
    ModelSpec {
        model: Model::Pro3,
        label: "AirPods Pro (3rd gen)",
        pids: &[0x2027],
        model_numbers: &["A3063", "A3064"],
        art: "pro3",
        verified: false,
        flags: Capabilities {
            has_loud_sound_reduction: true,
            has_press_volume: true,
            ..ANC_BUDS
        },
    },
    // Over-ear: one battery, no case, no microphone choice, and head
    // detection rather than in-ear detection — which podctl does not
    // read yet, so it stays off.
    ModelSpec {
        model: Model::Max,
        label: "AirPods Max",
        pids: &[0x200A],
        model_numbers: &[],
        art: "max-lightning",
        verified: false,
        flags: Capabilities {
            has_anc: true,
            has_transparency: true,
            has_spatial_audio: true,
            has_press_and_hold: true,
            has_rename: true,
            ..NONE
        },
    },
    ModelSpec {
        model: Model::MaxUsbC,
        label: "AirPods Max (USB-C)",
        pids: &[0x201F],
        model_numbers: &[],
        art: "max-usbc",
        verified: false,
        flags: Capabilities {
            has_anc: true,
            has_transparency: true,
            has_spatial_audio: true,
            has_press_and_hold: true,
            has_rename: true,
            ..NONE
        },
    },
    // The H2 generation adds conversation awareness and loud-sound
    // reduction over the first Max.
    ModelSpec {
        model: Model::Max2,
        label: "AirPods Max (2nd gen)",
        pids: &[0x202D],
        model_numbers: &[],
        art: "max2",
        verified: false,
        flags: Capabilities {
            has_anc: true,
            has_transparency: true,
            has_conv_awareness: true,
            has_loud_sound_reduction: true,
            has_spatial_audio: true,
            has_press_and_hold: true,
            has_rename: true,
            ..NONE
        },
    },
];

impl Model {
    pub fn spec(self) -> Option<&'static ModelSpec> {
        MODELS.iter().find(|s| s.model == self)
    }

    /// Bluetooth product code → model. An unknown code still detects as
    /// AirPods through the BlueZ icon and gets the conservative row, so
    /// the buds show up; only model-specific features are gated.
    pub fn from_code(code: u16) -> Self {
        MODELS
            .iter()
            .find(|s| s.pids.contains(&code))
            .map(|s| s.model)
            .unwrap_or(Model::Unknown)
    }

    /// One product code can cover two variants. The A-number the buds
    /// report in 0x1D tells them apart; anything else is left alone.
    pub fn refine(self, model_numbers: &[String]) -> Self {
        let Some(mine) = self.spec() else {
            return self;
        };
        let mut hit: Option<Model> = None;
        for number in model_numbers {
            let number = number.trim();
            let found = MODELS.iter().find(|s| {
                s.model_numbers.contains(&number) && s.pids.iter().any(|p| mine.pids.contains(p))
            });
            if let Some(other) = found {
                match hit {
                    // Numbers of both variants in one list: that list
                    // belongs to two different pairs, and picking from
                    // it would hand one of them the other's row. The
                    // alphabetical order the daemon keeps them in made
                    // that choice systematic rather than random.
                    Some(first) if first != other.model => return self,
                    _ => hit = Some(other.model),
                }
            }
        }
        hit.unwrap_or(self)
    }

    pub fn label(self) -> &'static str {
        self.spec()
            .map(|s| s.label)
            .unwrap_or("AirPods (unknown model)")
    }

    /// Whether the capability row below was checked against hardware.
    pub fn verified(self) -> bool {
        self.spec().is_some_and(|s| s.verified)
    }

    /// Slug of the drawing in the bubble.
    pub fn art(self) -> &'static str {
        self.spec().map(|s| s.art).unwrap_or("pro2-usbc")
    }

    pub fn capabilities(self) -> Capabilities {
        self.spec()
            .map(|s| s.caps())
            .unwrap_or_else(|| Capabilities::conservative(self))
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Capabilities {
    pub model: Model,
    pub has_anc: bool,
    pub has_transparency: bool,
    pub has_adaptive: bool,
    pub has_conv_awareness: bool,
    pub has_case_battery: bool,
    pub has_in_ear_detection: bool,
    pub has_ear_detection_setting: bool,
    pub has_loud_sound_reduction: bool,
    pub has_spatial_audio: bool,
    pub has_press_volume: bool,
    pub has_press_and_hold: bool,
    pub has_mic_selection: bool,
    pub has_rename: bool,
    pub has_tone_on_press: bool,
}

impl Capabilities {
    /// Turn on what a reported setting proves the device has. A product
    /// code nobody has written down still answers its settings dump, and
    /// refusing every command on such a pair helps no one. Returns
    /// whether anything changed.
    pub fn learn_from_setting(&mut self, id: u8, value: u8) -> bool {
        use crate::aap::setting;
        let before = *self;
        match id {
            setting::NOISE_CONTROL => {
                self.has_anc = true;
                self.has_transparency = true;
                // The device picked adaptive itself, so it has it.
                if crate::aap::wire::mode_from_byte(value) == Some(crate::model::Mode::Adaptive) {
                    self.has_adaptive = true;
                }
            }
            setting::CONV_AWARE => self.has_conv_awareness = true,
            setting::AUTO_ANC_LEVEL => {
                // The slider only exists on buds that cancel noise and
                // let it through. Learning adaptive alone let someone
                // switch to it and then be told that Off "is not
                // supported on AirPods (unknown model)".
                self.has_adaptive = true;
                self.has_anc = true;
                self.has_transparency = true;
            }
            // Keeping ANC with one bud in is an ANC setting; a device
            // that reports it has the two modes it switches between.
            setting::ONE_BUD_ANC => {
                self.has_anc = true;
                self.has_transparency = true;
            }
            setting::EAR_DETECTION => {
                self.has_in_ear_detection = true;
                self.has_ear_detection_setting = true;
            }
            setting::MIC_MODE => self.has_mic_selection = true,
            _ => {}
        }
        *self != before
    }

    /// The device said which bud sits in an ear, so it has in-ear
    /// detection whatever its product code is. Settings dumps do not
    /// always carry 0x0a — the AirPods Pro 3 leave it out — so this is
    /// the only proof for an unknown model.
    pub fn learn_in_ear(&mut self) -> bool {
        !std::mem::replace(&mut self.has_in_ear_detection, true)
    }

    /// What any pair of AirPods does: report a case battery and take a
    /// new name. Used for a product code nobody has written down yet.
    pub fn conservative(model: Model) -> Self {
        Self {
            model,
            has_case_battery: true,
            has_rename: true,
            ..NONE
        }
    }
}

impl Default for Capabilities {
    fn default() -> Self {
        Capabilities::conservative(Model::Unknown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_product_codes_resolve_to_their_model() {
        assert_eq!(Model::from_code(0x2002), Model::Gen1);
        assert_eq!(Model::from_code(0x200F), Model::Gen2);
        assert_eq!(Model::from_code(0x2013), Model::Gen3);
        assert_eq!(Model::from_code(0x2019), Model::Gen4);
        assert_eq!(Model::from_code(0x200E), Model::Pro);
        assert_eq!(Model::from_code(0x2014), Model::Pro2);
        assert_eq!(Model::from_code(0x2024), Model::Pro2UsbC);
        assert_eq!(Model::from_code(0x2027), Model::Pro3);
        assert_eq!(Model::from_code(0x201B), Model::Gen4Anc);
        assert_eq!(Model::from_code(0x200A), Model::Max);
        assert_eq!(Model::from_code(0x201F), Model::MaxUsbC);
        assert_eq!(Model::from_code(0x202D), Model::Max2);
    }

    #[test]
    fn beats_pids_are_not_misread_as_airpods() {
        // 0x2025 is Beats Solo 4, 0x2026 is Beats Solo Buds. They were
        // previously (and wrongly) mapped to AirPods 4 / 4 ANC.
        assert_eq!(Model::from_code(0x2025), Model::Unknown);
        assert_eq!(Model::from_code(0x2026), Model::Unknown);
    }

    #[test]
    fn both_airpods_5_codes_land_on_the_usb_c_row_first() {
        // Nothing but the A-number tells the two apart, so the variant
        // without the extras is the safer default.
        assert_eq!(Model::from_code(0x2030), Model::Gen5);
        assert_eq!(Model::from_code(0x2036), Model::Gen5);
        assert!(!Model::Gen5.capabilities().has_press_volume);
    }

    #[test]
    fn an_a_number_tells_the_airpods_5_cases_apart() {
        let wireless = vec!["A3440".to_string()];
        assert_eq!(Model::Gen5.refine(&wireless), Model::Gen5Wireless);
        assert!(Model::Gen5Wireless.capabilities().has_press_volume);
        let usbc = vec!["A3532".to_string()];
        assert_eq!(Model::Gen5Wireless.refine(&usbc), Model::Gen5);
        // Whitespace from the wire does not hide the match.
        assert_eq!(
            Model::Gen5.refine(&[" A3439 ".to_string()]),
            Model::Gen5Wireless
        );
    }

    #[test]
    fn an_a_number_never_switches_to_a_different_product() {
        // A3047 belongs to the Pro 2 USB-C, which has its own code.
        let pro = vec!["A3047".to_string()];
        assert_eq!(Model::Gen5.refine(&pro), Model::Gen5);
        assert_eq!(Model::Pro3.refine(&pro), Model::Pro3);
        assert_eq!(Model::Unknown.refine(&pro), Model::Unknown);
        assert_eq!(Model::Gen5.refine(&[]), Model::Gen5);
    }

    #[test]
    fn pro3_inherits_pro2_capabilities() {
        let caps = Model::Pro3.capabilities();
        assert!(caps.has_anc);
        assert!(caps.has_adaptive);
        assert!(caps.has_conv_awareness);
        assert!(caps.has_loud_sound_reduction);
    }

    #[test]
    fn the_over_ear_models_differ_from_the_buds() {
        for m in [Model::Max, Model::MaxUsbC, Model::Max2] {
            let caps = m.capabilities();
            assert!(caps.has_anc, "{m:?}");
            assert!(!caps.has_case_battery, "{m:?} has no case");
            assert!(
                !caps.has_in_ear_detection,
                "{m:?} detects a head, not an ear"
            );
            assert!(!caps.has_mic_selection, "{m:?} has no two microphones");
        }
        // The first Max does not do conversation awareness; the H2 one does.
        assert!(!Model::Max.capabilities().has_conv_awareness);
        assert!(!Model::Max.capabilities().has_loud_sound_reduction);
        assert!(Model::Max2.capabilities().has_conv_awareness);
        assert!(Model::Max2.capabilities().has_loud_sound_reduction);
    }

    #[test]
    fn an_unknown_code_keeps_what_every_pair_can_do() {
        let caps = Model::from_code(0x1234).capabilities();
        assert_eq!(caps.model, Model::Unknown);
        assert!(caps.has_case_battery);
        assert!(caps.has_rename);
        assert!(!caps.has_anc);
        assert_eq!(Model::Unknown.label(), "AirPods (unknown model)");
        // A drawing is always needed, so an unknown pair gets the
        // commonest case rather than an empty card.
        assert_eq!(Model::Unknown.art(), "pro2-usbc");
    }

    #[test]
    fn an_unknown_pair_learns_from_what_it_reports() {
        use crate::aap::setting;
        let mut caps = Capabilities::conservative(Model::Unknown);
        assert!(!caps.has_anc);
        // The noise-control setting proves both modes.
        assert!(caps.learn_from_setting(setting::NOISE_CONTROL, 2));
        assert!(caps.has_anc && caps.has_transparency);
        assert!(!caps.has_adaptive, "nothing said adaptive yet");
        // Seeing the device sit in adaptive does.
        assert!(caps.learn_from_setting(setting::NOISE_CONTROL, 4));
        assert!(caps.has_adaptive);
        assert!(caps.learn_from_setting(setting::CONV_AWARE, 1));
        assert!(caps.has_conv_awareness);
        assert!(caps.learn_from_setting(setting::EAR_DETECTION, 1));
        assert!(caps.has_in_ear_detection && caps.has_ear_detection_setting);
        assert!(caps.learn_from_setting(setting::MIC_MODE, 0));
        assert!(caps.has_mic_selection);
        // Nothing new to learn, and an id we know nothing about.
        assert!(!caps.learn_from_setting(setting::MIC_MODE, 0));
        assert!(!caps.learn_from_setting(0xFE, 1));
    }

    #[test]
    fn learning_never_invents_what_was_not_reported() {
        use crate::aap::setting;
        let mut caps = Capabilities::conservative(Model::Unknown);
        // The chime is the sound the buds make on a mode change, not
        // the tone on a press, and it says nothing about either.
        caps.learn_from_setting(setting::CHIME_VOLUME, 50);
        assert_eq!(caps, Capabilities::conservative(Model::Unknown));
    }

    #[test]
    fn a_learned_row_never_contradicts_itself() {
        use crate::aap::setting;
        // Settings arrive one frame at a time and in the device's own
        // order, so each one has to leave a row that holds together.
        // Adaptive without ANC let someone switch to it and then be
        // refused the way back.
        for (id, value) in [
            (setting::AUTO_ANC_LEVEL, 50),
            (setting::ONE_BUD_ANC, 1),
            (setting::NOISE_CONTROL, 4),
            (setting::EAR_DETECTION, 1),
            (setting::MIC_MODE, 0),
        ] {
            let mut caps = Capabilities::conservative(Model::Unknown);
            caps.learn_from_setting(id, value);
            assert!(coherent(&caps), "setting {id:02x} left a broken row");
        }
    }

    /// The rules the table keeps and a learned row must keep too.
    fn coherent(c: &Capabilities) -> bool {
        (!c.has_adaptive || (c.has_anc && c.has_transparency))
            && (!c.has_conv_awareness || c.has_anc)
            && (!c.has_ear_detection_setting || c.has_in_ear_detection)
    }

    #[test]
    fn every_model_has_exactly_one_row() {
        // A new enum variant without a row goes quiet rather than
        // wrong: `label()` turns into "unknown model" and
        // `capabilities()` into the conservative guess.
        for m in Model::ALL {
            let rows = MODELS.iter().filter(|s| s.model == m).count();
            assert_eq!(rows, 1, "{m:?} has {rows} rows");
        }
        assert_eq!(MODELS.len(), Model::ALL.len());
        assert!(
            !MODELS.iter().any(|s| s.model == Model::Unknown),
            "Unknown must not have a row"
        );
        assert!(Model::Unknown.spec().is_none());
    }

    #[test]
    fn numbers_of_two_variants_in_one_list_refine_nothing() {
        // Such a list belongs to two different pairs — one was still
        // remembered when the other turned up. Choosing from it handed
        // one pair the other's row, and the daemon sorts the numbers,
        // so it was always the same wrong one.
        let mixed = vec!["A3440".to_string(), "A3532".to_string()];
        assert_eq!(Model::Gen5.refine(&mixed), Model::Gen5);
        assert_eq!(Model::Gen5Wireless.refine(&mixed), Model::Gen5Wireless);
        // One number alone still decides.
        assert_eq!(
            Model::Gen5.refine(&["A3440".to_string()]),
            Model::Gen5Wireless
        );
        // Twice the same number is not a conflict.
        assert_eq!(
            Model::Gen5.refine(&["A3440".to_string(), "A3440".to_string()]),
            Model::Gen5Wireless
        );
    }

    #[test]
    fn every_row_in_the_table_holds_together() {
        for s in MODELS {
            assert!(coherent(&s.caps()), "{:?} has a broken row", s.model);
        }
    }

    #[test]
    fn the_ear_frame_alone_proves_the_detection() {
        // The AirPods Pro 3 never send setting 0x0a, so a dump cannot
        // show it. The frame that names the bud in the ear does.
        let mut caps = Capabilities::conservative(Model::Unknown);
        assert!(!caps.has_in_ear_detection);
        assert!(caps.learn_in_ear());
        assert!(caps.has_in_ear_detection);
        // Nothing else is invented, and the second frame changes nothing.
        assert!(!caps.learn_in_ear());
        assert!(!caps.has_ear_detection_setting);
        assert!(!caps.has_mic_selection);
        assert!(!caps.has_anc);
    }

    #[test]
    fn the_table_is_consistent() {
        for s in MODELS {
            assert_eq!(s.caps().model, s.model, "{:?}", s.model);
            assert!(!s.pids.is_empty(), "{:?} has no product code", s.model);
            assert!(!s.label.is_empty(), "{:?} has no name", s.model);
            assert!(!s.art.is_empty(), "{:?} has no drawing", s.model);
            assert_eq!(s.model.spec().map(|f| f.label), Some(s.label));
        }
        // Only the owner's own pair is checked against hardware.
        let verified: Vec<Model> = MODELS
            .iter()
            .filter(|s| s.verified)
            .map(|s| s.model)
            .collect();
        assert_eq!(verified, vec![Model::Pro2UsbC]);
    }

    #[test]
    fn a_product_code_belongs_to_one_model_unless_variants_share_it() {
        for s in MODELS {
            for pid in s.pids {
                let owners: Vec<Model> = MODELS
                    .iter()
                    .filter(|o| o.pids.contains(pid))
                    .map(|o| o.model)
                    .collect();
                // Sharing is only allowed where an A-number can decide.
                if owners.len() > 1 {
                    for o in &owners {
                        let spec = o.spec().expect("in the table");
                        assert!(
                            !spec.model_numbers.is_empty(),
                            "{o:?} shares 0x{pid:04X} but has no A-number to tell it apart"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn an_a_number_belongs_to_one_model() {
        let mut seen: Vec<&str> = MODELS
            .iter()
            .flat_map(|s| s.model_numbers)
            .copied()
            .collect();
        let count = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), count, "an A-number is listed twice");
    }
}
