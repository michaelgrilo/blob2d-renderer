# Beyond vs Below

Rust WASM turn-based 1v1 PvP dueling game.

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
