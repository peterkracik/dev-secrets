---
name: landing-page
description: Build, preview, and edit the dev-secrets static landing page in docs/ (HTML + Tailwind CSS v4, served by GitHub Pages). Use when working on the marketing site, the demo-video placeholder, install commands, terminal mocks, or the Tailwind build.
---

# Landing page (docs/)

The marketing site lives in `docs/` and is served by **GitHub Pages** from that
folder. It is plain `index.html` styled with **Tailwind CSS v4**, precompiled to
a committed static `docs/tailwind.css` (no CDN, no runtime).

## Files

- `docs/index.html` — the page. All markup + Tailwind utility classes.
- `docs/src/input.css` — Tailwind **source**: design tokens in `@theme`,
  component classes (`.btn`, `.card`, `.term`, `.install`, …) in `@layer`,
  plus terminal token colors (`.c-dim`, `.c-green`, …).
- `docs/tailwind.css` — **compiled output** (committed; do not hand-edit).
- `docs/script.js` — vanilla JS: clipboard copy + scroll reveal.
- `docs/favicon.svg` — logo / favicon.
- `docs/package.json` — Tailwind CLI + build scripts.
- `.github/workflows/pages.yml` — CI build + deploy to Pages.

## Workflow

1. Edit `docs/index.html` (and `docs/src/input.css` for tokens/components).
2. Rebuild the CSS — **required** after any class or input.css change:
   ```sh
   cd docs && npm install   # first time only
   npm run build            # one-off, minified
   # or: npm run watch      # rebuild on change while iterating
   ```
3. Preview locally: open `docs/index.html`, or
   `cd docs && python3 -m http.server 8000` then visit `localhost:8000`.
4. Commit both `index.html`/`input.css` **and** the regenerated `tailwind.css`.

## Gotchas

- Tailwind only scans `index.html` (`@source "../index.html"` in `input.css`).
  A new utility class only ends up in the output if it appears in `index.html`.
  If a style "doesn't apply", you probably forgot to rebuild.
- `docs/node_modules/` is git-ignored; never commit it.
- The `#video` section is a **placeholder** — replace it with a real
  `<video>`/asciinema/YouTube embed when a recording exists.
- Keep copy aligned with the product framing: **lightweight, local-first,
  single Rust binary, no cloud**. It is not a production secrets manager.
- Design language: dark terminal theme, green primary (`--color-brand`) with
  cyan/violet accents, JetBrains Mono for code, Inter for prose.
