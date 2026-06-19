//! Implementations of the non-interactive CLI subcommands.

use std::fs;
use std::io::{IsTerminal, Write};
use std::path::PathBuf;

use anyhow::{bail, Context, Result};

use crate::cli::{Command, EnvAction, ProjectAction, SecretAction, SettingsAction};
use crate::config::{self, Settings};
use crate::meta;
use crate::model::{Environment, Project};
use crate::store::StoreHandle;
use crate::{envfile, resolve};

/// Dispatch a parsed subcommand. Returns `Ok(())` on success.
pub fn run(command: Command) -> Result<()> {
    match command {
        Command::Setup { folder } => setup(folder),
        Command::Settings { action } => settings(action),
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

/// Assign a folder (default: the current directory) to a project + environment.
fn setup(folder: Option<PathBuf>) -> Result<()> {
    // Make sure settings + store exist so the rest of the app is happy.
    if !config::is_initialised() {
        config::save(&Settings::default())?;
    }
    let mut handle = StoreHandle::open()?;
    handle.save()?;

    let dir = match folder {
        Some(f) => meta::canonical_dir(&f)?,
        None => meta::current_dir()?,
    };

    let mut m = meta::load()?;
    if let Some(existing) = m.get(&dir) {
        println!(
            "This folder is currently assigned to `{}/{}`.",
            existing.project, existing.env
        );
    }

    if !std::io::stdin().is_terminal() {
        println!("(non-interactive input — cannot run the setup wizard)");
        println!("Folder: {dir}");
        return Ok(());
    }

    println!("Configuring folder: {dir}\n");

    // Step 1: choose or create a project.
    let project = wizard_project(&mut handle)?;
    // Step 2: choose or create an environment within it.
    let env = wizard_env(&mut handle, &project)?;
    handle.save()?;

    // Record the folder → (project, env) assignment.
    m.set(dir.clone(), project.clone(), env.clone());
    meta::save(&m)?;

    println!("\nSetup complete.");
    println!("Folder {dir} is now assigned to `{project}/{env}`.");
    println!("From here you can just run: devsecrets export .env");
    Ok(())
}

/// `devsecrets settings [show|store <path>]`.
fn settings(action: Option<SettingsAction>) -> Result<()> {
    match action.unwrap_or(SettingsAction::Show) {
        SettingsAction::Show => {
            let settings = config::load()?;
            let meta = meta::load()?;
            println!("Config dir:     {}", config::config_dir().display());
            println!("Settings file:  {}", config::settings_file().display());
            println!("Meta file:      {}", meta::meta_file().display());
            println!("Store file:     {}", settings.store_path.display());
            println!("\nFolder assignments:");
            if meta.assignments.is_empty() {
                println!("  (none — run `devsecrets setup` in a project folder)");
            } else {
                for (folder, a) in &meta.assignments {
                    println!("  {folder} → {}/{}", a.project, a.env);
                }
            }
        }
        SettingsAction::Store { path } => {
            let new_path = config::store_path_for(&meta::canonical_path(&path)?);
            let mut settings = config::load()?;
            let old_path = settings.store_path.clone();
            if new_path == old_path {
                println!("Store is already at {}.", new_path.display());
                return Ok(());
            }
            if let Some(parent) = new_path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("creating {}", parent.display()))?;
            }
            // Move existing data to the new location if there is any.
            if old_path.exists() && !new_path.exists() && fs::rename(&old_path, &new_path).is_err() {
                fs::copy(&old_path, &new_path).with_context(|| {
                    format!("copying {} to {}", old_path.display(), new_path.display())
                })?;
                let _ = fs::remove_file(&old_path);
            }
            settings.store_path = new_path.clone();
            config::save(&settings)?;
            // Ensure the file exists at the new location.
            StoreHandle::open_at(new_path.clone())?.save()?;
            println!("Store is now kept at {}.", new_path.display());
        }
    }
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
        ProjectAction::Create { name } => {
            if handle.store.projects.contains_key(&name) {
                bail!("project `{name}` already exists");
            }
            handle
                .store
                .projects
                .insert(name.clone(), Project::default());
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

    // Fall back to this folder's assignment for project/env when not given.
    let assignment = meta::load()?.get(&meta::current_dir()?).cloned();

    let project = match project {
        Some(p) => p,
        None => assignment
            .as_ref()
            .map(|a| a.project.clone())
            .context(
                "no project given and this folder isn't assigned \
                 (run `devsecrets setup` here, or pass --project)",
            )?,
    };

    let proj = handle
        .store
        .project(&project)
        .with_context(|| format!("project `{project}` not found"))?;

    // env: explicit flag, else the folder's assigned env (same project), else default.
    let requested_env = env.or_else(|| {
        assignment
            .as_ref()
            .filter(|a| a.project == project)
            .map(|a| a.env.clone())
    });
    let env_name = proj
        .resolve_env(requested_env.as_deref())
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
