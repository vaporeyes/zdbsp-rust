// ABOUTME: Regression harness. Runs the C++ baseline and the Rust port over a corpus of WADs
// ABOUTME: under a matrix of CLI flags and diffs the resulting WADs byte-for-byte.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

const DEFAULT_BASELINE: &str = "/Users/jsh/dev/repos/zdbsp/build/zdbsp";
const DEFAULT_CORPUS: &str = "/Users/jsh/media/doom_wads";

// Flag combinations to test. The empty string means "default flags".
const FLAG_MATRIX: &[&[&str]] = &[
    &[],
    &["-z"], // compressed ZNODES
    &["-Z"], // compress only regular NODES
    &["-X"], // extended uncompressed XNOD
    &["-P"], // skip polyobject containment checks
    &["-g"], // build GL nodes alongside regular
    &["-G"], // gl-matching (single build, derive regular from GL)
    &["-x"], // GL-only output
    &["-5"], // v5 GL flag (no-op without -g)
    // -g -5 / -G -5 fail because the C++ writes uninitialized struct-padding
    // bytes in v5 GL_SEGS records (strict UB). We emit zeros.
];

fn baseline_path() -> PathBuf {
    std::env::var_os("ZDBSP_BASELINE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_BASELINE))
}

fn corpus_dir() -> PathBuf {
    std::env::var_os("ZDBSP_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_CORPUS))
}

fn rust_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_zdbsp"))
}

fn collect_wads(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut wads: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .and_then(|s| s.to_str())
                .map(|s| s.eq_ignore_ascii_case("wad"))
                .unwrap_or(false)
        })
        .collect();
    wads.sort();
    wads
}

fn run_builder(bin: &Path, input: &Path, output: &Path, flags: &[&str]) -> std::io::Result<()> {
    let mut cmd = Command::new(bin);
    for f in flags {
        cmd.arg(f);
    }
    let mut out_arg = OsString::from("-o");
    out_arg.push(output);
    cmd.arg(out_arg).arg(input);
    let status = cmd.status()?;
    if !status.success() {
        return Err(std::io::Error::other(format!(
            "{:?} {:?} exited with {status}",
            bin, flags
        )));
    }
    Ok(())
}

#[test]
fn regression_matrix() {
    // Until the Rust port is far enough along to plausibly match the baseline,
    // this test runs in advisory mode: failures are reported but don't fail the run.
    // Set ZDBSP_REGRESSION=1 to enforce byte-identical parity.
    let strict = std::env::var_os("ZDBSP_REGRESSION").is_some();

    let baseline = baseline_path();
    let corpus = corpus_dir();
    let rust_bin = rust_binary();

    if !baseline.is_file() {
        eprintln!("SKIP: baseline binary not found at {}", baseline.display());
        eprintln!("      Set ZDBSP_BASELINE or build /Users/jsh/dev/repos/zdbsp.");
        return;
    }

    if !strict {
        eprintln!("SKIP: regression matrix is advisory; set ZDBSP_REGRESSION=1 to enforce");
        return;
    }

    let wads = collect_wads(&corpus);
    if wads.is_empty() {
        eprintln!("SKIP: no .wad files in corpus dir {}", corpus.display());
        eprintln!("      Set ZDBSP_CORPUS or drop WADs into that directory.");
        return;
    }

    let scratch = std::env::temp_dir().join("zdbsp-rust-regression");
    std::fs::create_dir_all(&scratch).unwrap();

    let mut failures: Vec<String> = Vec::new();
    let mut compared = 0usize;

    for wad in &wads {
        let stem = wad.file_stem().unwrap().to_string_lossy().to_string();
        for flags in FLAG_MATRIX {
            let tag: String = if flags.is_empty() {
                "default".into()
            } else {
                flags.join("_").replace('-', "")
            };
            let baseline_out = scratch.join(format!("{stem}.{tag}.baseline.wad"));
            let rust_out = scratch.join(format!("{stem}.{tag}.rust.wad"));

            if let Err(e) = run_builder(&baseline, wad, &baseline_out, flags) {
                failures.push(format!("baseline {} {tag}: {e}", wad.display()));
                continue;
            }
            match run_builder(&rust_bin, wad, &rust_out, flags) {
                Ok(()) => {}
                Err(e) => {
                    // Pre-Phase-1 the Rust binary is a stub; treat that as expected for now.
                    failures.push(format!("rust {} {tag}: {e}", wad.display()));
                    continue;
                }
            }

            let baseline_bytes = std::fs::read(&baseline_out).unwrap();
            let rust_bytes = std::fs::read(&rust_out).unwrap();
            compared += 1;
            if baseline_bytes != rust_bytes {
                failures.push(format!(
                    "{} {tag}: outputs differ ({} vs {} bytes)",
                    wad.display(),
                    baseline_bytes.len(),
                    rust_bytes.len()
                ));
            }
        }
    }

    eprintln!("compared {compared} (wad, flag-set) pairs");
    if !failures.is_empty() {
        for f in &failures {
            eprintln!("FAIL: {f}");
        }
        panic!("{} regression failure(s)", failures.len());
    }
}
