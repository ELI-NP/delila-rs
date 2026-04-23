# OperatorUi

Angular 20 standalone app that serves as the DELILA-rs Operator control panel.
Built by `@angular/build:application`, 3.2 GSa/s DAQ control + Run history + Tune Up + Settings.

## Deployment policy (IMPORTANT)

**`dist/` is committed to this repository.** Users who only want to run DELILA do **not** need Node.js.
Rust (`cargo build --release --bin operator`) serves the pre-built UI via `ServeDir` at `web/operator-ui/dist/operator-ui/browser/`.

**Developers who modify `src/` must rebuild and commit `dist/` together**:

```bash
cd web/operator-ui
npm install          # once, or when package.json changes
npm run build        # ng build (production config, outputHashing: "media")
git add dist/
git commit ...
```

Then redeploy with the usual rsync. No Node.js needed on the lab machine.

**Stable filenames:** entry chunks (`main.js`, `polyfills.js`, `styles.css`, `index.html`) are
hash-free. Lazy chunks keep content-based hashes (`chunk-XXXXXXXX.js`), so they only change
when their content actually changes — git diffs stay clean.

---

## Development

### Dev server (hot reload)
```bash
ng serve
```
http://localhost:4200/ — auto-reloads on file change. Proxy to Operator backend is configured
in `proxy.conf.json` (default target `http://localhost:9090`).

### Build (for commit/deploy)
```bash
npm run build
```
Output: `dist/operator-ui/browser/` (~2.6 MB). **Commit this directory.**

### Watch mode (dev build + rebuild on change)
```bash
npm run watch
```

### Code scaffolding
```bash
ng generate component component-name
```
For a complete list, run `ng generate --help`.

### Lint
```bash
npm run lint
```

### Tests
```bash
npm test        # Karma unit tests (not currently used in CI)
```

---

## Deployment pre-check (optional CI task)

To verify `dist/` is in sync with `src/`, run `ng build` and diff — any output means the
committer forgot to rebuild:
```bash
npm run build && git diff --exit-code dist/
```

---

## Links

- [Angular CLI Overview](https://angular.dev/tools/cli)
- DELILA-rs [CLAUDE.md](../../CLAUDE.md) — project overview and conventions
- Operator backend: [src/bin/operator.rs](../../src/bin/operator.rs)
