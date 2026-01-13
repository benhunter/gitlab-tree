# Roadmap

## Completed
- Add API filter options (env/config) for GitLab queries.
- Open the selected group/project in the system web browser.
- Copy the selected group/project URL to the clipboard.
- Add Vim-style navigation keybinds (e.g., `j/k`, `h/l`, `gg`, `G`).
- Show a loading view while fetching GitLab data to avoid a blank screen.
- Show personal projects in the tree using the username as the top-level group.
- Show a toast notification after yanking a URL to the clipboard.
- Show project metadata (path, visibility, last activity) in the UI.
- Add search and fuzzy filtering across groups/projects.
- Cache API responses to speed up startup.
- Add a refresh keybind (`r`) to reload the tree from GitLab.
- Enforce feature-branch-only commits in contributor workflow.
- Add clipboard fallback for headless sessions (e.g., `wl-copy`, `xclip`).
- Expand/contract the tree by pressing Enter.
- Alphabetize the groups and projects.
- Ctrl-C closes the app.
- PgUp/PgDn navigates by page.
- Configurable sort options for the group order and project order, such as alph or date of recent activity.

## Near Term
- the group with the same name as the user should not include projects that are in another group. The user node should only have personal projects that are not under a group.

## Future
- TBD
