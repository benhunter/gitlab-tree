# gitlab-tree

Ratatui-based CLI to explore GitLab groups and projects as a tree.
The right-hand details pane shows metadata like path, visibility, and last activity.

## Requirements

- Rust toolchain (stable)
- GitLab personal access token

## Configuration

Set the following environment variables:

- `GITLAB_TOKEN` (required): GitLab personal access token.
- `GITLAB_URL` (optional): GitLab base URL. Defaults to `https://gitlab.com`.
- `GITLAB_ALL_AVAILABLE` (optional): include all accessible groups (`true`/`false`).
- `GITLAB_OWNED` (optional): only return owned groups (`true`/`false`).
- `GITLAB_TOP_LEVEL_ONLY` (optional): only top-level groups (`true`/`false`).
- `GITLAB_INCLUDE_SUBGROUPS` (optional): include subgroup projects (`true`/`false`).
- `GITLAB_VISIBILITY` (optional): filter by visibility (`private`, `internal`, `public`).
- `GITLAB_PER_PAGE` (optional): page size for API calls (default `100`).

## Run

```bash
GITLAB_TOKEN=... cargo run
```

Optional custom URL:

```bash
GITLAB_URL=https://gitlab.example.com GITLAB_TOKEN=... cargo run
```

## Controls

- `q`: quit
- `up/down` or `k/j`: move selection
- `right/left` or `l/h`: expand/collapse
- `gg`: jump to top
- `G`: jump to bottom
- `y`: copy selected group/project URL to clipboard
- `o`: open selected group/project in your browser
