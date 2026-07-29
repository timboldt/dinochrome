# dinochrome

A top-down 2D tank game in Rust and [Bevy](https://bevy.org). You command a lone
hover tank in a large procedurally generated maze, hunting down the drone
factories hidden in it.

**Status: M0 (skeleton).** The workspace builds, the window opens, a rectangle
drives around with WASD, the menu/playing/paused states work, and the same build
runs in a browser and deploys to GitHub Pages. There is no maze, no combat and
no enemy yet — see `plan.md` for the milestone list.

## Attribution

This game is inspired by Jim Lane's 1982 Apple II game *Bolo* (Synergistic
Software) and by the mood of Keith Laumer's *Bolo: Annals of the Dinochrome
Brigade*.

It is **not affiliated with or endorsed by** the Laumer estate, Baen Books, or
Synergistic Software. All code and assets in this repository are original; no
text, art, or audio has been copied from the 1982 game or from the books. This
is an homage rather than a port, and the design deliberately departs from the
original where the 1982 version was working around hardware limits.

## Controls

| Key | Action |
| --- | --- |
| `W` `A` `S` `D` | Drive the hull |
| `Enter` / `Space` | Start (from the menu) |
| `Esc` | Pause / resume |
| `Q` | Abandon the run (while paused) |

## Building

Rust stable, latest.

```sh
# Native — the fast dev loop.
cargo run

# Tests: engine-free logic plus a headless integration smoke test.
cargo test --workspace

# Lints, as enforced in CI.
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
```

### Web

The browser build is the primary deployment target, so it is built on every push
and never allowed to fall behind.

```sh
rustup target add wasm32-unknown-unknown
cargo install --locked trunk

trunk serve            # http://127.0.0.1:8080
trunk build --release  # bundle in dist/
```

`trunk` downloads its own `wasm-bindgen` and `wasm-opt`; nothing else is needed.
The renderer targets **WebGL2**, not WebGPU, so the game runs in every current
browser rather than only the ones with WebGPU enabled.

Pushes to `main` build the bundle and publish it to GitHub Pages via
`.github/workflows/ci.yml`. Because Pages serves the site from a `/<repo>/`
subpath, CI passes `--public-url` so every emitted URL is correctly prefixed;
`trunk serve` locally uses the root and needs no such flag.

To enable deployment on a fresh clone: **Settings → Pages → Build and
deployment → Source: GitHub Actions**.

## Layout

```
crates/
  dinochrome-core/   engine-free simulation: grid math, maze gen, collision, LOS, A*
  dinochrome-game/   Bevy app: components, systems, rendering, input
assets/
index.html           canvas host and loading screen for the web build
```

`dinochrome-core` has **no Bevy dependency**, on purpose. Every rule that decides
what happens in the game lives there so it can be unit-tested headlessly, without
an `App` or a render context. It shares `glam` with Bevy, so `Vec2` crosses the
boundary without conversion.

The simulation runs on a fixed 60 Hz timestep and no simulation system reads a
variable frame delta, so gameplay is identical at any render frame rate.

## Licence

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your
option.
