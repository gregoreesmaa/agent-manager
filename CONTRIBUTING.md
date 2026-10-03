# Contributing to staap

Thank you for volunteering! staap is free software (MIT) —
every contribution stays free: no paywalls, no tiers, no telemetry.

## Volunteer rotations

Small, copy-paste-sized roles; pick one for a release cycle:

- **Triage gardener** — label new issues (`ux` / `product` / `technical`),
  close duplicates, keep the Top-10 lists honest.
- **Docs shepherd** — keep README keymap + this file matching the code;
  every new key binding must land in the README table first.
- **Parser gardener** — extend `src/parsers/` (see minimal example below);
  new link kinds keep first-seen order, dedupe, and the 50-link cap.
- **Release shepherd** — run the checklist below, write CHANGELOG entries,
  push/merge to `main`.

## Workflow

1. Work on `main` (or a short-lived `fix/<issue>-<slug>` branch merged
   back promptly — stale branches rot against the gpui pins).
2. Every behavior change ships with a committed test: the repo runs
   `cargo test` (headless PTY tests, no window needed), plus
   `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings`.
   Do not skip failing tests — fix the code.
3. Keep `gui/shell.rs` split: no file in `src/gui/` over ~400 lines.
   Put pure logic in framework-free modules with headless tests.
4. Run the full gate before pushing:
   `cargo fmt && cargo test --all-targets && cargo clippy --all-targets -- -D warnings`.
5. Open a PR with the template (`.github/PULL_REQUEST_TEMPLATE.md`);
   link the issue (`Fix #N`). Close issues only when the fix is on `main`
   and CI is green.

## Minimal parser example

New link kinds plug into the registry with the same ordering contract
(first-seen, deduplicated). Copy-paste starter (`src/parsers/mine.rs`):

```rust
use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;

static MINE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"ticket/[A-Z]+-\d+").expect("static mine regex"));

/// Extract `ticket/ABC-123` refs in first-seen order, deduplicated.
pub fn extract_mine(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for m in MINE_RE.find_iter(text) {
        if seen.insert(m.as_str()) {
            out.push(m.as_str().to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_and_dedupes() {
        assert_eq!(
            extract_mine("see ticket/ABC-123, again ticket/ABC-123"),
            vec!["ticket/ABC-123"]
        );
    }
}
```

Then chain it into `parsers::related_links` and `RegistryParser::parse`
so provider discovery and the live shell pick it up with the same
50-link cap and `N more` display folding.

## Minimal provider example

Session sources implement one trait (`src/providers/traits.rs`):

```rust
use crate::app::ChatSession;
use crate::providers::traits::Provider;

pub struct MyProvider;

impl Provider for MyProvider {
    fn discover_sessions(&self) -> Vec<ChatSession> {
        // Unreachable store => vec![] (degraded empty list): discovery
        // never fails for a missing directory so the app still starts.
        Vec::new()
    }
}
```

Status must come from the single `app::classify` — never a private
heuristic — so live and historic runs can never disagree.
