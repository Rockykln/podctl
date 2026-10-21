use std::path::{Path, PathBuf};
use std::process::Command;

use tiny_skia::Pixmap;

fn dump(out: &Path, args: &[&str]) {
    let status = Command::new(env!("CARGO_BIN_EXE_podctl-popup"))
        .arg("--dump")
        .arg(out)
        .args(args)
        .status()
        .expect("spawn podctl-popup");
    assert!(
        status.success(),
        "podctl-popup --dump {args:?} exited {status}"
    );
}

fn tmp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("podctl-popup-{}-{name}", std::process::id()))
}

#[test]
fn dump_is_valid_png_and_deterministic() {
    let (a, b) = (tmp("a.png"), tmp("b.png"));
    dump(&a, &["--theme", "dark"]);
    dump(&b, &["--theme", "dark"]);
    let pa = std::fs::read(&a).unwrap();
    let pb = std::fs::read(&b).unwrap();
    assert_eq!(&pa[..8], b"\x89PNG\r\n\x1a\n", "not a PNG");
    assert_eq!(pa, pb, "render is not deterministic across runs");
    let _ = std::fs::remove_file(&a);
    let _ = std::fs::remove_file(&b);
}

#[test]
fn an_unknown_model_is_refused() {
    let status = Command::new(env!("CARGO_BIN_EXE_podctl-popup"))
        .args([
            "--dump",
            tmp("x.png").to_str().unwrap(),
            "--model",
            "airpods9",
        ])
        .status()
        .expect("spawn podctl-popup");
    assert!(!status.success());
}

/// The design, model by model: every reference in
/// tests/golden is drawn again and compared. Only antialiasing may
/// differ — at most 5/255 per channel, on under 0.5 % of the pixels.
#[test]
fn every_model_matches_the_approved_design() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    let mut refs: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("tests/golden")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "png"))
        .collect();
    refs.sort();
    assert_eq!(refs.len(), 28, "14 models, dark and light");

    let mut failed = Vec::new();
    for reference in &refs {
        let name = reference.file_name().unwrap().to_str().unwrap();
        let (slug, theme) = name
            .strip_prefix("model-")
            .and_then(|n| n.strip_suffix("@2x.png"))
            .and_then(|n| n.rsplit_once('-'))
            .expect("model-<slug>-<theme>@2x.png");
        let out = tmp(name);
        dump(
            &out,
            &[
                "--model",
                slug,
                "--theme",
                theme,
                "--scale",
                "2",
                "--backdrop",
            ],
        );
        let got = Pixmap::load_png(&out).unwrap();
        let want = Pixmap::load_png(reference).unwrap();
        let _ = std::fs::remove_file(&out);
        assert_eq!(
            (got.width(), got.height()),
            (want.width(), want.height()),
            "{name}"
        );
        let mut off = 0usize;
        for (a, b) in got.data().chunks(4).zip(want.data().chunks(4)) {
            if a.iter().zip(b).any(|(x, y)| x.abs_diff(*y) > 5) {
                off += 1;
            }
        }
        let share = off as f64 / (got.width() * got.height()) as f64;
        if share >= 0.005 {
            failed.push(format!(
                "{name}: {:.2} % of the pixels differ",
                share * 100.0
            ));
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}
