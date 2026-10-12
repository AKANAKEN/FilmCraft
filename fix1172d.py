p = "apps/filmcraft/src/control_server.rs"
s = open(p).read()
old = """/// returns the reply object (the request's `id` is added here).
fn serve_lines"""
new = """/// returns the reply object (the request's `id` is added here). Test helper: production uses
/// `serve_lines_gated` (#163).
#[cfg(test)]
fn serve_lines"""
assert old in s
s = s.replace(old, new)
open(p, "w").write(s)
print("serve_lines gated to tests")

p = "apps/filmcraft/src/main.rs"
s = open(p).read()
old = """            if let Some(dir) = data_dir.clone().or_else(default_data_dir) {
                if std::fs::create_dir_all(&dir).is_ok() {
                    control_server::write_token_file(&dir.join("control.token"), &token);
                }
            }"""
new = """            if let Some(dir) = data_dir.clone().or_else(default_data_dir)
                && std::fs::create_dir_all(&dir).is_ok()
            {
                control_server::write_token_file(&dir.join("control.token"), &token);
            }"""
assert old in s
s = s.replace(old, new)
open(p, "w").write(s)
print("if collapsed")
