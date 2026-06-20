# CLAUDE.md

Project memory for **dev-secrets** — read this first when working in this repo.

## What this is

`dev-secrets` (binary name: `devsecrets`) is a fast, **local-first**,
Telescope-style **terminal UI + CLI** for managing **local development**
secrets, organized by **project → environment → secret** instead of scattered
`.env` files. Written in **Rust**, shipped as a single self-contained binary.

> ⚠️ It is **not** a production secrets manager. The store is plain,
> **unencrypted** JSON on disk, intended for local, non-sensitive test values
> only. Keep that framing in docs and copy.

User-facing documentation lives in `README.md` — it is the source of truth for
behavior; keep it in sync when you change features.

## Repository layout

```
src/                Rust application
  main.rs           Entry point: parse CLI, init config, dispatch TUI vs command
  cli.rs            clap argument/subcommand definitions
  commands.rs       Non-interactive CLI command implementations (CLI parity)
  tui.rs            The Telescope-style terminal UI (ratatui) — by far the largest module
  model.rs          Core data model (projects, environments, secrets)
  store.rs          Load/save the JSON secrets store
  config.rs         settings.toml + config-dir resolution
  meta.rs           Folder → (project, env) assignments (meta.json)
  resolve.rs        ${...} reference resolution (recursive, cycle detection)
  envfile.rs        .env / shell / json / toml import & export formats
  fuzzy.rs          Fuzzy matching for the TUI filter
  clip.rs           Clipboard (pbcopy/wl-copy/xclip/xsel/clip) + OSC 52 fallback
tests/cli.rs        Integration tests that drive the built binary
Formula/            Homebrew tap formula (devsecrets.rb)
docs/               Static landing page (GitHub Pages) — see below
.github/workflows/  ci.yml (fmt/clippy/test), release.yml (cross-build), pages.yml (site)
```

## Build, test, lint (Rust)

CI runs all three on every push/PR; match it locally before pushing:

```sh
cargo fmt --all --check      # formatting (CI uses --check)
cargo clippy --all-targets -- -D warnings   # lints; warnings are errors in CI
cargo test --all --locked    # unit + integration tests
cargo build                  # debug build
cargo build --release        # optimized (strip + lto)
```

- Minimum Rust: **1.74+**.
- Integration tests in `tests/cli.rs` invoke the compiled `devsecrets` binary.
  When changing CLI flags or output, update those tests.

## Key concepts (when editing features)

- **Store** (`store.json`): the data, plain JSON, key order preserved for clean
  diffs. Default under `~/.config/devsecrets/` (XDG-aware; `%APPDATA%` on Windows).
- **Meta** (`meta.json`): folder → (project, env) assignments from
  `devsecrets setup`, so `export`/TUI "just know" what to use in a folder.
- **References**: `${SECRET}`, `${env.SECRET}`, `${project.env.SECRET}` resolved
  in `resolve.rs` at export / `secret get` time. Nestable, with cycle and
  missing-target detection. `--raw` keeps them literal.
- **Formats**: `env` (default), `shell`, `json`, `toml` — see `envfile.rs`,
  auto-detected from file extension or `--format`.
- **CLI ↔ TUI parity**: every interactive action has a scriptable CLI
  equivalent. Preserve this — if you add a TUI action, add/confirm the CLI path.

## Conventions

- Keep `README.md` authoritative and in sync with behavior changes.
- Don't introduce a system-library dependency (the "single self-contained
  binary, no system libs" property is a selling point). Pure-Rust crates only.
- Emphasize **lightweight** and **local-first** in any user-facing copy.

## Landing page (`docs/`)

A static marketing site served by **GitHub Pages** from the `docs/` folder.

- **Stack**: hand-written `index.html` + **Tailwind CSS v4**, compiled to a
  static `docs/tailwind.css` (no runtime/CDN). Small vanilla `script.js`
  (clipboard + scroll reveal). `favicon.svg` is the logo.
- **Source vs output**: `docs/src/input.css` is the Tailwind source (tokens in
  `@theme`, components in `@layer`); `docs/tailwind.css` is the committed build
  output. `docs/node_modules/` is git-ignored.
- **Build the CSS** after editing markup or `input.css`:
  ```sh
  cd docs && npm install && npm run build   # or: npm run watch
  ```
  Tailwind only scans `index.html` (`@source` in `input.css`), so new utility
  classes must be used there to be emitted.
- **Deploy**: `.github/workflows/pages.yml` rebuilds and publishes on pushes to
  `main` that touch `docs/`. One-time setup: repo Settings → Pages → Source =
  "GitHub Actions". `.nojekyll` disables Jekyll processing.
- **Design language**: dark terminal aesthetic, green (`--color-brand`) primary
  with cyan/violet accents, JetBrains Mono for code/terminal mocks, Inter for
  prose. Faux-terminal blocks reuse the `.term` component. There is a
  **demo-video placeholder** in the `#video` section — swap it for a real
  `<video>`/asciinema/YouTube embed when a recording exists.
