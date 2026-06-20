//! Implementations of the non-interactive CLI subcommands.

use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::cli::{Command, EnvAction, Format, ProjectAction, SecretAction};
use crate::config::{self, Settings};
use crate::meta;
use crate::model::{Environment, Project};
use crate::store::StoreHandle;
use crate::{envfile, resolve};

/// Dispatch a parsed subcommand. Returns `Ok(())` on success.
pub fn run(command: Command) -> Result<()> {
    match command {
        Command::Setup { folder } => setup(folder),
        Command::Version => version(),
        Command::Project { action } => project(action),
        Command::Env { action } => env(action),
        Command::Secret { action } => secret(action),
        Command::Import {
            file,
            project,
            env,
            overwrite,
            replace,
        } => import(file, project, env, overwrite, replace),
        Command::Export {
            file,
            project,
            env,
            format,
            raw,
        } => export(file, project, env, format, raw),
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

/// `devsecrets version` — version plus config locations and assignments.
fn version() -> Result<()> {
    let settings = config::load()?;
    let meta = meta::load()?;
    println!("devsecrets {}", env!("CARGO_PKG_VERSION"));
    println!();
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
    Ok(())
}

const CREATE_NEW: &str = "➕  Create new…";

/// Choose an existing name from `existing` or create a new one, using
/// dialoguer. Returns the chosen/created name.
fn pick_or_create(label: &str, existing: &[String]) -> Result<String> {
    use dialoguer::{theme::ColorfulTheme, Input, Select};
    let theme = ColorfulTheme::default();

    if existing.is_empty() {
        let name: String = Input::with_theme(&theme)
            .with_prompt(format!("New {label} name"))
            .interact_text()?;
        return Ok(name.trim().to_string());
    }

    let mut items: Vec<String> = existing.to_vec();
    items.push(CREATE_NEW.to_string());
    let choice = Select::with_theme(&theme)
        .with_prompt(format!("Select a {label}"))
        .items(&items)
        .default(0)
        .interact()?;

    if choice == existing.len() {
        let name: String = Input::with_theme(&theme)
            .with_prompt(format!("New {label} name"))
            .interact_text()?;
        Ok(name.trim().to_string())
    } else {
        Ok(existing[choice].clone())
    }
}

/// Wizard step 1: pick or create a project. Returns its name.
fn wizard_project(handle: &mut StoreHandle) -> Result<String> {
    let names: Vec<String> = handle.store.projects.keys().cloned().collect();
    let name = pick_or_create("project", &names)?;
    if name.is_empty() {
        bail!("project name cannot be empty");
    }
    handle.store.projects.entry(name.clone()).or_default();
    Ok(name)
}

/// Wizard step 2: pick or create an environment within `project`.
fn wizard_env(handle: &mut StoreHandle, project: &str) -> Result<String> {
    let names: Vec<String> = handle
        .store
        .project(project)
        .map(|p| p.environments.keys().cloned().collect())
        .unwrap_or_default();
    let name = pick_or_create("environment", &names)?;
    if name.is_empty() {
        bail!("environment name cannot be empty");
    }
    let proj = handle
        .store
        .project_mut(project)
        .context("project disappeared")?;
    proj.environments.entry(name.clone()).or_default();
    if proj.default_env.is_none() {
        proj.default_env = Some(name.clone());
    }
    Ok(name)
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
            let project = resolve_project(project)?;
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
            kind,
        } => {
            // Default to the existing type when not given (else text).
            let kind = kind.unwrap_or_else(|| {
                handle
                    .store
                    .projects
                    .get(&project)
                    .and_then(|p| p.environments.get(&env))
                    .map(|e| e.kind(&key))
                    .unwrap_or_default()
            });
            if let Err(msg) = kind.validate(&value) {
                bail!("{msg}");
            }
            let e = env_mut(&mut handle, &project, &env)?;
            e.values.insert(key.clone(), value);
            e.set_kind(&key, kind);
            handle.save()?;
            println!("Set `{key}` in `{project}/{env}` ({}).", kind.label());
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
        SecretAction::List {
            project,
            env,
            mask: masked,
        } => {
            let (project, env) = resolve_target(project, env)?;
            let e = env_ref(&handle, &project, &env)?;
            for (key, value) in &e.values {
                if masked {
                    println!("{key}={}", mask(value));
                } else {
                    println!("{key}={value}");
                }
            }
        }
        SecretAction::Delete { project, env, key } => {
            let e = env_mut(&mut handle, &project, &env)?;
            if e.values.shift_remove(&key).is_none() {
                bail!("secret `{key}` not found in `{project}/{env}`");
            }
            e.types.shift_remove(&key);
            handle.save()?;
            println!("Deleted `{key}` from `{project}/{env}`.");
        }
    }
    Ok(())
}

fn import(
    file: PathBuf,
    project: Option<String>,
    env: Option<String>,
    overwrite: bool,
    replace: bool,
) -> Result<()> {
    let text = fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
    let parsed = envfile::parse(&text);

    let mut handle = StoreHandle::open()?;
    // Fall back to this folder's assignment when project/env are omitted.
    let (project, env) = resolve_target(project, env)?;

    let proj = handle.store.projects.entry(project.clone()).or_default();
    if proj.default_env.is_none() {
        proj.default_env = Some(env.clone());
    }
    let environment = proj.environments.entry(env.clone()).or_default();

    // --replace: clear the environment entirely, then load the file.
    if replace {
        let count = parsed.len();
        environment.values.clear();
        environment.values.extend(parsed);
        handle.save()?;
        println!("Imported {count} secrets into `{project}/{env}` (replaced env).");
        return Ok(());
    }

    // Otherwise add new keys, and decide what to do with changed keys.
    let mut added = 0usize;
    let mut conflicts: Vec<(String, String)> = Vec::new(); // (key, new_value)
    for (k, v) in parsed {
        match environment.values.get(&k) {
            None => {
                environment.values.insert(k, v);
                added += 1;
            }
            Some(old) if *old != v => conflicts.push((k, v)),
            Some(_) => {}
        }
    }

    let interactive = std::io::stdin().is_terminal();
    let mut updated = 0usize;
    let mut skipped = 0usize;
    let mut overwrite_rest = overwrite;

    for (key, new_value) in conflicts {
        let do_update = if overwrite_rest {
            true
        } else if !interactive {
            // No TTY and no --overwrite: keep existing, report at the end.
            false
        } else {
            let old = environment.values.get(&key).cloned().unwrap_or_default();
            match conflict_choice(&key, &old, &new_value)? {
                ConflictChoice::Overwrite => true,
                ConflictChoice::Keep => false,
                ConflictChoice::OverwriteAll => {
                    overwrite_rest = true;
                    true
                }
                ConflictChoice::Stop => break,
            }
        };
        if do_update {
            environment.values.insert(key, new_value);
            updated += 1;
        } else {
            skipped += 1;
        }
    }

    handle.save()?;
    println!(
        "Imported into `{project}/{env}`: {added} added, {updated} updated, {skipped} unchanged."
    );
    if skipped > 0 && !interactive && !overwrite {
        println!("({skipped} existing key(s) left as-is; pass --overwrite to update them.)");
    }
    Ok(())
}

enum ConflictChoice {
    Overwrite,
    Keep,
    OverwriteAll,
    Stop,
}

/// Ask the user what to do about a single changed key.
fn conflict_choice(key: &str, old: &str, new: &str) -> Result<ConflictChoice> {
    use dialoguer::{theme::ColorfulTheme, Select};
    let prompt = format!("`{key}` differs:\n    old: {old}\n    new: {new}\n  what now?");
    let items = [
        "Keep existing",
        "Overwrite",
        "Overwrite all remaining",
        "Stop",
    ];
    let choice = Select::with_theme(&ColorfulTheme::default())
        .with_prompt(prompt)
        .items(&items)
        .default(0)
        .interact()?;
    Ok(match choice {
        1 => ConflictChoice::Overwrite,
        2 => ConflictChoice::OverwriteAll,
        3 => ConflictChoice::Stop,
        _ => ConflictChoice::Keep,
    })
}

/// Resolve a (project, env) target from explicit args, falling back to the
/// current folder's assignment. Used by import (and mirrors export's logic).
fn resolve_target(project: Option<String>, env: Option<String>) -> Result<(String, String)> {
    let assignment = meta::load()?.get(&meta::current_dir()?).cloned();
    let project = project
        .or_else(|| assignment.as_ref().map(|a| a.project.clone()))
        .context(
            "no project given and this folder isn't assigned \
             (run `devsecrets setup` here, or pass --project)",
        )?;
    let env = env
        .or_else(|| {
            assignment
                .as_ref()
                .filter(|a| a.project == project)
                .map(|a| a.env.clone())
        })
        .context(
            "no environment given and this folder isn't assigned \
             (run `devsecrets setup` here, or pass --env)",
        )?;
    Ok((project, env))
}

/// Resolve a project from an explicit arg, falling back to the current
/// folder's assignment. Used by listings that only need a project.
fn resolve_project(project: Option<String>) -> Result<String> {
    let assignment = meta::load()?.get(&meta::current_dir()?).cloned();
    project.or_else(|| assignment.map(|a| a.project)).context(
        "no project given and this folder isn't assigned \
             (run `devsecrets setup` here, or pass --project)",
    )
}

fn export(
    file: Option<PathBuf>,
    project: Option<String>,
    env: Option<String>,
    format: Option<Format>,
    raw: bool,
) -> Result<()> {
    let handle = StoreHandle::open()?;

    // Fall back to this folder's assignment for project/env when not given.
    let assignment = meta::load()?.get(&meta::current_dir()?).cloned();

    let project = match project {
        Some(p) => p,
        None => assignment.as_ref().map(|a| a.project.clone()).context(
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
    // Choose the format: explicit flag, else the file extension, else env.
    let fmt = format
        .or_else(|| file.as_deref().and_then(format_from_path))
        .unwrap_or(Format::Env);
    let output = render(&resolved, fmt)?;

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

/// Guess an output format from a file extension.
pub fn format_from_path(path: &Path) -> Option<Format> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("json") => Some(Format::Json),
        Some("toml") => Some(Format::Toml),
        Some("sh" | "bash" | "zsh") => Some(Format::Shell),
        Some("env") => Some(Format::Env),
        _ => None,
    }
}

/// Render resolved key/value pairs in the requested format.
pub fn render(values: &indexmap::IndexMap<String, String>, fmt: Format) -> Result<String> {
    Ok(match fmt {
        Format::Env => envfile::serialize(values),
        Format::Shell => values
            .iter()
            .map(|(k, v)| format!("export {}\n", envfile::kv_line(k, v)))
            .collect(),
        Format::Json => {
            let mut s = serde_json::to_string_pretty(values)?;
            s.push('\n');
            s
        }
        Format::Toml => toml::to_string_pretty(values)?,
    })
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
    let len = value.chars().count();
    if len <= 4 {
        return "•".repeat(len.max(1));
    }
    // Only hint with leading alphanumerics so structured values don't leak.
    let visible: String = value
        .chars()
        .take(2)
        .take_while(|c| c.is_alphanumeric())
        .collect();
    format!("{visible}{}", "•".repeat(6))
}
