# gitlab-tree

Ratatui-based CLI to explore GitLab groups and projects as a tree.

## Requirements

- Rust toolchain (stable)
- GitLab personal access token

## Configuration

Set the following environment variables:

- `GITLAB_TOKEN` (required): GitLab personal access token.
- `GITLAB_URL` (optional): GitLab base URL. Defaults to `https://gitlab.com`.

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
