//! End-to-end tests that drive the compiled `devsecrets` binary.
//!
//! Each test runs in an isolated config directory (via `XDG_CONFIG_HOME`) so
//! they don't touch the developer's real store and can run in parallel.

use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

/// A throwaway config directory that is removed on drop.
struct Sandbox {
    dir: PathBuf,
    counter: std::cell::Cell<u32>,
}

impl Sandbox {
    fn new() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let mut dir = std::env::temp_dir();
        dir.push(format!("devsecrets-it-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Sandbox {
            dir,
            counter: std::cell::Cell::new(0),
        }
    }

    fn run(&self, args: &[&str]) -> Output {
        // A fresh HOME too, so nothing leaks from the runner's environment.
        Command::new(env!("CARGO_BIN_EXE_devsecrets"))
            .args(args)
            .env("XDG_CONFIG_HOME", &self.dir)
            .env("HOME", &self.dir)
            .output()
            .expect("failed to run devsecrets")
    }

    fn ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "command {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    /// Unique temp file path inside the sandbox.
    fn tmp(&self, name: &str) -> PathBuf {
        let n = self.counter.get();
        self.counter.set(n + 1);
        self.dir.join(format!("{n}-{name}"))
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn project_env_secret_roundtrip() {
    let sb = Sandbox::new();
    sb.ok(&["project", "create", "api"]);
    sb.ok(&["env", "create", "-p", "api", "dev"]);
    sb.ok(&[
        "secret",
        "set",
        "-p",
        "api",
        "-e",
        "dev",
        "DB_HOST",
        "localhost",
    ]);

    let got = sb.ok(&["secret", "get", "-p", "api", "-e", "dev", "DB_HOST"]);
    assert_eq!(got.trim(), "localhost");

    let list = sb.ok(&["secret", "list", "-p", "api", "-e", "dev"]);
    assert!(list.contains("DB_HOST=localhost"), "list was: {list}");
}

#[test]
fn references_resolve_in_all_forms() {
    let sb = Sandbox::new();
    sb.ok(&["project", "create", "api"]);
    sb.ok(&["env", "create", "-p", "api", "dev"]);
    sb.ok(&["env", "create", "-p", "api", "shared"]);
    sb.ok(&[
        "secret",
        "set",
        "-p",
        "api",
        "-e",
        "dev",
        "HOST",
        "localhost",
    ]);
    sb.ok(&["secret", "set", "-p", "api", "-e", "shared", "TOKEN", "abc"]);

    // same-env, other-env references
    sb.ok(&[
        "secret",
        "set",
        "-p",
        "api",
        "-e",
        "dev",
        "URL",
        "http://${HOST}:5432",
    ]);
    sb.ok(&[
        "secret",
        "set",
        "-p",
        "api",
        "-e",
        "dev",
        "AUTH",
        "Bearer ${shared.TOKEN}",
    ]);

    assert_eq!(
        sb.ok(&["secret", "get", "-p", "api", "-e", "dev", "URL"])
            .trim(),
        "http://localhost:5432"
    );
    assert_eq!(
        sb.ok(&["secret", "get", "-p", "api", "-e", "dev", "AUTH"])
            .trim(),
        "Bearer abc"
    );
    // --raw keeps the reference literal
    assert_eq!(
        sb.ok(&["secret", "get", "-p", "api", "-e", "dev", "URL", "--raw"])
            .trim(),
        "http://${HOST}:5432"
    );
}

#[test]
fn export_formats() {
    let sb = Sandbox::new();
    sb.ok(&["project", "create", "api"]);
    sb.ok(&["env", "create", "-p", "api", "dev"]);
    sb.ok(&["secret", "set", "-p", "api", "-e", "dev", "A", "1"]);
    sb.ok(&["secret", "set", "-p", "api", "-e", "dev", "B", "two"]);

    let env = sb.ok(&["export", "-p", "api", "-e", "dev", "--format", "env"]);
    assert!(env.contains("A=1") && env.contains("B=two"));

    let shell = sb.ok(&["export", "-p", "api", "-e", "dev", "--format", "shell"]);
    assert!(shell.contains("export A=1"));

    let json = sb.ok(&["export", "-p", "api", "-e", "dev", "--format", "json"]);
    assert!(json.contains("\"A\": \"1\"") && json.contains("\"B\": \"two\""));

    let toml = sb.ok(&["export", "-p", "api", "-e", "dev", "--format", "toml"]);
    assert!(toml.contains("A = \"1\""));
}

#[test]
fn import_then_export_roundtrip() {
    let sb = Sandbox::new();
    sb.ok(&["project", "create", "api"]);
    sb.ok(&["env", "create", "-p", "api", "dev"]);

    let file = sb.tmp("in.env");
    std::fs::write(&file, "FOO=bar\nBAZ=\"with space\"\n").unwrap();
    sb.ok(&["import", file.to_str().unwrap(), "-p", "api", "-e", "dev"]);

    let list = sb.ok(&["secret", "list", "-p", "api", "-e", "dev"]);
    assert!(list.contains("FOO=bar"), "list was: {list}");

    let out = sb.tmp("out.env");
    sb.ok(&["export", out.to_str().unwrap(), "-p", "api", "-e", "dev"]);
    let written = std::fs::read_to_string(&out).unwrap();
    assert!(written.contains("FOO=bar"));
    assert!(written.contains("BAZ=\"with space\""));
}

#[test]
fn type_validation_rejects_bad_values() {
    let sb = Sandbox::new();
    sb.ok(&["project", "create", "api"]);
    sb.ok(&["env", "create", "-p", "api", "dev"]);

    // a valid number is accepted
    sb.ok(&[
        "secret", "set", "-p", "api", "-e", "dev", "PORT", "5432", "--type", "number",
    ]);
    // a non-number is rejected
    let bad = sb.run(&[
        "secret", "set", "-p", "api", "-e", "dev", "NOPE", "abc", "--type", "number",
    ]);
    assert!(!bad.status.success(), "expected number validation to fail");

    // invalid JSON is rejected; valid JSON is accepted
    let bad_json = sb.run(&[
        "secret", "set", "-p", "api", "-e", "dev", "CFG", "notjson", "--type", "json",
    ]);
    assert!(
        !bad_json.status.success(),
        "expected JSON validation to fail"
    );
    sb.ok(&[
        "secret",
        "set",
        "-p",
        "api",
        "-e",
        "dev",
        "CFG",
        r#"{"a":1}"#,
        "--type",
        "json",
    ]);
}

#[test]
fn duplicate_environment_copies_secrets() {
    let sb = Sandbox::new();
    sb.ok(&["project", "create", "api"]);
    sb.ok(&["env", "create", "-p", "api", "dev"]);
    sb.ok(&["secret", "set", "-p", "api", "-e", "dev", "K", "v"]);
    sb.ok(&["duplicate", "-p", "api", "dev", "staging"]);

    let got = sb.ok(&["secret", "get", "-p", "api", "-e", "staging", "K"]);
    assert_eq!(got.trim(), "v");
}
