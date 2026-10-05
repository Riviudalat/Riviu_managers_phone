# Repository Guidelines

## Project Structure & Module Organization

Read [README.md](README.md), [the developer guide](docs/developer-guide.md), and [agent contracts](docs/agents/README.md) first.

- `apps/desktop/src/`: React/TypeScript UI; `api.ts` owns frontend IPC.
- `apps/desktop/src-tauri/`: Rust/Tauri commands and lifecycle.
- `crates/`: core, SQLite, device ownership, Android/iOS drivers, scripting, signing, and deployment checks.
- `sidecars/`: runtimes, device agents, and tools. `scripts/` contains verification and packaging tooling.
- UI unit tests sit beside source; browser scenarios live in `apps/desktop/e2e/`. Rust tests live in crate modules or `tests/`. Product documentation lives in `docs/`.

## Build, Test, and Development Commands

Use pinned toolchains and lockfiles; do not mix package managers. From the repository root:

```powershell
npm ci --prefix apps/desktop
cargo fmt --all -- --check
cargo test --locked -p riviu-core -- --test-threads=1
cargo clippy --locked -p riviu-core --all-targets -- -D warnings
```

These install dependencies, check formatting, test core, and run Clippy. From `apps/desktop`:

```powershell
npm run dev                 # Frontend development server
npm run tauri:dev           # Native app; may operate real devices/schedules
npm run lint               # Oxlint
npm test                   # Vitest
npx tsc -b --pretty false   # TypeScript checks
npm run build              # Frontend build with provenance
npm run test:e2e            # Playwright browser scenarios
```

Follow [the release runbook](docs/development/build-release.md) for installers; frontend builds do not qualify packaged runtimes.

## Coding Style & Naming Conventions

Match neighboring code: two-space TypeScript indentation, double-quoted strings, PascalCase components/types, and camelCase functions. Rust uses rustfmt, four-space indentation, snake_case functions/modules, and PascalCase types. Keep IPC typed and device ownership in the existing control plane.

## Testing Guidelines

Use `test-audit` before writing, changing, or reviewing tests. Prefer one owner test per observable contract; extend existing cases. Regression tests must fail before the fix for the intended reason. Avoid test-only production seams and unnecessary tests for documentation or renames. Name frontend tests `*.test.ts`/`*.test.tsx`; use Rust `#[test]`/`#[tokio::test]`. Run focused checks first. Distinguish browser mocks, native smoke, real-device, and installer evidence.

## Commit & Pull Request Guidelines

Follow history: `fix(publish): preserve retry identity`. Keep commits scoped. Describe behavior changes, verification commands/results, existing failures, and unverified scope; link relevant issues and attach screenshots for UI changes. CI requires manual dispatch. Update product documentation rather than adding changelog entries here.

## Device & Agent Constraints

Keep credentials and operational logs outside Git. Preserve ownership, cancellation, and uncertain outcomes; never replay unconfirmed Post/Send. Read [WDA safety](docs/agents/02-wda-doc-truoc-khi-sua.md) before iOS changes. For AI changes, use `typesafe-ai` and its current documentation. Respect coordinator-assigned file and execution scope.
