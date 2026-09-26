## What (one line)

Fix #N: ...

## Verification (paste output)

- `cargo fmt --check`:
- `cargo test --all-targets`:
- `cargo clippy --all-targets -- -D warnings`:

## Notes

- Issues close only when the fix is on `main` and CI is green.
- New keys parser kinds need: committed test, README keymap row (keys),
  registry + cap wiring (parsers), 400-line module budget (`src/gui/`).
