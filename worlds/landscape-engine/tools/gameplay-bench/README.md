# Gameplay benchmarks

The measurements behind `docs/gameplay.md`: scripting languages, ECS crates, Rapier and
`solstice-world` itself (`b_world`), natively and on the web. These are throwaway benchmarks,
not part of the engine's workspace, and are not built by CI.

Monty 1.5 needs Rust 1.96 or newer, and the project pins 1.95, so these use 1.98. Every figure in
the doc came from Rust 1.98, on a Ryzen 7 9800X3D under Windows 11, with Node 24.

## Native

```bash
cargo +1.98.0 build --release
for b in b_base b_rhai b_starlark b_monty b_rapier b_bevy b_hecs b_world b_anim; do ./target/release/$b; done
```

The probes check which Python features each language accepts:

```bash
cargo +1.98.0 run --release -p b_starlark --bin probe
cargo +1.98.0 run --release -p b_monty --bin probe
```

## Web

Each crate builds as a wasm library with a `run()` export, wrapped by `wasm-bindgen` 0.2.130 (it
must match the `wasm-bindgen` crate exactly; `scripts/build-web.sh` shows how to install it), and
run in Node, which uses Chrome's engine. `.cargo/config.toml` selects `getrandom`'s browser
backend.

```bash
cargo +1.98.0 build --release --target wasm32-unknown-unknown --lib
for b in b_base b_rhai b_starlark b_monty b_rapier b_bevy b_hecs b_world b_anim; do
  wasm-bindgen --target nodejs --remove-name-section --remove-producers-section \
    --out-dir js/$b target/wasm32-unknown-unknown/release/$b.wasm
  node run.js ./js/$b/$b.js
done
```

Rapier with wasm SIMD:

```bash
RUSTFLAGS='--cfg getrandom_backend="wasm_js" -C target-feature=+simd128' CARGO_TARGET_DIR=target_simd \
  cargo +1.98.0 build --release --target wasm32-unknown-unknown --lib -p b_rapier
```

Download sizes are the `_bg.wasm` files gzipped (`gzip -9 -c FILE | wc -c`).

## RustPython

`rustpython/` is its own workspace: RustPython's fork of Ruff's parser clashes with the one
Monty uses. RustPython 0.6.0 also doesn't build as published, because its Ruff fork broke its API
in a patch release (0.16.5 to 0.16.10). The committed `Cargo.lock` pins it back to 0.16.5. Build
and run it as above, from inside `rustpython/`.

## Pyodide

```bash
npm install pyodide@0.28.3
node pyodide/pyodide_bench.mjs node_modules/pyodide
```
