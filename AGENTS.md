# Repository Guidelines

## Developer Workflow

1. Git pull on `main`.
2. Run all tests to verify correctness.
3. Identify the next change to make. Work in small changes.
4. Create a new feature branch.
5. Write a new test. Verify the test fails.
6. Implement the feature to make the test pass.
7. Update all related documentation.
8. Commit and push. Do not merge to main.

## Project Structure & Module Organization
- `src/main.rs`: single-binary Rust CLI entry point and core logic.
- `Cargo.toml`: dependencies and package metadata.
- `Cargo.lock`: pinned dependency versions.
- `README.md`: usage and configuration summary.
- `target/`: build artifacts (ignored).

## Build, Test, and Development Commands
- `cargo build`: compile the CLI without running it.
- `cargo run`: build and launch the TUI.
- `cargo test`: run unit tests (currently in `src/main.rs`).
- Example with config: `GITLAB_TOKEN=... GITLAB_URL=https://gitlab.example.com cargo run`.

## Coding Style & Naming Conventions
- Use standard Rust formatting (`cargo fmt`) with 4-space indentation.
- Prefer descriptive, domain-specific names (e.g., `Config`, `VisibleNode`).
- Keep UI/layout logic in `ui(...)` and core tree logic in the `App` impl.
- Avoid printing secrets; never log `GITLAB_TOKEN`.

## Testing Guidelines
- Framework: Rust built-in test harness (`cargo test`).
- Unit tests live in `#[cfg(test)]` modules within `src/main.rs`.
- Test names use behavior-based phrasing, e.g., `config_from_env_reader_requires_token_and_defaults_url`.
- Aim to cover config parsing and tree visibility/selection behavior.
- Practice test-driven development: write tests first, then implement the code.
- Only commit after tests pass; do not proceed with commits until `cargo test` is green.

## Commit & Pull Request Guidelines
- Commit messages use concise, imperative phrases (e.g., “Add README with configuration and usage”).
- Keep commits focused: one logical change set per commit.
- PRs should describe the user-facing behavior change and include:
  - What changed and why
  - How to test (commands and env vars)
  - Screenshots only if UI behavior changed significantly

## Security & Configuration Tips
- Required config: `GITLAB_TOKEN` (personal access token).
- Optional config: `GITLAB_URL` (defaults to `https://gitlab.com`).
- Do not commit `.envrc` or token values; `.envrc` is gitignored.
