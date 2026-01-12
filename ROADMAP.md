# Roadmap

## Completed
- Add Vim-style navigation keybinds (e.g., `j/k`, `h/l`, `gg`, `G`).

## Near Term
- Add API filter options (env/config) for GitLab queries:
  - `include_subgroups`, `top_level_only`, `owned`, `all_available`
  - Group/project visibility filters
  - Pagination controls for large instances
- Open the selected group/project in the system web browser.
- Copy the selected group/project URL to the clipboard.
- Show a loading view while fetching GitLab data to avoid a blank screen.
- Show personal projects in the tree using the username as the top-level group.
- Show a toast notification after yanking a URL to the clipboard.
- Expand/contract the tree by pressing Enter.

## Future
- Show project metadata (path, visibility, last activity) in the UI.
- Add search and fuzzy filtering across groups/projects.
- Cache API responses to speed up startup.
