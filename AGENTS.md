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

## 作品集（apex-code）

本專案列在 zoosewu 的作品集：https://zoosewu.github.io/apex-code/
- 截圖：依 https://github.com/zoosewu/apex-code/blob/main/docs/SCREENSHOTS.md 放在 `docs/screenshots/`；
  介面有明顯改變時一併更新截圖與 captions.json
- 要顯示「作者的其他專案」時：依 https://github.com/zoosewu/apex-code/blob/main/docs/API.md 讀取 API，不要把清單寫死在本專案
- 截圖由 `cargo run --example screenshots` 以示範資料離屏產生，改了介面就重跑
