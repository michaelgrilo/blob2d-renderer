# Beyond vs Below

Rust WASM turn-based 1v1 PvP dueling game.

## Docs

- Docs Index: `docs/index.md`
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

## Build (wasm-pack)

To produce a `pkg/` directory suitable for direct web usage:

```bash
wasm-pack build --target web
```
