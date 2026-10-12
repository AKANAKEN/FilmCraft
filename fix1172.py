import re

# ---------- control_server.rs ----------
p = "apps/filmcraft/src/control_server.rs"
s = open(p).read()

# 1. Empty token can never be accepted: matches() rejects empty tokens outright.
old = """    /// Constant-time compare: the reply does not leak how much of a guess was right.
    fn matches(&self, guess: &str) -> bool {
        let (a, b) = (self.0.as_bytes(), guess.as_bytes());"""
new = """    /// Constant-time compare: the reply does not leak how much of a guess was right.
    /// An empty token (generation failed and no `FILMCRAFT_CONTROL_TOKEN` was set) matches
    /// nothing: the gate stays closed rather than admitting anyone.
    fn matches(&self, guess: &str) -> bool {
        if self.0.is_empty() {
            return false;
        }
        let (a, b) = (self.0.as_bytes(), guess.as_bytes());"""
assert old in s
s = s.replace(old, new)

# 2. Windows build: gate the unix-only OpenOptionsExt/mode import behind cfg(unix).
old = """pub fn write_token_file(path: &std::path::Path, token: &Token) {
    use std::fs::OpenOptions;
    use std::os::unix::fs::OpenOptionsExt;
    let result = OpenOptions::new().create(true).write(true).truncate(true).mode(0o600).open(path).and_then(|mut f| f.write_all(token.as_str().as_bytes()));
    if let Err(e) = result {
        eprintln!("filmcraft: control token could not be written to {}: {e}", path.display());
    }
}"""
new = """pub fn write_token_file(path: &std::path::Path, token: &Token) {
    use std::fs::OpenOptions;
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;
    let mut opts = OpenOptions::new();
    opts.create(true).write(true).truncate(true);
    // Owner-only on unix; on other platforms the file system's own permissions apply.
    #[cfg(unix)]
    opts.mode(0o600);
    let result = opts.open(path).and_then(|mut f| f.write_all(token.as_str().as_bytes()));
    if let Err(e) = result {
        eprintln!("filmcraft: control token could not be written to {}: {e}", path.display());
    }
}"""
assert old in s
s = s.replace(old, new)

# 3. Generation failure: no silent empty token — say so loudly at startup too.
old = """            Err(e) => {
                // No OS randomness: refuse to run without a token rather than invent a weak one.
                eprintln!("filmcraft: control server could not generate a token ({e}); set FILMCRAFT_CONTROL_TOKEN");
                Self(String::new())
            }"""
new = """            Err(e) => {
                // No OS randomness: run with no token rather than invent a weak one. `matches`
                // rejects an empty token, so the control channel stays shut until the operator
                // sets FILMCRAFT_CONTROL_TOKEN.
                eprintln!("filmcraft: control server could not generate a token ({e}); the control channel stays closed until FILMCRAFT_CONTROL_TOKEN is set");
                Self(String::new())
            }"""
assert old in s
s = s.replace(old, new)

# 4. Test: an empty token never matches, not even an empty guess.
old = """        let a = Token::from_env(None);
        let b = Token::from_env(None);"""
assert old in s
add = """        let a = Token::from_env(None);
        let b = Token::from_env(None);"""
# find the end of that test fn to append a new test after it — simpler: append at file end inside tests mod? Just append a new test at end of file.
s = s.rstrip() + """

#[cfg(test)]
mod empty_token_tests {
    use super::Token;

    #[test]
    fn an_empty_token_never_matches() {
        // Generation failure leaves an empty token; it must admit nobody, not even the empty guess.
        let t = Token(String::new());
        assert!(!t.matches(""));
        assert!(!t.matches("anything"));
    }
}
"""
open(p, "w").write(s)
print("control_server patched")

# ---------- bridge.rs ----------
p = "crates/automation/src/bridge.rs"
s = open(p).read()
old = """    let dir = filmcraft_engine::autosave::default_data_dir()?;
    let text = std::fs::read_to_string(dir.join(TOKEN_FILE)).ok()?;"""
new = """    // Same resolution the app uses for the token file: `--data-dir` (exported as
    // FILMCRAFT_DATA_DIR) wins over the default data directory (#163).
    let dir = std::env::var_os("FILMCRAFT_DATA_DIR")
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(filmcraft_engine::autosave::default_data_dir)?;
    let text = std::fs::read_to_string(dir.join(TOKEN_FILE)).ok()?;"""
assert old in s
s = s.replace(old, new)
open(p, "w").write(s)
print("bridge patched")

# ---------- main.rs: export FILMCRAFT_DATA_DIR so the bridge resolves the same dir ----------
p = "apps/filmcraft/src/main.rs"
s = open(p).read()
old = """    let data_root = launch.data_dir.clone().or_else(default_data_dir);
    let log_dir = data_root.as_ref().map(|d| d.join(logging::LOG_DIR));"""
new = """    let data_root = launch.data_dir.clone().or_else(default_data_dir);
    // Publish the resolved directory so child processes (the CLI bridge) read the token file
    // from the same place the app writes it (#163).
    if let Some(d) = data_root.as_ref() {
        std::env::set_var("FILMCRAFT_DATA_DIR", d);
    }
    let log_dir = data_root.as_ref().map(|d| d.join(logging::LOG_DIR));"""
assert old in s
s = s.replace(old, new)
open(p, "w").write(s)
print("main patched")
