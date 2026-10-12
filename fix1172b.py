p = "apps/filmcraft/src/main.rs"
s = open(p).read()

# unsafe set_var (edition 2024)
old = """    if let Some(d) = data_root.as_ref() {
        std::env::set_var("FILMCRAFT_DATA_DIR", d);
    }"""
new = """    if let Some(d) = data_root.as_ref() {
        // SAFETY: main is single-threaded here; no other thread reads the environment yet.
        unsafe { std::env::set_var("FILMCRAFT_DATA_DIR", d) };
    }"""
assert old in s
s = s.replace(old, new)

# re-apply the token wiring that main.rs lost (main's version has no token)
old = """        if let Some(port) = control_port {
            let rx = control_server::start(port, cc.egui_ctx.clone());"""
new = """        if let Some(port) = control_port {
            let token = control_server::Token::new();
            // The CLI bridge picks the token up from here (or from FILMCRAFT_CONTROL_TOKEN).
            if let Some(dir) = data_dir.clone().or_else(default_data_dir) {
                if std::fs::create_dir_all(&dir).is_ok() {
                    control_server::write_token_file(&dir.join("control.token"), &token);
                }
            }
            let rx = control_server::start(port, token, cc.egui_ctx.clone());"""
assert old in s
s = s.replace(old, new)
open(p, "w").write(s)
print("main fixed")
