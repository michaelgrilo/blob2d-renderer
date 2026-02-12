# Beyond vs Below

Rust WASM turn-based 1v1 PvP dueling game.

## Docs

- Docs Index: `docs/index.md`
- Development Runbook: `docs/development.md`
- Overview: `docs/overview.md`
- Architecture: `docs/architecture.md`
- Methodology (DTI): `docs/methodology.md`
- TODO: `docs/todo.md`
- Notes: `docs/notes.md`
- Documentation Standards: `documentation_standards.md`

## Dev

Use Trunk to build and serve the app:

```bash
trunk serve
```

`index.html` includes a `data-trunk` Rust link so Trunk builds the crate to WebAssembly.

For stable local visual testing and flicker troubleshooting, use the runbook:

- `docs/development.md`

## Offline Support (Service Worker)

Offline support is wired through your `workbox-rs` port in a dedicated Service Worker crate:

- SW crate: `sw/`
- SW bootstrap scripts: `sw_bootstrap_sync.js` (current) and `sw_bootstrap.js` (legacy compatibility)
- SW output assets: `assets/sw/`

Runtime behavior:

- Registers `/sw_bootstrap_sync.js` as a module service worker.
- Removes stale SW registrations that are not using `sw_bootstrap_sync.js`.
- Precaches app shell + key assets during SW install.
- Uses `workbox_rs::CacheFirst` for same-origin GET runtime caching in non-localhost environments.
- On `localhost` / `127.0.0.1`, runtime caching is disabled to avoid dev reload loops; precache remains enabled for offline tests.
- In local dev (`localhost` / `127.0.0.1`), SW registration is enabled by default.
- Use `?nosw=1` to run without a service worker.

Local troubleshooting + deterministic offline test steps are documented in:

- `docs/development.md` (`Offline Incident Notes (2026-02-11)` and `Deterministic Offline Test Flow`)

Build/update SW assets:

```bash
./scripts/build_sw.sh
```

## Build (wasm-pack)

To produce a `pkg/` directory suitable for direct web usage:

```bash
wasm-pack build --target web
```
