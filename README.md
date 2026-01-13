# gitlab-tree

Ratatui-based CLI to explore GitLab groups and projects as a tree.
The right-hand details pane shows metadata like path, visibility, and last activity.
Use `/` to filter the tree by name with fuzzy matching.
Groups and projects are sorted alphabetically.

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
- `GITLAB_CACHE_TTL_SECONDS` (optional): cache TTL in seconds (default `300`).
- `GITLAB_CACHE_PATH` (optional): override cache file location.

## Run

```bash
GITLAB_TOKEN=... cargo run
```

Optional custom URL:

```bash
GITLAB_URL=https://gitlab.example.com GITLAB_TOKEN=... cargo run
```

## Controls

- `q` or `ctrl-c`: quit
- `up/down` or `k/j`: move selection
- `right/left` or `l/h`: expand/collapse
- `gg`: jump to top
- `G`: jump to bottom
- `y`: copy selected group/project URL to clipboard
- `o`: open selected group/project in your browser
- `/`: enter search mode
- `enter`: apply search
- `esc`: clear search
- `r`: refresh the tree from GitLab
- `enter`: toggle expand/collapse when not searching
- `pgup/pgdn`: page up/down in the tree

Clipboard fallback: if no GUI clipboard is available, the app will try `wl-copy` (Wayland), `xclip` (X11), or OSC52 (tmux-compatible terminals) when present.
