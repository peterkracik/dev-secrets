<h1 align="center">dev-secrets</h1>

<p align="center">
  A fast, <a href="https://github.com/nvim-telescope/telescope.nvim">Telescope</a>-style terminal UI
  (and full CLI) for managing your <strong>local development</strong> secrets — organized by
  <strong>project</strong> and <strong>environment</strong> instead of scattered <code>.env</code> files.
</p>

<p align="center">
  <code>devsecrets</code> &nbsp;·&nbsp; written in Rust &nbsp;·&nbsp; TUI + scriptable CLI
</p>

<p align="center">
  🌐 <a href="https://peterkracik.github.io/dev-secrets/"><strong>peterkracik.github.io/dev-secrets</strong></a>
  &nbsp;·&nbsp; <a href="https://github.com/peterkracik/dev-secrets/releases">Releases</a>
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
devsecrets export .env          # writes this folder's assigned environment to ./.env
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
- 🔭 **Telescope-style TUI** — centered floating window, fuzzy search, live preview.
- ⌨️ **Full CLI parity** — every action is scriptable.
- 📥 **Import** existing `.env` files (merge or replace).
- 📤 **Export** any environment to a file or stdout.
- 🔗 **References**: write `${project.env.KEY}` to reuse a value instead of
  duplicating it. Resolved (recursively, with cycle detection) on export.
- 📌 **Folder assignments** — `devsecrets setup` binds a folder to a
  project/env, so `devsecrets export` (and the TUI) "just know" what to use.
- 📋 **Clipboard** copy of a secret value or a whole environment.
- 📝 **Plain JSON store** you can read, diff, and back up yourself.

---

## Installation

dev-secrets is a single self-contained binary called `devsecrets`.

### Homebrew (recommended — macOS / Linux)

```sh
brew install peterkracik/tap/devsecrets
```

…which is shorthand for:

```sh
brew tap peterkracik/tap
brew install devsecrets
```

Upgrade later with `brew upgrade devsecrets`.

### Download a prebuilt binary

Grab the archive for your platform from the
[**Releases**](https://github.com/peterkracik/dev-secrets/releases) page (built
automatically by CI), or pull the latest release from the command line:

```sh
# Linux (x86_64)
curl -L https://github.com/peterkracik/dev-secrets/releases/latest/download/devsecrets-x86_64-unknown-linux-gnu.tar.gz | tar xz
sudo install devsecrets-x86_64-unknown-linux-gnu/devsecrets /usr/local/bin/

# macOS (Apple Silicon)
curl -L https://github.com/peterkracik/dev-secrets/releases/latest/download/devsecrets-aarch64-apple-darwin.tar.gz | tar xz
sudo install devsecrets-aarch64-apple-darwin/devsecrets /usr/local/bin/
```

Available targets: `x86_64-unknown-linux-gnu`, `aarch64-apple-darwin`,
`x86_64-apple-darwin`, and `x86_64-pc-windows-msvc` (`.zip`).

### Build from source

Requires [Rust](https://rustup.rs/) **1.74+** (the only build dependency — no
system libraries needed).

```sh
cargo install --git https://github.com/peterkracik/dev-secrets.git
# or, from a clone:
git clone https://github.com/peterkracik/dev-secrets.git && cd dev-secrets
cargo install --path .
```

This places `devsecrets` in `~/.cargo/bin` (ensure it's on your `PATH`).

### Verify

```sh
devsecrets --version
devsecrets --help
```

### Recommended: a short `ds` alias

`devsecrets` is a lot to type for a tool you reach for constantly. We recommend
aliasing it to **`ds`**. Add this to your shell's startup file (`~/.bashrc`,
`~/.zshrc`, …):

```sh
alias ds=devsecrets
```

Reload your shell (or `source` the file) and everything in this README works
with `ds`:

```sh
ds setup
ds secret list      # this folder's secrets
ds env list         # this folder's environments
ds export .env
```

The rest of the docs spell out `devsecrets` in full, but `ds` is interchangeable
once the alias is set.

---

## Quick start

```sh
# 1. In your project folder, run setup. It walks you through choosing
#    (or creating) a project and environment, and remembers this folder.
cd ~/code/api
devsecrets setup

# 2. Add some secrets
devsecrets secret set -p api -e dev DB_HOST localhost
devsecrets secret set -p api -e dev DB_PORT 5432

# 3. Export them when you need them — no flags, this folder is assigned
devsecrets export .env
```

…or just run the TUI and do all of the above interactively:

```sh
devsecrets
```

---

## Configuration

### Where data is stored

Everything lives under `~/.config/devsecrets/`:

| File            | Purpose                                                   |
|-----------------|-----------------------------------------------------------|
| `settings.toml` | Points at where the secrets store file lives.             |
| `meta.json`     | Folder → (project, environment) assignments (see below).  |
| `store.json`    | The secrets themselves (default location).                |

The config directory is platform-specific:

| OS            | Default location          |
|---------------|---------------------------|
| Linux / macOS | `~/.config/devsecrets/`   |
| Windows       | `%APPDATA%\devsecrets\`   |

You can override the base directory with the standard `XDG_CONFIG_HOME`
environment variable (Unix).

### `devsecrets setup` — assign a folder

`setup` configures the **current folder**: it asks (the first time) which
project and environment this folder belongs to, and remembers the
folder → (project, env) assignment in `meta.json`. From then on, whenever you
work in that folder, dev-secrets knows what to use — a bare `devsecrets export`
or launching the TUI jumps straight to the right project/environment.

```sh
cd ~/code/api
devsecrets setup        # wizard: pick/create project, then environment
# ...later, from the same folder:
devsecrets export .env  # no flags needed — uses this folder's project/env
```

The wizard:

1. **Project** — pick an existing project by number, or type a name to create one.
2. **Environment** — pick or create an environment within that project.

`setup` is skipped (no-op wizard) when input isn't an interactive terminal, so
it stays scriptable. You can also assign a different folder explicitly:
`devsecrets setup /path/to/folder`. Inside the TUI, press `f` to assign the
current folder to the selected project/environment.

### `devsecrets version` — where everything lives

```sh
devsecrets version
```

prints the version plus the config locations and current folder assignments:

```
devsecrets 0.1.0

Config dir:     ~/.config/devsecrets
Settings file:  ~/.config/devsecrets/settings.toml
Meta file:      ~/.config/devsecrets/meta.json
Store file:     ~/.config/devsecrets/store.json

Folder assignments:
  /home/you/code/api → api/dev
```

### Relocating the store

The store file location lives in `settings.toml` as `store_path`. To keep it
somewhere else (e.g. a synced folder shared across your machines), move the
file and point `store_path` at the new location:

```toml
# ~/.config/devsecrets/settings.toml
store_path = "/home/you/Dropbox/devsecrets/store.json"
```

(Remember: unencrypted — local test values only.)

---

## The TUI

Run `devsecrets` with no arguments. The interface is
[Telescope](https://github.com/nvim-telescope/telescope.nvim)-style: a
centered, bounded floating window (it never fills the whole terminal) with a
fuzzy-filter prompt, a results list with match highlighting, and a live
preview pane.

**Projects and environments share one tree** — projects with their
environments nested underneath — which makes fuzzy navigation fast:

```
▾ api        (2 env)
    – dev ★  3 keys
    – prod   3 keys
▸ web        (1 env)      ← collapsed
```

Press `/` to fuzzy-filter; it matches **both** project and environment names.
A match in an environment keeps its project header visible; a match on a
project keeps all of its environments. Projects can be collapsed to hide their
environments. Opening an environment drills into its **Secrets** screen.

| Key             | Action                                                |
|-----------------|-------------------------------------------------------|
| `/`             | Fuzzy-filter (matches projects **and** environments)  |
| `↑`/`k` `↓`/`j` | Move selection (`Ctrl-n`/`Ctrl-p` while searching)    |
| `→`/`l`         | Expand a project / open an environment → Secrets      |
| `←`/`h`         | Collapse a project / jump to the parent project       |
| `Enter`         | Toggle a project (collapse), or open an environment   |
| `Esc`           | Back out of the Secrets screen                         |
| `p`             | New project                                            |
| `n`             | New environment (under the selected project) / secret |
| `e`             | Edit a secret, or the selected env inline             |
| `a`             | Edit the whole environment inline (multi-line)        |
| `E`             | Edit the whole environment in `$EDITOR` (as `.env`)   |
| `g`             | Jump to the secret a `$ref:` value points at          |
| `c`             | Copy: secret value (on Secrets) / whole env (else)    |
| `C`             | Copy the whole environment as a `.env` document       |
| `d`             | Delete the selected project / env / secret (confirm)  |
| `y`             | Duplicate the selected environment                    |
| `i`             | Import a `.env` file into the selected environment    |
| `x`             | Export the selected env (path + format in one dialog) |
| `D`             | Set the selected environment as the project default   |
| `f`             | Assign the current folder to this project/env         |
| `s`             | Toggle showing / hiding secret values                 |
| `?`             | Help overlay                                          |
| `q` / `Ctrl-C`  | Quit                                                  |

New secrets are entered through a two-field form (**Key** and **Value** as
separate boxes, `Tab` to switch) so a value can be pasted on its own. Values
are masked by default — press `s` to reveal them. The preview pane shows the
selected item's details, including a secret's resolved value when it contains
`${...}` references.

### Copy to clipboard

Press `c` to copy the selected secret's resolved value, or `C` (also `c` when
not on a single secret) to copy the whole environment as a resolved `.env`
document. dev-secrets uses the system clipboard tool when available
(`pbcopy`, `wl-copy`, `xclip`/`xsel`, `clip`) and otherwise falls back to the
terminal's OSC 52 sequence, which also works over SSH and tmux.

### Editing a whole environment at once

You can edit the entire environment as one `.env` document — two ways:

- **Inline (`a`, or `e` when not on a single secret):** a built-in multi-line
  editor opens inside the TUI. Type freely; `Ctrl-S` saves and applies,
  `Esc` cancels. No external tools required.
- **External (`E`):** opens the environment in your editor (`$VISUAL`, then
  `$EDITOR`, falling back to `vi` / `notepad`). Save and quit to apply; quit
  without saving to cancel.

Either way, add/change/reorder/remove lines freely; on save the document is
parsed back and replaces the environment's contents. References like
`${project.env.KEY}` are preserved verbatim and resolved on export.

### Importing a `.env`

Press `i`, give a path, then choose how to apply it:

- **merge** — add new keys and **confirm each changed key** individually
  (`y` overwrite · `n` keep · `a` overwrite all · `q` stop);
- **overwrite** — add new keys and replace every changed key without asking;
- **replace** — clear the environment first, then load only the file.

The CLI mirrors this: `devsecrets import file.env` prompts per changed key in a
terminal, or use `--overwrite` / `--replace`. With no terminal (scripts), the
default leaves existing keys untouched unless you pass `--overwrite`.

---

## CLI reference

Everything below also works interactively in the TUI.

### Setup & info

```sh
devsecrets setup [<folder>]   # assign a folder to a project/env (wizard)
devsecrets version            # version + config locations + folder assignments
```

### Projects

```sh
devsecrets project create <name>
devsecrets project list
devsecrets project delete <name>
```

### Environments

```sh
devsecrets env create -p <project> <name>
devsecrets env list [-p <project>]
devsecrets env set-default -p <project> <name>
devsecrets env delete -p <project> <name>
```

`env list` drops the `-p` flag once the folder is assigned (`devsecrets
setup`): a bare `devsecrets env list` shows the environments of this folder's
project.

### Secrets

```sh
devsecrets secret set    -p <project> -e <env> <KEY> <VALUE> [--type text|number|json]
devsecrets secret get    -p <project> -e <env> <KEY> [--raw]
devsecrets secret list   [-p <project>] [-e <env>] [--mask]
devsecrets secret delete -p <project> -e <env> <KEY>
```

`get` resolves references by default (use `--raw` for the literal value).
`list` shows values by default; pass `--mask` to obscure them.

Like `export` and `import`, `secret list` drops the `-p`/`-e` flags once the
folder is assigned (`devsecrets setup`). From inside an assigned folder, a bare
`devsecrets secret list` shows that folder's secrets (`secrets` is also accepted
as an alias for `secret`):

```sh
cd ~/code/api
devsecrets secret list   # no flags — uses this folder's project/env
devsecrets secrets list  # same thing
```

**Value types.** Each secret has a type — `text` (default), `number`, or
`json` — which is validated on set. `--type number` rejects non-numbers and
`--type json` rejects invalid JSON. In the TUI, the new-secret form has a Type
field (`←/→` to change), and a `json` value is pretty-printed in the preview
when revealed (`s`).

### Import & export

```sh
# Import a .env file. Like export, -p/-e are optional once the folder is
# assigned (devsecrets setup). New keys are added automatically; for each
# CHANGED key you're asked whether to overwrite (interactive menu).
devsecrets import existing.env
devsecrets import existing.env -p api -e dev

# Skip the per-key prompts:
devsecrets import existing.env --overwrite   # overwrite all changed keys
devsecrets import existing.env --replace     # clear the env, then load the file

# Export to a file
devsecrets export .env -p api -e dev

# Export the project's default environment to stdout
devsecrets export -p api

# Export using this folder's assigned project + env (after `devsecrets setup`)
devsecrets export

# Choose a format explicitly, or let the file extension decide
devsecrets export --format json -p api -e dev      # to stdout as JSON
devsecrets export config.toml -p api -e dev        # inferred TOML
devsecrets export -p api --format shell            # export KEY=VALUE lines

# Export raw values (do not resolve ${...} references)
devsecrets export .env -p api --raw
```

**Formats** (`--format`, or auto-detected from the file extension):

| Format  | Looks like                | Extensions          |
|---------|---------------------------|---------------------|
| `env`   | `KEY=VALUE` (default)     | `.env`              |
| `shell` | `export KEY=VALUE`        | `.sh` `.bash` `.zsh`|
| `json`  | `{ "KEY": "VALUE" }`      | `.json`             |
| `toml`  | `KEY = "VALUE"`           | `.toml`             |

`shell` is handy for `eval "$(devsecrets export --format shell)"`.

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

A value may reference another secret so a shared value lives in exactly one
place. There are three forms, depending on how much context you need to
repeat — they resolve **relative to where the value lives**:

| Form                       | Resolves to                                   |
|----------------------------|-----------------------------------------------|
| `${SECRET}`                | same project, **same environment**            |
| `${env.SECRET}`            | same project, **another environment**         |
| `${project.env.SECRET}`    | **anywhere** in the store                     |

```sh
devsecrets secret set -p api -e dev  DB_HOST localhost
devsecrets secret set -p api -e dev  URL  'http://${DB_HOST}:5432'        # same env
devsecrets secret set -p api -e prod URL  'https://${dev.DB_HOST}/api'    # other env
devsecrets secret set -p api -e dev  DSN  '${infra.prod.DATABASE_URL}'    # cross-project

devsecrets secret get -p api -e dev URL
# → http://localhost:5432
```

- References are resolved at **export** time (and by `secret get`).
- They can be **nested** (a referenced value may contain more references),
  and each level resolves relative to *its own* project/env.
- **Cycles** and **missing targets** are detected and reported as errors.
- Use `--raw` on `export` / `get` to keep references literal.

**Autocomplete:** in the TUI, while editing a secret value (or the value field
of a new secret), type `${` to pop up a fuzzy-filterable list of available
secrets. Use `↑`/`↓`/`Tab` to choose and `Enter` to insert the right form
(`${SECRET}` / `${env.SECRET}` / `${project.env.SECRET}` depending on where it
lives); `Esc` dismisses the popup.

**Display & jump:** while values are hidden, references show as
`$ref:project.env.key` (e.g. `AUTH = $ref:shared.TOKEN`, with a coloured
`$ref:` prefix) and ordinary values are masked. Press `s` to reveal: ordinary
values appear and **references resolve to their actual value** (in the secrets
list and the environment preview). Press `g` on a referencing secret to jump
straight to the secret it points at.

---

## Loading secrets into your shell (direnv)

Because `devsecrets export` writes a standard `.env` (and, after
`devsecrets setup`, needs no arguments inside an assigned folder), it drops
straight into [direnv](https://direnv.net/).

Add an `.envrc` to your project folder:

```sh
# .envrc — regenerate .env from dev-secrets and load it
devsecrets export .env
dotenv .env
```

Then:

```sh
echo '.env' >> .gitignore   # don't commit the generated file
direnv allow
```

Now every time you `cd` into the project, direnv refreshes `.env` from the
folder's assigned project/environment and loads the variables into your shell;
they're unloaded when you leave.

Prefer not to write a file? Load straight from stdout instead:

```sh
# .envrc
set -a                       # export everything that follows
eval "$(devsecrets export)"  # uses this folder's assigned project/env
set +a
```

The same `devsecrets export .env` line works in a `Makefile`, a container
entrypoint, or a CI step.

---

## How the store looks on disk

It's just JSON — readable, diffable, and easy to back up. `store.json`:

```json
{
  "projects": {
    "api": {
      "default_env": "dev",
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

…and `meta.json` records folder assignments:

```json
{
  "assignments": {
    "/home/you/code/api": { "project": "api", "env": "dev" }
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

**Can a value be a JSON object, contain `=`, spaces, or newlines?**
Yes. Values are arbitrary strings and are stored verbatim. On `.env`
export/editing they are double-quoted and escaped only when needed (e.g. a
`{"a": 1}` JSON value, a multi-line cert, or a Windows path), so the
round-trip is lossless.

**What happens if I run a command before `setup`?**
Everything works — the store is created automatically on first run. `setup`
only records a folder → project/env assignment so you can drop the
`--project`/`--env` flags in that folder. Use explicit flags anywhere else, and
`devsecrets version` to see where the store lives.

---

## Development

```sh
cargo build         # debug build
cargo test          # unit + integration tests (tests/cli.rs drives the binary)
cargo clippy        # lints
cargo fmt           # format
```

CI (GitHub Actions) runs fmt, clippy, and the unit + integration tests on every
push/PR (`.github/workflows/ci.yml`). On pushes to `main` and on `v*` tags,
`.github/workflows/release.yml` cross-builds binaries for Linux, macOS
(Intel + Apple Silicon), and Windows, uploads them as build artifacts, and — for
tags — attaches them to a GitHub Release. The Homebrew tap formula is
[`Formula/devsecrets.rb`](Formula/devsecrets.rb).

Contributions and issues are welcome.

## License

MIT
