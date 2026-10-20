//! `podctl meter` — live RMS / peak meter on the AirPods playback sink.
//!
//! Captures the sink (see `audio::capture_cmd`), reads s16le samples,
//! computes RMS + peak per window, and prints dBFS.

use std::io::{Read, Write};
use std::process::Stdio;

use podctl::{audio, exitcode};

const SAMPLE_RATE: u32 = 44100;
const CHANNELS: u32 = 2;

/// A window shorter than this reads zero samples at a time and spins.
const MIN_INTERVAL_MS: u64 = 10;

/// Dropped before measuring: the first samples out of `pw-record` are
/// whatever the ring buffer held when it started, which read as
/// -1.3 dBFS peak on a silent sink.
const WARMUP_MS: u64 = 100;

pub fn run(args: &[String], json: bool, pretty: bool) -> i32 {
    let json = json || args.iter().any(|a| a == "--json");
    let plain = args.iter().any(|a| a == "--plain");
    let one_shot = args.iter().any(|a| a == "--once");
    // A value that is not a number used to fall back to 100 ms without
    // a word, and 0 spun a core.
    let interval_ms: u64 = match arg_value(args, "--interval") {
        None => 100,
        Some(raw) => match raw.parse::<u64>() {
            Ok(ms) if ms >= MIN_INTERVAL_MS => ms,
            _ => {
                eprintln!(
                    "podctl: meter: unknown value '{raw}' (expected milliseconds, at least {MIN_INTERVAL_MS})"
                );
                eprintln!("hint: see 'podctl help meter'");
                return exitcode::USAGE;
            }
        },
    };
    let device = arg_value(args, "--device").or_else(find_airpods_sink);

    let Some(sink) = device else {
        eprintln!("podctl: no AirPods sink found — is the bud connected?");
        return exitcode::UNAVAILABLE;
    };

    let Some(mut cmd) = audio::capture_cmd(&sink, SAMPLE_RATE, CHANNELS) else {
        eprintln!(
            "podctl: no capture tool — install pipewire-audio (pw-record) or pulseaudio-utils (parec)."
        );
        return exitcode::UNAVAILABLE;
    };
    let mut child = match cmd.stdout(Stdio::piped()).stderr(Stdio::null()).spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("podctl: capture failed to start ({e})");
            return exitcode::UNAVAILABLE;
        }
    };
    let mut stdout = child.stdout.take().expect("capture stdout piped");

    let samples_per_window = (SAMPLE_RATE as u64 * interval_ms / 1000) as usize * CHANNELS as usize;
    let bytes_per_window = samples_per_window * 2;
    let mut buf = vec![0u8; bytes_per_window];

    let isatty = unsafe { libc_isatty(1) != 0 };
    let bar_width: usize = 36;

    let warmup = (SAMPLE_RATE as u64 * WARMUP_MS / 1000) as usize * CHANNELS as usize * 2;
    let _ = stdout.read_exact(&mut vec![0u8; warmup]);

    loop {
        if stdout.read_exact(&mut buf).is_err() {
            break;
        }
        let (rms, peak) = rms_peak(&buf);
        let rms_db = to_dbfs(rms);
        let peak_db = to_dbfs(peak);
        if json {
            // One envelope per window: a meter is NDJSON by nature.
            podctl::json::Envelope::ok(
                "meter",
                serde_json::json!({
                    "rms_dbfs": round1(rms_db),
                    "peak_dbfs": round1(peak_db),
                }),
            )
            .print(pretty);
            let _ = std::io::stdout().flush();
        } else if plain || !isatty {
            println!("RMS {rms_db:6.1} dBFS  peak {peak_db:6.1} dBFS");
        } else {
            // ASCII bar; clear line + carriage return for in-place update.
            print!(
                "\r\x1b[K  RMS {:6.1} dBFS  peak {:6.1}  {}",
                rms_db,
                peak_db,
                bar(rms_db, bar_width)
            );
            let _ = std::io::stdout().flush();
        }
        if one_shot {
            break;
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    if isatty && !json && !plain {
        println!();
    }
    exitcode::OK
}

/// One decimal, as the plain output prints it.
fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

fn arg_value(args: &[String], key: &str) -> Option<String> {
    let mut iter = args.iter();
    while let Some(a) = iter.next() {
        if a == key {
            return iter.next().cloned();
        }
        if let Some(v) = a.strip_prefix(&format!("{key}=")) {
            return Some(v.to_string());
        }
    }
    None
}

fn rms_peak(buf: &[u8]) -> (f64, f64) {
    let mut sum_sq: f64 = 0.0;
    let mut peak: u16 = 0;
    let mut n: u64 = 0;
    for chunk in buf.as_chunks::<2>().0 {
        let v = i16::from_le_bytes(*chunk);
        let av = v.unsigned_abs();
        if av > peak {
            peak = av;
        }
        sum_sq += (v as f64) * (v as f64);
        n += 1;
    }
    let rms = if n > 0 {
        (sum_sq / n as f64).sqrt()
    } else {
        0.0
    };
    (rms, peak as f64)
}

fn to_dbfs(v: f64) -> f64 {
    if v < 1.0 {
        -120.0
    } else {
        20.0 * (v / 32768.0).log10()
    }
}

fn bar(db: f64, width: usize) -> String {
    // Map -60..0 dB to 0..width.
    let clamped = db.clamp(-60.0, 0.0);
    let filled = ((clamped + 60.0) / 60.0 * width as f64) as usize;
    let mut s = String::with_capacity(width + 2);
    s.push('[');
    for i in 0..width {
        s.push(if i < filled { '#' } else { ' ' });
    }
    s.push(']');
    s
}

/// The sink of the paired AirPods. Any other Bluetooth sink — a
/// speaker, a headset — is somebody else's audio.
fn find_airpods_sink() -> Option<String> {
    let addr = podctl::bluez::primary_airpods_address()?;
    audio::sink_for(&addr).map(|s| s.name)
}

unsafe extern "C" {
    #[link_name = "isatty"]
    fn libc_isatty(fd: i32) -> i32;
}
