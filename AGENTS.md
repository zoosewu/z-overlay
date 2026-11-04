## Agent skills

### Issue tracker

Issues are tracked as local Markdown under `.scratch/`. See `docs/agents/issue-tracker.md`.

### Triage labels

Use the default five-role vocabulary. See `docs/agents/triage-labels.md`.

### Domain docs

This is a single-context repository. See `docs/agents/domain.md`.

## Commit convention

Use [Conventional Commits](https://www.conventionalcommits.org/): `type(scope): description`. Scope is optional.

Common types: `feat`, `fix`, `perf`, `refactor`, `chore`, `test`, `build`, `ci`, `docs`.

## Releases

CI (`.github/workflows/ci.yml`) runs `cargo fmt --check`, clippy, the tests and a release build on every push and pull request.

To release, set `version` in `Cargo.toml`, run `cargo check` so `Cargo.lock` follows, commit, then push a matching tag:

```powershell
git tag v0.2.0
git push origin v0.2.0
```

`.github/workflows/release.yml` fails if the tag does not match the crate version; otherwise it runs the same checks, creates the GitHub Release with generated notes, and attaches `z-overlay.exe` with its `.sha256`.
