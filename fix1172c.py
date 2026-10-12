# main.rs: no unsafe set_var — export a dedicated token-path var via std::env (still unsafe)...
# Actually any env mutation is unsafe in edition 2024. Instead: the bridge takes the path from
# FILMCRAFT_CONTROL_TOKEN_FILE, which the *launcher* can set; the app cannot set env for its own
# children without unsafe. So: revert the set_var, and instead have main pass nothing extra —
# the bridge resolves FILMCRAFT_DATA_DIR itself (already patched). For --data-dir (CLI only),
# document that the bridge needs FILMCRAFT_DATA_DIR or FILMCRAFT_CONTROL_TOKEN.
p = "apps/filmcraft/src/main.rs"
s = open(p).read()
old = """    // Publish the resolved directory so child processes (the CLI bridge) read the token file
    // from the same place the app writes it (#163).
    if let Some(d) = data_root.as_ref() {
        // SAFETY: main is single-threaded here; no other thread reads the environment yet.
        unsafe { std::env::set_var("FILMCRAFT_DATA_DIR", d) };
    }
    let log_dir"""
new = """    let log_dir"""
assert old in s
s = s.replace(old, new)
open(p, "w").write(s)
print("main reverted set_var")

# bridge.rs: also honor FILMCRAFT_CONTROL_TOKEN_FILE as the explicit path.
p = "crates/automation/src/bridge.rs"
s = open(p).read()
old = """    // Same resolution the app uses for the token file: `--data-dir` (exported as
    // FILMCRAFT_DATA_DIR) wins over the default data directory (#163).
    let dir = std::env::var_os("FILMCRAFT_DATA_DIR")
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(filmcraft_engine::autosave::default_data_dir)?;
    let text = std::fs::read_to_string(dir.join(TOKEN_FILE)).ok()?;"""
new = """    // An explicit path wins; otherwise the same directory the app resolves: FILMCRAFT_DATA_DIR
    // (the CLI's `--data-dir` is app-side only, so an out-of-process bridge sets the env var)
    // over the default data directory (#163).
    let path = std::env::var_os("FILMCRAFT_CONTROL_TOKEN_FILE")
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| {
            let dir = std::env::var_os("FILMCRAFT_DATA_DIR")
                .filter(|v| !v.is_empty())
                .map(std::path::PathBuf::from)
                .or_else(filmcraft_engine::autosave::default_data_dir)?;
            Some(dir.join(TOKEN_FILE))
        })?;
    let text = std::fs::read_to_string(path).ok()?;"""
assert old in s
s = s.replace(old, new)
open(p, "w").write(s)
print("bridge token-file var added")
