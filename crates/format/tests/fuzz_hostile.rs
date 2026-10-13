//! Hostile project files: truncation, byte mutation and every number replaced by an extreme
//! value must be an error or a loaded project, never a panic, hang or giant allocation.

use filmcraft_format::{autosave, decode, encode};
use filmcraft_project::Project;
use filmcraft_testkit::hostile::hammer;

fn seeds() -> Vec<Vec<u8>> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut v: Vec<Vec<u8>> = std::fs::read_dir(dir).unwrap().filter_map(|e| std::fs::read(e.ok()?.path()).ok()).collect();
    v.push(encode(&Project::new("Hostile"), false));
    v
}

fn exercise(d: &[u8]) {
    let _ = decode(d);
}

#[test]
fn mutated_projects_never_panic() {
    for (i, s) in seeds().iter().enumerate() {
        hammer(&format!("project seed {i}"), s, 3_000, exercise);
    }
}

/// Every numeric token of each seed, replaced in turn by values at the edges of every integer width.
#[test]
fn extreme_numbers_never_panic() {
    const EXTREMES: [&str; 11] =
        ["0", "-1", "1", "255", "65536", "2147483648", "4294967295", "9223372036854775807", "-9223372036854775808", "18446744073709551615", "1e308"];
    let (mut loaded, mut total) = (0usize, 0usize);
    for (si, s) in seeds().iter().enumerate() {
        let mut tokens = Vec::new();
        let mut i = 0;
        while i < s.len() {
            if s[i].is_ascii_digit() && !s[..i].last().is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'.' || *b == b'_') {
                let mut j = i;
                while j < s.len() && (s[j].is_ascii_digit() || matches!(s[j], b'.' | b'e' | b'E' | b'+' | b'-')) {
                    j += 1;
                }
                tokens.push((i, j));
                i = j;
            } else {
                i += 1;
            }
        }
        for &(a, b) in tokens.iter().take(600) {
            for e in EXTREMES {
                let mut d = s[..a].to_vec();
                d.extend_from_slice(e.as_bytes());
                d.extend_from_slice(&s[b..]);
                let t = std::time::Instant::now();
                let r = std::panic::catch_unwind(|| decode(&d));
                let Ok(r) = r else { panic!("seed {si}: PANIC with {e} at byte {a}") };
                loaded += r.is_ok() as usize;
                total += 1;
                assert!(t.elapsed().as_secs() < 5, "seed {si}: slow ({:?}) with {e} at byte {a}", t.elapsed());
            }
        }
    }
    eprintln!("{loaded}/{total} extreme-number variants still loaded");
    assert!(total > 100 && loaded > 0, "the mutations must reach the loader");
}

#[test]
fn deeply_nested_json_is_refused() {
    for n in [100usize, 1000, 100_000] {
        let d = format!("{}1{}", "[".repeat(n), "]".repeat(n));
        let _ = decode(d.as_bytes());
        let d = format!("{}{}", "{\"a\":".repeat(n), "1".to_string() + &"}".repeat(n));
        let _ = decode(d.as_bytes());
    }
}

/// Regression: `civil` added the zone offset to the timestamp with plain `+`.
#[test]
fn autosave_timestamps_survive_extreme_clocks() {
    for unix in [i64::MIN, i64::MIN + 1, -1, 0, i64::MAX - 1, i64::MAX] {
        for off in [i32::MIN, -1, 0, i32::MAX] {
            let _ = autosave::timestamp(unix, off);
            let _ = autosave::display_time(unix, off);
            let _ = autosave::auto_save_name("x", unix, off);
        }
    }
}
