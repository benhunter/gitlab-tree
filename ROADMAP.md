# Roadmap

## Near Term
- Add API filter options (env/config) for GitLab queries:
  - `include_subgroups`, `top_level_only`, `owned`, `all_available`
  - Group/project visibility filters
  - Pagination controls for large instances
- Open the selected group/project in the system web browser.
- Copy the selected group/project URL to the clipboard.
- Add Vim-style navigation keybinds (e.g., `j/k`, `h/l`, `gg`, `G`).
- Show a loading view while fetching GitLab data to avoid a blank screen.

## Future
- Show project metadata (path, visibility, last activity) in the UI.
- Add search and fuzzy filtering across groups/projects.
- Cache API responses to speed up startup.
