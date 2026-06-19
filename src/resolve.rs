//! Reference resolution for secret values.
//!
//! A value may contain one or more references to other secrets, written in
//! one of three forms depending on how much context you want to repeat:
//!
//! - `${project.env.secret}` — fully qualified, points anywhere in the store;
//! - `${env.secret}`         — another environment in the *same* project;
//! - `${secret}`             — another secret in the *same* project + env.
//!
//! References resolve relative to the secret that contains them, including
//! nested references (a referenced value is resolved relative to *its* own
//! project/env). Resolution is recursive with cycle detection, so shared
//! values can live in exactly one place instead of being duplicated.

use std::collections::HashSet;

use anyhow::{bail, Result};

use crate::model::Store;

/// A fully-qualified pointer to a single secret.
type Ref = (String, String, String);

/// Resolve all references inside `raw`, using `store` for lookups.
///
/// `origin` is the secret currently being resolved (project, env, key); it
/// provides the context for relative references and seeds cycle detection.
pub fn resolve(store: &Store, origin: &Ref, raw: &str) -> Result<String> {
    let mut seen = HashSet::new();
    seen.insert(origin.clone());
    resolve_inner(store, &origin.0, &origin.1, raw, &mut seen)
}

/// Convenience: resolve a value that lives at a known location in the store.
pub fn resolve_at(store: &Store, project: &str, env: &str, key: &str) -> Result<String> {
    let raw = store.value(project, env, key).cloned().unwrap_or_default();
    resolve(
        store,
        &(project.to_string(), env.to_string(), key.to_string()),
        &raw,
    )
}

/// Resolve references in `raw`, where bare/relative references are interpreted
/// relative to `cur_project` / `cur_env`.
fn resolve_inner(
    store: &Store,
    cur_project: &str,
    cur_env: &str,
    raw: &str,
    seen: &mut HashSet<Ref>,
) -> Result<String> {
    let mut out = String::with_capacity(raw.len());
    let bytes = raw.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        // Look for the start of a reference: `${`
        if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            if let Some(end) = raw[i + 2..].find('}') {
                let inner = &raw[i + 2..i + 2 + end];
                let resolved = resolve_reference(store, cur_project, cur_env, inner, seen)?;
                out.push_str(&resolved);
                i = i + 2 + end + 1; // skip past the closing `}`
                continue;
            }
        }
        // Not a reference: copy the byte as part of a UTF-8 char.
        let ch_len = utf8_char_len(bytes[i]);
        out.push_str(&raw[i..i + ch_len]);
        i += ch_len;
    }

    Ok(out)
}

fn resolve_reference(
    store: &Store,
    cur_project: &str,
    cur_env: &str,
    inner: &str,
    seen: &mut HashSet<Ref>,
) -> Result<String> {
    let parts: Vec<&str> = inner.split('.').collect();
    if parts.iter().any(|p| p.is_empty()) {
        bail!(
            "invalid reference `${{{inner}}}` (expected `${{secret}}`, \
             `${{env.secret}}` or `${{project.env.secret}}`)"
        );
    }
    let key_ref: Ref = match parts.as_slice() {
        [secret] => (
            cur_project.to_string(),
            cur_env.to_string(),
            secret.to_string(),
        ),
        [env, secret] => (cur_project.to_string(), env.to_string(), secret.to_string()),
        [project, env, secret] => (project.to_string(), env.to_string(), secret.to_string()),
        _ => bail!(
            "invalid reference `${{{inner}}}` (expected `${{secret}}`, \
             `${{env.secret}}` or `${{project.env.secret}}`)"
        ),
    };

    if seen.contains(&key_ref) {
        bail!(
            "circular reference detected at `${{{}.{}.{}}}`",
            key_ref.0,
            key_ref.1,
            key_ref.2
        );
    }

    let target = match store.value(&key_ref.0, &key_ref.1, &key_ref.2) {
        Some(v) => v.clone(),
        None => bail!(
            "reference `${{{inner}}}` points to a missing secret `{}.{}.{}`",
            key_ref.0,
            key_ref.1,
            key_ref.2
        ),
    };

    seen.insert(key_ref.clone());
    // Nested references resolve relative to the referenced secret's location.
    let resolved = resolve_inner(store, &key_ref.0, &key_ref.1, &target, seen)?;
    seen.remove(&key_ref);
    Ok(resolved)
}

/// Length in bytes of the UTF-8 character starting at `first_byte`.
fn utf8_char_len(first_byte: u8) -> usize {
    match first_byte {
        b if b < 0x80 => 1,
        b if b >> 5 == 0b110 => 2,
        b if b >> 4 == 0b1110 => 3,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Environment, Project};
    use indexmap::IndexMap;

    fn store_with(values: &[(&str, &str, &str, &str)]) -> Store {
        let mut store = Store::default();
        for (proj, env, key, val) in values {
            let p = store
                .projects
                .entry(proj.to_string())
                .or_insert_with(Project::default);
            let e = p
                .environments
                .entry(env.to_string())
                .or_insert_with(Environment::default);
            e.values.insert(key.to_string(), val.to_string());
        }
        store
    }

    #[test]
    fn plain_value_unchanged() {
        let store = Store::default();
        let origin = ("p".into(), "e".into(), "k".into());
        assert_eq!(resolve(&store, &origin, "hello").unwrap(), "hello");
    }

    #[test]
    fn resolves_single_reference() {
        let store = store_with(&[("shared", "common", "DB", "postgres://x")]);
        let origin = ("app".into(), "dev".into(), "DATABASE_URL".into());
        let out = resolve(&store, &origin, "${shared.common.DB}").unwrap();
        assert_eq!(out, "postgres://x");
    }

    #[test]
    fn resolves_embedded_reference() {
        let store = store_with(&[("shared", "common", "HOST", "db.local")]);
        let origin = ("app".into(), "dev".into(), "URL".into());
        let out = resolve(&store, &origin, "http://${shared.common.HOST}:5432").unwrap();
        assert_eq!(out, "http://db.local:5432");
    }

    #[test]
    fn nested_reference_resolved() {
        let store = store_with(&[
            ("base", "e", "A", "value-a"),
            ("base", "e", "B", "${base.e.A}-b"),
        ]);
        let origin = ("x".into(), "y".into(), "z".into());
        let out = resolve(&store, &origin, "${base.e.B}").unwrap();
        assert_eq!(out, "value-a-b");
    }

    #[test]
    fn detects_cycle() {
        let mut store = Store::default();
        let mut env = Environment::default();
        env.values.insert("A".into(), "${p.e.B}".into());
        env.values.insert("B".into(), "${p.e.A}".into());
        let mut envs = IndexMap::new();
        envs.insert("e".to_string(), env);
        store.projects.insert(
            "p".to_string(),
            Project {
                default_env: None,
                environments: envs,
            },
        );
        let err = resolve_at(&store, "p", "e", "A").unwrap_err();
        assert!(err.to_string().contains("circular"));
    }

    #[test]
    fn missing_reference_errors() {
        let store = Store::default();
        let origin = ("p".into(), "e".into(), "k".into());
        let err = resolve(&store, &origin, "${a.b.c}").unwrap_err();
        assert!(err.to_string().contains("missing"));
    }

    #[test]
    fn same_env_reference() {
        // ${secret} → same project + env
        let store = store_with(&[
            ("api", "dev", "HOST", "db.local"),
            ("api", "dev", "URL", "http://${HOST}:5432"),
        ]);
        assert_eq!(
            resolve_at(&store, "api", "dev", "URL").unwrap(),
            "http://db.local:5432"
        );
    }

    #[test]
    fn same_project_other_env_reference() {
        // ${env.secret} → same project, different env
        let store = store_with(&[
            ("api", "shared", "TOKEN", "abc123"),
            ("api", "dev", "AUTH", "Bearer ${shared.TOKEN}"),
        ]);
        assert_eq!(
            resolve_at(&store, "api", "dev", "AUTH").unwrap(),
            "Bearer abc123"
        );
    }

    #[test]
    fn relative_reference_uses_target_context() {
        // A (api/dev) → ${web.KEY} resolves to api/web/KEY, whose value uses a
        // bare ${BASE} that must resolve within api/web (not api/dev).
        let store = store_with(&[
            ("api", "web", "BASE", "from-web"),
            ("api", "web", "KEY", "[${BASE}]"),
            ("api", "dev", "A", "${web.KEY}"),
        ]);
        assert_eq!(resolve_at(&store, "api", "dev", "A").unwrap(), "[from-web]");
    }

    #[test]
    fn self_reference_is_a_cycle() {
        let store = store_with(&[("p", "e", "A", "${A}")]);
        let err = resolve_at(&store, "p", "e", "A").unwrap_err();
        assert!(err.to_string().contains("circular"));
    }

    #[test]
    fn empty_reference_errors() {
        let store = Store::default();
        let origin = ("p".into(), "e".into(), "k".into());
        assert!(resolve(&store, &origin, "${}").is_err());
        assert!(resolve(&store, &origin, "${a.}").is_err());
    }
}
