# Testing

Run everything with:

```sh
scripts/check.sh
```

It runs `cargo fmt --check`, clippy with `-D warnings`, `cargo test
--workspace`, and the headless UI smoke test. It takes about 35 s on the dev
host. Tests need **no GPU and no display**:
- Rendering uses wgpu's software fallback adapter (Mesa **lavapipe**) through
  `BACKSTAGE_WGPU_FALLBACK=1`.
- The UI runs inside a **headless `cage`** Wayland compositor.

## Tiers

| Tier | Where | What it covers |
|---|---|---|
| Unit | `#[cfg(test)]` in each crate | Protocol round-trips (proptest), garbage input never panics, frame-ring seqlock stress test, scene layout math, `StageHealth` crash/hang policy, argument and file-name parsing |
| Integration | `crates/backstage_stage/tests/`, `crates/backstage_tools/tests/supervisor.rs` | The real stage binary driven over the real protocol (handshake, frames, resize, shutdown, disconnect, version mismatch, pointer). The real `Supervisor` spawning, killing, restarting, and cleaning up stages. |
| Regression: golden images | `crates/backstage_render/tests/golden.rs`, `tests/golden/*.png` | Pixel output of the renderer on lavapipe |
| Regression: wire format | `crates/backstage_protocol/src/snapshots/` (insta) | Byte encoding of every protocol message |
| Regression: architecture | `crates/backstage_tools/tests/dependency_boundaries.rs` | Runtime crates never link GTK/Relm4. The tools crate never links wgpu, the renderer, or the VM. |
| UI smoke | `crates/backstage_tools/tests/ui_smoke.rs` (ignored by default) | The real editor in headless cage: frames arrive, the stage is killed, it restarts, and frames arrive again |
| Probe | `latency_probe` in `stage_protocol.rs` (ignored) | Pointer → published frame latency (prints numbers, asserts nothing) |

## Updating reference data
- **Golden images:** when a rendering change is intended, look at the
  failure output in `target/golden-failures/` (`*.actual.png`,
  `*.diff.png`, with magenta marking bad pixels), then re-bless:
  ```sh
  BACKSTAGE_BLESS=1 cargo test -p backstage_render --test golden
  ```
  At most 16 pixels may differ by more than 2 per channel. The budget is
  small on purpose: a 1 px line moving changes only about 100 pixels.
  Goldens are tied to lavapipe, so after a Mesa upgrade, review the diffs
  and re-bless.
- **Wire-format snapshot:** a diff here means old and new processes can no
  longer talk. Bump `PROTOCOL_VERSION`, then accept the new snapshot:
  ```sh
  INSTA_UPDATE=always cargo test -p backstage_protocol wire_format
  ```
  `cargo install cargo-insta` adds an interactive `cargo insta review`.

## Useful single runs
```sh
cargo test -p backstage_protocol                     # fast, pure
cargo test -p backstage_stage                        # real stage process
cargo test -p backstage_tools --test ui_smoke -- --ignored
cargo test -p backstage_stage --test stage_protocol -- --ignored --nocapture latency
```
The latency probe uses the real GPU unless `BACKSTAGE_WGPU_FALLBACK=1` is
set.

## Requirements
- Mesa lavapipe (`lvp_icd`). It ships with the system Mesa on Bazzite.
- `cage` for the UI smoke test (system package on Bazzite). The test skips
  itself if `cage` is missing.
- GTK 4 dev files, as described in [dev-setup.md](dev-setup.md).

## CI
`.github/workflows/ci.yml` runs `scripts/check.sh` on every push to `main`,
on pull requests, and on demand.
- **Environment:** a `fedora:44` container, to match the dev host's GTK,
  cage, and Mesa lavapipe.
- **On failure:** golden-image diffs are uploaded as the `golden-failures`
  artifact.
- **If lavapipe output differs between CI and local:** re-bless from CI's
  `*.actual.png` files, after reviewing them.
