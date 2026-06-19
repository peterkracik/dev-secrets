<h1 align="center">dev-secrets</h1>

<p align="center">
  A fast, <a href="https://k9scli.io/">k9s</a>-style terminal UI (and full CLI) for managing
  your <strong>local development</strong> secrets — organized by <strong>project</strong> and
  <strong>environment</strong> instead of scattered <code>.env</code> files.
</p>

<p align="center">
  <code>devsecrets</code> &nbsp;·&nbsp; written in Rust &nbsp;·&nbsp; TUI + scriptable CLI
</p>

---

## What is this?

If you work on several projects, each with a few environments (`dev`,
`staging`, `prod`, a teammate's setup, …), you usually end up with a mess of
`.env` files copied between folders and slowly drifting out of sync.

**dev-secrets** keeps all of those values in **one place**, grouped logically:

```
Project              e.g. "api"
└── Environment      e.g. "dev", "staging", "prod"   (an "instance")
    └── Secrets      KEY = VALUE pairs
```

When you sit down to work, you export the right set on demand:

```sh
devsecrets export .env          # writes the default environment to ./.env
```

Everything you can do in the interactive TUI you can also do from the CLI, so
it fits into scripts and Makefiles just as well as interactive use.

> [!WARNING]
> **This is not a secrets manager for production credentials.**
> The store is plain, human-readable JSON on your disk and is **not
> encrypted**. It is designed for *local, non-sensitive, test* values only
> (think `localhost` URLs, dummy API keys, dev ports). Do not put real
> production secrets in here.

---

## Features

- 🗂️ **Organized by project → environment → secret**, not by folder.
- 🖥️ **k9s-style TUI** — three master/detail panes, single-key actions, live editing.
- ⌨️ **Full CLI parity** — every action is scriptable.
- 📥 **Import** existing `.env` files (merge or replace).
- 📤 **Export** any environment to a file or stdout.
- 🔗 **References**: write `${project.env.KEY}` to reuse a value instead of
  duplicating it. Resolved (recursively, with cycle detection) on export.
- 📌 **Default environment** per project + **folder linking** so `devsecrets
  export` "just knows" which project you mean inside a project directory.
- 📝 **Plain JSON store** you can read, diff, and back up yourself.

---

## Installation

dev-secrets is a single self-contained binary called `devsecrets`.

### Requirements

- [Rust](https://rustup.rs/) **1.74 or newer** (stable). That's the only
  build dependency — no system libraries required.

### Option 1 — install from source with Cargo (recommended)

```sh
git clone https://github.com/peterkracik/localenvs.git
cd localenvs
cargo install --path .
```

This builds an optimized binary and places `devsecrets` in
`~/.cargo/bin` (make sure that directory is on your `PATH`).

### Option 2 — install directly from Git

```sh
cargo install --git https://github.com/peterkracik/localenvs.git
```

### Option 3 — build a release binary manually

```sh
git clone https://github.com/peterkracik/localenvs.git
cd localenvs
cargo build --release
# binary is at ./target/release/devsecrets — copy it anywhere on your PATH:
sudo cp target/release/devsecrets /usr/local/bin/
```

### Verify

```sh
devsecrets --version
devsecrets --help
```

---

## Quick start

```sh
# 1. (Optional) choose where the store lives. Skip to use the default.
devsecrets setup

# 2. Create a project and an environment
devsecrets project create api
devsecrets env create -p api dev

# 3. Add some secrets
devsecrets secret set -p api -e dev DB_HOST localhost
devsecrets secret set -p api -e dev DB_PORT 5432

# 4. Export them when you need them
devsecrets export .env -p api -e dev
```

…or just run the TUI and do all of the above interactively:

```sh
devsecrets
```

---

## Configuration

### Where data is stored

dev-secrets keeps two files under your OS config directory:

| File          | Purpose                                            |
|---------------|----------------------------------------------------|
| `config.json` | Small pointer file — remembers where the store is. |
| `store.json`  | The actual data (projects, environments, secrets). |

The default config directory is platform-specific:

| OS      | Default location                                      |
|---------|-------------------------------------------------------|
| Linux   | `~/.config/dev-secrets/`                              |
| macOS   | `~/Library/Application Support/dev-secrets/`          |
| Windows | `%APPDATA%\dev-secrets\`                              |

On Linux/macOS you can override the base directory with the standard
`XDG_CONFIG_HOME` environment variable.

### `devsecrets setup`

Run `setup` once to initialize the app and, optionally, to keep the store
somewhere other than the default:

```sh
devsecrets setup                      # use the default location
devsecrets setup ~/Dropbox/devsecrets # keep store.json in a folder you choose
devsecrets setup ~/secrets/store.json # or point at an exact file path
```

- Pass a **directory** → the store is created as `store.json` inside it.
- Pass a **`.json` file path** → that exact file is used.

Putting the store in a synced folder (Dropbox, iCloud, a private Git repo,
etc.) is an easy way to share dev values across your own machines. (Remember:
unencrypted — local test values only.)

If you never run `setup`, dev-secrets just uses the default location
automatically the first time you run it.

### Linking a project to a folder

Associate a project with a working directory so `devsecrets export` (with no
`--project`) auto-selects it when run from there:

```sh
devsecrets project create api --folder ~/code/api
# or for an existing project:
devsecrets project set-folder api ~/code/api

cd ~/code/api
devsecrets export .env          # knows this is the "api" project
```

---

## The TUI

Run `devsecrets` with no arguments. You get three panes — **Projects →
Environments → Secrets** — and navigate left-to-right as you drill in.

| Key             | Action                                              |
|-----------------|-----------------------------------------------------|
| `↑`/`k` `↓`/`j` | Move selection                                      |
| `→`/`l` `Enter` | Drill in (Projects → Envs → Secrets)                |
| `←`/`h`         | Go back                                             |
| `Tab`           | Cycle focus between panes                           |
| `n`             | New project / env / secret (based on focused pane)  |
| `e` / `Enter`   | Edit the selected secret's value                    |
| `E`             | Edit the whole environment in `$EDITOR` (as `.env`) |
| `d`             | Delete the focused item (asks for confirmation)     |
| `y`             | Duplicate the selected environment                  |
| `i`             | Import a `.env` file into the selected environment  |
| `x`             | Export the selected environment to a `.env` file    |
| `D`             | Set the selected environment as the project default |
| `f`             | Assign a working folder to the project              |
| `s`             | Toggle showing / hiding secret values               |
| `?`             | Help overlay                                        |
| `q` / `Ctrl-C`  | Quit                                                |

New secrets are entered as `KEY=VALUE`. Values are masked by default — press
`s` to reveal them.

### Bulk-editing an environment in your editor

Press `E` with an environment selected to open **the entire environment** in
your editor (`$VISUAL`, then `$EDITOR`, falling back to `vi` / `notepad`) as a
normal `.env` document. Add, change, reorder, or remove lines freely; when you
save and quit, the file is parsed back and replaces the environment's
contents. Quit without saving to cancel — nothing changes. References like
`${project.env.KEY}` are preserved verbatim and resolved on export as usual.

---

## CLI reference

Everything below also works interactively in the TUI.

### Projects

```sh
devsecrets project create <name> [--folder <dir>]
devsecrets project list
devsecrets project set-folder <name> [<dir>]   # omit <dir> to use current dir
devsecrets project delete <name>
```

### Environments

```sh
devsecrets env create -p <project> <name>
devsecrets env list -p <project>
devsecrets env set-default -p <project> <name>
devsecrets env delete -p <project> <name>
```

### Secrets

```sh
devsecrets secret set    -p <project> -e <env> <KEY> <VALUE>
devsecrets secret get    -p <project> -e <env> <KEY> [--raw]
devsecrets secret list   -p <project> -e <env> [--show]
devsecrets secret delete -p <project> -e <env> <KEY>
```

By default `get` resolves references and `list` masks values; use `--raw` /
`--show` respectively to see literal values.

### Import & export

```sh
# Import a .env file (merges by default; --replace overwrites the env first)
devsecrets import existing.env -p api -e dev
devsecrets import existing.env -p api -e dev --replace

# Export to a file
devsecrets export .env -p api -e dev

# Export the project's default environment to stdout
devsecrets export -p api

# Export, inferring the project from the current folder
devsecrets export

# Export raw values (do not resolve ${...} references)
devsecrets export .env -p api --raw
```

### Duplicate an environment

```sh
devsecrets duplicate -p api dev staging
```

### Overview

```sh
devsecrets list
```

---

## References: share values without duplicating

A value may contain one or more references of the form
`${project.env.KEY}`. They point at any secret in the store, so a shared
value lives in exactly one place:

```sh
devsecrets secret set -p shared -e common DB_HOST db.local
devsecrets secret set -p api    -e dev    API_URL 'http://${shared.common.DB_HOST}:5432'

devsecrets secret get -p api -e dev API_URL
# → http://db.local:5432
```

- References are resolved at **export** time (and by `secret get`).
- They can be **nested** (a referenced value may contain more references).
- **Cycles** and **missing targets** are detected and reported as errors.
- Use `--raw` on `export` / `get` to keep references literal.

---

## How the store looks on disk

It's just JSON — readable, diffable, and easy to back up:

```json
{
  "projects": {
    "api": {
      "default_env": "dev",
      "folder": "/home/you/code/api",
      "environments": {
        "dev": {
          "values": {
            "DB_HOST": "localhost",
            "DB_PORT": "5432"
          }
        }
      }
    }
  }
}
```

---

## FAQ

**Is it safe to commit my store to Git?**
Only if it truly contains non-sensitive test values, and ideally in a private
repo. The file is not encrypted, so treat it like any other plaintext config.

**Can I edit `store.json` by hand?**
Yes. Keep it valid JSON; key order is preserved on save so diffs stay clean.

**Does it support comments / `export FOO=bar` in `.env` files?**
On import, yes — blank lines, `#` comments, an optional `export ` prefix, and
single/double quoted values are all understood.

**What happens if I run a command before `setup`?**
dev-secrets initializes the default location automatically, so `setup` is
only needed if you want a custom store location.

---

## Development

```sh
cargo build         # debug build
cargo test          # run unit tests
cargo clippy        # lints
cargo fmt           # format
```

Contributions and issues are welcome.

## License

MIT
