//! `podctl meter` — live RMS / peak meter on the AirPods playback sink.
//!
//! Spawns `parec` against the bluez_output monitor source, reads s16le
//! samples, computes RMS + peak per window, and prints dBFS.

use std::io::{Read, Write};
use std::process::{Command, Stdio};

use podctl::exitcode;

const SAMPLE_RATE: u32 = 44100;
const CHANNELS: u32 = 2;

pub fn run(args: &[String]) -> i32 {
    let json = args.iter().any(|a| a == "--json");
    let plain = args.iter().any(|a| a == "--plain");
    let one_shot = args.iter().any(|a| a == "--once");
    let interval_ms: u64 = arg_value(args, "--interval")
        .and_then(|v| v.parse().ok())
        .unwrap_or(100);
    let device = arg_value(args, "--device").or_else(find_airpods_monitor);

    let Some(monitor) = device else {
        eprintln!("podctl: no AirPods monitor source found — is the bud connected?");
        return exitcode::UNAVAILABLE;
    };

    let mut child = match Command::new("parec")
        .args([
            &format!("--device={monitor}"),
            &format!("--rate={SAMPLE_RATE}"),
            &format!("--channels={CHANNELS}"),
            "--format=s16le",
            "--raw",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("podctl: parec failed to start ({e}) — install pulseaudio-utils?");
            return exitcode::UNAVAILABLE;
        }
    };
    let mut stdout = child.stdout.take().expect("parec stdout piped");

    let samples_per_window = (SAMPLE_RATE as u64 * interval_ms / 1000) as usize * CHANNELS as usize;
    let bytes_per_window = samples_per_window * 2;
    let mut buf = vec![0u8; bytes_per_window];

    let isatty = unsafe { libc_isatty(1) != 0 };
    let bar_width: usize = 36;

    loop {
        if stdout.read_exact(&mut buf).is_err() {
            break;
        }
        let (rms, peak) = rms_peak(&buf);
        let rms_db = to_dbfs(rms);
        let peak_db = to_dbfs(peak);
        if json {
            println!("{{\"rms_dbfs\":{rms_db:.1},\"peak_dbfs\":{peak_db:.1}}}");
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

fn find_airpods_monitor() -> Option<String> {
    let out = Command::new("pactl")
        .env("LC_ALL", "C")
        .args(["list", "short", "sources"])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    for line in s.lines() {
        // tab-separated: id<TAB>name<TAB>...
        let mut parts = line.split('\t');
        let _id = parts.next()?;
        let name = parts.next()?;
        if name.starts_with("bluez_output.") && name.ends_with(".monitor") {
            return Some(name.to_string());
        }
    }
    None
}

unsafe extern "C" {
    #[link_name = "isatty"]
    fn libc_isatty(fd: i32) -> i32;
}
