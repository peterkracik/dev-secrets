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

    /// Like `run`, but executes the binary from `cwd` so folder-based
    /// assignments resolve against it.
    fn run_in(&self, cwd: &std::path::Path, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_devsecrets"))
            .args(args)
            .current_dir(cwd)
            .env("XDG_CONFIG_HOME", &self.dir)
            .env("HOME", &self.dir)
            .output()
            .expect("failed to run devsecrets")
    }

    fn ok_in(&self, cwd: &std::path::Path, args: &[&str]) -> String {
        let out = self.run_in(cwd, args);
        assert!(
            out.status.success(),
            "command {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
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
fn list_falls_back_to_folder_assignment() {
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

    // A folder we'll assign to api/dev.
    let folder = sb.dir.join("workdir");
    std::fs::create_dir_all(&folder).unwrap();
    let canonical = std::fs::canonicalize(&folder).unwrap();
    let canonical = canonical.to_str().unwrap();

    // Write the assignment directly (setup's wizard needs a TTY). Build the
    // JSON with serde so the path is escaped correctly on every platform
    // (Windows paths contain backslashes that would break a raw string).
    let cfg = sb.dir.join("devsecrets");
    std::fs::create_dir_all(&cfg).unwrap();
    let meta = serde_json::json!({
        "assignments": { canonical: { "project": "api", "env": "dev" } }
    });
    std::fs::write(cfg.join("meta.json"), serde_json::to_string(&meta).unwrap()).unwrap();

    // `secret list` with no -p/-e uses the folder's assignment.
    let list = sb.ok_in(&folder, &["secret", "list"]);
    assert!(list.contains("DB_HOST=localhost"), "list was: {list}");

    // The `secrets` alias works too.
    let aliased = sb.ok_in(&folder, &["secrets", "list"]);
    assert!(aliased.contains("DB_HOST=localhost"), "list was: {aliased}");

    // `env list` with no -p uses the folder's assigned project.
    let envs = sb.ok_in(&folder, &["env", "list"]);
    assert!(envs.contains("dev"), "env list was: {envs}");

    // Without an assignment (run elsewhere), it errors helpfully.
    let elsewhere = sb.run_in(&sb.dir, &["secret", "list"]);
    assert!(
        !elsewhere.status.success(),
        "expected error without assignment"
    );
}

#[test]
fn run_injects_secrets_into_command_env() {
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
        "HOST",
        "localhost",
    ]);
    // A reference is resolved before being injected.
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

    // The child process sees the resolved values in its environment.
    let out = sb.ok(&[
        "run",
        "-p",
        "api",
        "-e",
        "dev",
        "--",
        "sh",
        "-c",
        "echo $URL",
    ]);
    assert_eq!(out.trim(), "http://localhost:5432");

    // The `exec` alias works too, and --raw keeps references literal.
    let raw = sb.ok(&[
        "exec",
        "-p",
        "api",
        "-e",
        "dev",
        "--raw",
        "--",
        "sh",
        "-c",
        "echo $URL",
    ]);
    assert_eq!(raw.trim(), "http://${HOST}:5432");
}

#[test]
fn run_propagates_command_exit_code() {
    let sb = Sandbox::new();
    sb.ok(&["project", "create", "api"]);
    sb.ok(&["env", "create", "-p", "api", "dev"]);

    let out = sb.run(&["run", "-p", "api", "-e", "dev", "--", "sh", "-c", "exit 3"]);
    assert_eq!(out.status.code(), Some(3), "expected child's exit code");
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
