# dev-secrets

A small **k9s-style TUI + CLI** for managing *local development* secrets,
organized by **project** and **environment** instead of scattered `.env`
files per folder.

> ⚠️ Not for real secrets. The store is plain JSON on disk and **not
> encrypted**. It is meant for local/test values only.

## Why

Instead of copying `.env` files around and keeping them in sync per folder,
you keep everything in one place, grouped by project. When you start working
you just run:

```sh
devsecrets export .env
```

…and the right environment is written out.

## Concepts

```
Project              e.g. "api"
└── Environment      e.g. "dev", "staging", "prod"  (a.k.a. instance)
    └── Secrets      KEY = VALUE pairs
```

- A project can have a **default environment** (used by `export` when you
  don't pass `--env`).
- A project can be **linked to a folder**, so running `devsecrets export`
  *inside that folder* auto-selects the project.
- Values can **reference** other secrets with `${project.env.KEY}` so you
  don't have to duplicate shared values. References are resolved on export
  (use `--raw` to keep them literal).

## Install

```sh
cargo install --path .
# or
cargo build --release   # binary at target/release/devsecrets
```

## First run

```sh
devsecrets setup                 # store at ~/.config/dev-secrets/store.json
devsecrets setup ~/my-secrets    # …or keep the store in a folder you choose
```

If you skip `setup`, the default location is used automatically.

## TUI

Just run `devsecrets` with no arguments. Three panes — Projects →
Environments → Secrets — like k9s.

| Key            | Action                                             |
|----------------|----------------------------------------------------|
| `↑/k` `↓/j`    | move selection                                     |
| `→/l` `Enter`  | drill in (Projects → Envs → Secrets)               |
| `←/h` `Tab`    | go back / cycle panes                              |
| `n`            | new project / env / secret (depends on focus)      |
| `e` / `Enter`  | edit secret value                                  |
| `d`            | delete focused item (with confirmation)            |
| `y`            | duplicate environment                              |
| `i`            | import a `.env` file into the selected env          |
| `x`            | export the selected env to a `.env` file            |
| `D`            | set selected env as the project default            |
| `f`            | assign a working folder to the project             |
| `s`            | toggle showing / hiding values                     |
| `?`            | help                                               |
| `q` / `Ctrl-C` | quit                                               |

## CLI

Everything in the TUI is scriptable:

```sh
# Projects
devsecrets project create api --folder .
devsecrets project list
devsecrets project set-folder api ~/code/api
devsecrets project delete api

# Environments
devsecrets env create -p api dev
devsecrets env list -p api
devsecrets env set-default -p api dev
devsecrets env delete -p api dev

# Secrets
devsecrets secret set -p api -e dev DB_HOST localhost
devsecrets secret set -p api -e dev API_URL 'http://${api.dev.DB_HOST}:5432'
devsecrets secret get -p api -e dev API_URL          # resolved
devsecrets secret get -p api -e dev API_URL --raw    # literal
devsecrets secret list -p api -e dev --show
devsecrets secret delete -p api -e dev DB_HOST

# Import / export
devsecrets import existing.env -p api -e dev          # merge (use --replace to overwrite)
devsecrets export .env -p api -e dev                  # to file
devsecrets export -p api                              # default env -> stdout
devsecrets export                                     # project inferred from current folder

# Duplicate an environment
devsecrets duplicate -p api dev staging

# Overview
devsecrets list
```

## References

A value like:

```
API_URL=http://${shared.common.DB_HOST}:5432
```

resolves `${shared.common.DB_HOST}` from the `shared` project's `common`
environment at export time. References can be nested; cycles are detected
and reported.

## Storage

- App config: `~/.config/dev-secrets/config.json` (points at the store).
- Data store: `~/.config/dev-secrets/store.json` by default, or wherever
  `setup` put it. Plain, human-readable JSON.

## License

MIT
