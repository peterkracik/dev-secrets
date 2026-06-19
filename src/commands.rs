//! Implementations of the non-interactive CLI subcommands.

use std::fs;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::cli::{Command, EnvAction, ProjectAction, SecretAction};
use crate::config::{self, Config};
use crate::model::{Environment, Project};
use crate::store::StoreHandle;
use crate::{envfile, resolve};

/// Dispatch a parsed subcommand. Returns `Ok(())` on success.
pub fn run(command: Command) -> Result<()> {
    match command {
        Command::Setup { folder } => setup(folder),
        Command::Project { action } => project(action),
        Command::Env { action } => env(action),
        Command::Secret { action } => secret(action),
        Command::Import {
            file,
            project,
            env,
            replace,
        } => import(file, project, env, replace),
        Command::Export {
            file,
            project,
            env,
            raw,
        } => export(file, project, env, raw),
        Command::Duplicate { project, from, to } => duplicate(project, from, to),
        Command::List => list_all(),
    }
}

fn setup(folder: Option<PathBuf>) -> Result<()> {
    let store_path = match folder {
        Some(f) => config::store_path_for_folder(&f),
        None => config::default_store_path(),
    };
    let cfg = Config {
        store_path: store_path.clone(),
    };
    config::save(&cfg)?;

    // Make sure the store file exists so subsequent runs are clean.
    let mut handle = StoreHandle::open_at(store_path.clone())?;
    handle.save()?;

    println!("dev-secrets initialised.");
    println!("Store location: {}", store_path.display());

    // Without an interactive terminal we cannot run the wizard.
    if !std::io::stdin().is_terminal() {
        println!("(non-interactive input — skipping project/environment setup)");
        return Ok(());
    }

    // Step 1: choose or create a project.
    let project = wizard_project(&mut handle)?;
    // Step 2: choose or create an environment within it.
    let env = wizard_env(&mut handle, &project)?;
    handle.save()?;

    // Offer to link the current folder so `devsecrets export` works from here.
    if let Ok(dir) = std::env::current_dir() {
        let dir = dir.to_string_lossy().into_owned();
        let already = handle
            .store
            .project(&project)
            .and_then(|p| p.folder.clone())
            .as_deref()
            == Some(dir.as_str());
        if !already {
            let answer = prompt(&format!("Link this folder to `{project}`?\n  {dir}\n[y/N]: "))?;
            if answer.eq_ignore_ascii_case("y") {
                if let Some(p) = handle.store.project_mut(&project) {
                    p.folder = Some(dir);
                }
                handle.save()?;
                println!("Linked.");
            }
        }
    }

    println!("\nSetup complete — project `{project}`, environment `{env}`.");
    println!("Add secrets with: devsecrets secret set -p {project} -e {env} KEY value");
    println!("Or browse interactively with: devsecrets");
    Ok(())
}

/// Read a trimmed line of input after printing `msg`.
fn prompt(msg: &str) -> Result<String> {
    print!("{msg}");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

/// Wizard step 1: select an existing project by number or create a new one
/// by typing its name. Returns the chosen project's name.
fn wizard_project(handle: &mut StoreHandle) -> Result<String> {
    println!("\nStep 1/2 — Project");
    let names: Vec<String> = handle.store.projects.keys().cloned().collect();
    if names.is_empty() {
        println!("  (no projects yet)");
    } else {
        for (i, n) in names.iter().enumerate() {
            println!("  {}) {}", i + 1, n);
        }
    }
    loop {
        let input = prompt("Select a number, or type a new project name: ")?;
        if input.is_empty() {
            continue;
        }
        if let Some(name) = pick_by_number(&input, &names) {
            return Ok(name);
        }
        if handle.store.projects.contains_key(&input) {
            println!("Project `{input}` already exists — selecting it.");
            return Ok(input);
        }
        handle
            .store
            .projects
            .insert(input.clone(), Project::default());
        println!("Created project `{input}`.");
        return Ok(input);
    }
}

/// Wizard step 2: select or create an environment within `project`.
fn wizard_env(handle: &mut StoreHandle, project: &str) -> Result<String> {
    println!("\nStep 2/2 — Environment in `{project}`");
    let names: Vec<String> = handle
        .store
        .project(project)
        .map(|p| p.environments.keys().cloned().collect())
        .unwrap_or_default();
    if names.is_empty() {
        println!("  (no environments yet)");
    } else {
        for (i, n) in names.iter().enumerate() {
            println!("  {}) {}", i + 1, n);
        }
    }
    loop {
        let input = prompt("Select a number, or type a new environment name: ")?;
        if input.is_empty() {
            continue;
        }
        if let Some(name) = pick_by_number(&input, &names) {
            return Ok(name);
        }
        let proj = handle
            .store
            .project_mut(project)
            .context("project disappeared")?;
        if proj.environments.contains_key(&input) {
            println!("Environment `{input}` already exists — selecting it.");
            return Ok(input);
        }
        proj.environments
            .insert(input.clone(), Environment::default());
        if proj.default_env.is_none() {
            proj.default_env = Some(input.clone());
        }
        println!("Created environment `{input}`.");
        return Ok(input);
    }
}

/// If `input` is a valid 1-based index into `names`, return that name.
fn pick_by_number(input: &str, names: &[String]) -> Option<String> {
    let idx: usize = input.parse().ok()?;
    if idx >= 1 && idx <= names.len() {
        Some(names[idx - 1].clone())
    } else {
        None
    }
}

fn project(action: ProjectAction) -> Result<()> {
    let mut handle = StoreHandle::open()?;
    match action {
        ProjectAction::Create { name, folder } => {
            if handle.store.projects.contains_key(&name) {
                bail!("project `{name}` already exists");
            }
            let folder = match folder {
                Some(f) => Some(abs_path(&f)?),
                None => None,
            };
            handle.store.projects.insert(
                name.clone(),
                Project {
                    folder,
                    ..Default::default()
                },
            );
            handle.save()?;
            println!("Created project `{name}`.");
        }
        ProjectAction::List => {
            print_project_list(&handle);
        }
        ProjectAction::Delete { name } => {
            if handle.store.projects.shift_remove(&name).is_none() {
                bail!("project `{name}` not found");
            }
            handle.save()?;
            println!("Deleted project `{name}`.");
        }
        ProjectAction::SetFolder { name, folder } => {
            let folder = match folder {
                Some(f) => abs_path(&f)?,
                None => abs_path(Path::new("."))?,
            };
            let proj = handle
                .store
                .project_mut(&name)
                .with_context(|| format!("project `{name}` not found"))?;
            proj.folder = Some(folder.clone());
            handle.save()?;
            println!("Project `{name}` is now linked to {folder}.");
        }
    }
    Ok(())
}

fn env(action: EnvAction) -> Result<()> {
    let mut handle = StoreHandle::open()?;
    match action {
        EnvAction::Create { project, name } => {
            let proj = handle
                .store
                .project_mut(&project)
                .with_context(|| format!("project `{project}` not found"))?;
            if proj.environments.contains_key(&name) {
                bail!("environment `{name}` already exists in `{project}`");
            }
            proj.environments
                .insert(name.clone(), Environment::default());
            if proj.default_env.is_none() {
                proj.default_env = Some(name.clone());
            }
            handle.save()?;
            println!("Created environment `{name}` in `{project}`.");
        }
        EnvAction::List { project } => {
            let proj = handle
                .store
                .project(&project)
                .with_context(|| format!("project `{project}` not found"))?;
            for (name, e) in &proj.environments {
                let default = if proj.default_env.as_deref() == Some(name) {
                    " (default)"
                } else {
                    ""
                };
                println!("{name}{default} — {} secrets", e.values.len());
            }
        }
        EnvAction::Delete { project, name } => {
            let proj = handle
                .store
                .project_mut(&project)
                .with_context(|| format!("project `{project}` not found"))?;
            if proj.environments.shift_remove(&name).is_none() {
                bail!("environment `{name}` not found in `{project}`");
            }
            if proj.default_env.as_deref() == Some(&name) {
                proj.default_env = proj.environments.keys().next().cloned();
            }
            handle.save()?;
            println!("Deleted environment `{name}` from `{project}`.");
        }
        EnvAction::SetDefault { project, name } => {
            let proj = handle
                .store
                .project_mut(&project)
                .with_context(|| format!("project `{project}` not found"))?;
            if !proj.environments.contains_key(&name) {
                bail!("environment `{name}` not found in `{project}`");
            }
            proj.default_env = Some(name.clone());
            handle.save()?;
            println!("Default environment for `{project}` is now `{name}`.");
        }
    }
    Ok(())
}

fn secret(action: SecretAction) -> Result<()> {
    let mut handle = StoreHandle::open()?;
    match action {
        SecretAction::Set {
            project,
            env,
            key,
            value,
        } => {
            let e = env_mut(&mut handle, &project, &env)?;
            e.values.insert(key.clone(), value);
            handle.save()?;
            println!("Set `{key}` in `{project}/{env}`.");
        }
        SecretAction::Get {
            project,
            env,
            key,
            raw,
        } => {
            let exists = handle.store.value(&project, &env, &key).is_some();
            if !exists {
                bail!("secret `{key}` not found in `{project}/{env}`");
            }
            if raw {
                println!("{}", handle.store.value(&project, &env, &key).unwrap());
            } else {
                println!(
                    "{}",
                    resolve::resolve_at(&handle.store, &project, &env, &key)?
                );
            }
        }
        SecretAction::List { project, env, show } => {
            let e = env_ref(&handle, &project, &env)?;
            for (key, value) in &e.values {
                if show {
                    println!("{key}={value}");
                } else {
                    println!("{key}={}", mask(value));
                }
            }
        }
        SecretAction::Delete { project, env, key } => {
            let e = env_mut(&mut handle, &project, &env)?;
            if e.values.shift_remove(&key).is_none() {
                bail!("secret `{key}` not found in `{project}/{env}`");
            }
            handle.save()?;
            println!("Deleted `{key}` from `{project}/{env}`.");
        }
    }
    Ok(())
}

fn import(file: PathBuf, project: String, env: String, replace: bool) -> Result<()> {
    let text = fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
    let parsed = envfile::parse(&text);

    let mut handle = StoreHandle::open()?;
    let proj = handle
        .store
        .projects
        .entry(project.clone())
        .or_insert_with(Project::default);
    let environment = proj
        .environments
        .entry(env.clone())
        .or_insert_with(Environment::default);
    if replace {
        environment.values.clear();
    }
    let count = parsed.len();
    for (k, v) in parsed {
        environment.values.insert(k, v);
    }
    if proj.default_env.is_none() {
        proj.default_env = Some(env.clone());
    }
    handle.save()?;
    println!("Imported {count} secrets into `{project}/{env}`.");
    Ok(())
}

fn export(
    file: Option<PathBuf>,
    project: Option<String>,
    env: Option<String>,
    raw: bool,
) -> Result<()> {
    let handle = StoreHandle::open()?;

    let project = match project {
        Some(p) => p,
        None => {
            let cwd = abs_path(Path::new("."))?;
            handle
                .store
                .project_for_folder(&cwd)
                .map(|s| s.to_string())
                .context(
                    "no project given and the current folder is not linked to a project \
                     (use --project or `devsecrets project set-folder`)",
                )?
        }
    };

    let proj = handle
        .store
        .project(&project)
        .with_context(|| format!("project `{project}` not found"))?;
    let env_name = proj
        .resolve_env(env.as_deref())
        .map(|s| s.to_string())
        .with_context(|| {
            format!("could not determine environment for `{project}` (set a default or pass --env)")
        })?;
    let environment = &proj.environments[&env_name];

    // Build the output, resolving references unless --raw.
    let mut resolved = indexmap::IndexMap::new();
    for key in environment.values.keys() {
        let value = if raw {
            environment.values[key].clone()
        } else {
            resolve::resolve_at(&handle.store, &project, &env_name, key)?
        };
        resolved.insert(key.clone(), value);
    }
    let output = envfile::serialize(&resolved);

    match file {
        Some(path) => {
            fs::write(&path, &output).with_context(|| format!("writing {}", path.display()))?;
            eprintln!(
                "Exported {} secrets from `{project}/{env_name}` to {}.",
                resolved.len(),
                path.display()
            );
        }
        None => {
            print!("{output}");
        }
    }
    Ok(())
}

fn duplicate(project: String, from: String, to: String) -> Result<()> {
    let mut handle = StoreHandle::open()?;
    let proj = handle
        .store
        .project_mut(&project)
        .with_context(|| format!("project `{project}` not found"))?;
    let source = proj
        .environments
        .get(&from)
        .with_context(|| format!("environment `{from}` not found in `{project}`"))?
        .clone();
    if proj.environments.contains_key(&to) {
        bail!("environment `{to}` already exists in `{project}`");
    }
    proj.environments.insert(to.clone(), source);
    handle.save()?;
    println!("Duplicated `{project}/{from}` to `{project}/{to}`.");
    Ok(())
}

fn list_all() -> Result<()> {
    let handle = StoreHandle::open()?;
    print_project_list(&handle);
    Ok(())
}

fn print_project_list(handle: &StoreHandle) {
    if handle.store.projects.is_empty() {
        println!("No projects yet. Create one with `devsecrets project create <name>`.");
        return;
    }
    for (name, proj) in &handle.store.projects {
        println!("{name}");
        for (env_name, e) in &proj.environments {
            let default = if proj.default_env.as_deref() == Some(env_name) {
                " *"
            } else {
                ""
            };
            println!("  {env_name}{default} — {} secrets", e.values.len());
        }
    }
}

// --- helpers ---------------------------------------------------------------

fn env_mut<'a>(
    handle: &'a mut StoreHandle,
    project: &str,
    env: &str,
) -> Result<&'a mut Environment> {
    let proj = handle
        .store
        .project_mut(project)
        .with_context(|| format!("project `{project}` not found"))?;
    proj.environments
        .get_mut(env)
        .with_context(|| format!("environment `{env}` not found in `{project}`"))
}

fn env_ref<'a>(handle: &'a StoreHandle, project: &str, env: &str) -> Result<&'a Environment> {
    let proj = handle
        .store
        .project(project)
        .with_context(|| format!("project `{project}` not found"))?;
    proj.environments
        .get(env)
        .with_context(|| format!("environment `{env}` not found in `{project}`"))
}

fn mask(value: &str) -> String {
    if value.len() <= 4 {
        "•".repeat(value.chars().count().max(1))
    } else {
        let visible: String = value.chars().take(2).collect();
        format!("{visible}{}", "•".repeat(6))
    }
}

/// Canonicalise a path to an absolute string (the path need not exist).
fn abs_path(p: &Path) -> Result<String> {
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()?.join(p)
    };
    // Best-effort canonicalisation; fall back to the joined path.
    let canonical = fs::canonicalize(&abs).unwrap_or(abs);
    Ok(canonical.to_string_lossy().into_owned())
}
