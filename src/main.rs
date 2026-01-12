use std::{collections::HashMap, env, io, time::Duration};

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Modifier, Style},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Terminal,
};
use serde::Deserialize;

fn main() -> Result<()> {
    let config = Config::from_env()?;
    let mut terminal = setup_terminal()?;
    let result = run_app(&mut terminal, config);
    restore_terminal(&mut terminal)?;
    result
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    Ok(Terminal::new(backend)?)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, config: Config) -> Result<()> {
    let mut app = match App::from_gitlab(config.clone()) {
        Ok(app) => app,
        Err(err) => App::sample_with_status(config, format!("load error: {err}")),
    };
    loop {
        let visible = app.visible_nodes();
        app.ensure_selection(visible.len());

        terminal.draw(|frame| ui(frame, &app, &visible))?;

        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') => return Ok(()),
                    KeyCode::Up => app.move_up(),
                    KeyCode::Down => app.move_down(visible.len()),
                    KeyCode::Left => app.collapse_or_parent(&visible),
                    KeyCode::Right => app.expand_or_child(&visible),
                    KeyCode::Char('k') => app.move_up(),
                    KeyCode::Char('j') => app.move_down(visible.len()),
                    KeyCode::Char('h') => app.collapse_or_parent(&visible),
                    KeyCode::Char('l') => app.expand_or_child(&visible),
                    KeyCode::Char('g') => {
                        if app.consume_pending_g() {
                            app.move_top();
                        } else {
                            app.set_pending_g();
                        }
                    }
                    KeyCode::Char('G') => app.move_bottom(visible.len()),
                    _ => {}
                }
                if key.code != KeyCode::Char('g') {
                    app.clear_pending_g();
                }
            }
        }
    }
}

fn ui(
    frame: &mut ratatui::Frame,
    app: &App,
    visible: &[VisibleNode],
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(2)])
        .split(frame.size());

    let items: Vec<ListItem> = visible
        .iter()
        .map(|node| {
            let data = &app.nodes[node.id];
            let marker = if data.children.is_empty() {
                " * "
            } else if data.expanded {
                "[-]"
            } else {
                "[+]"
            };
            let indent = "  ".repeat(node.depth);
            let kind = match data.kind {
                NodeKind::Group => "group",
                NodeKind::Project => "project",
            };
            let line = format!("{indent}{marker} {kind} {}", data.name);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().title("GitLab Tree").borders(Borders::ALL))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    let mut state = ListState::default();
    if !visible.is_empty() {
        state.select(Some(app.selected));
    }
    frame.render_stateful_widget(list, chunks[0], &mut state);

    let token_state = if app.config.gitlab_token.is_empty() {
        "token: unset"
    } else {
        "token: set"
    };
    let mut footer = format!(
        "q quit | up/down move | right expand | left collapse | {} | {}",
        app.config.gitlab_url, token_state
    );
    if let Some(status) = &app.status {
        footer.push_str(&format!(" | {status}"));
    }
    let help = Paragraph::new(footer);
    frame.render_widget(help, chunks[1]);
}

#[derive(Clone)]
struct Config {
    gitlab_url: String,
    gitlab_token: String,
}

impl Config {
    fn from_env() -> Result<Self> {
        Self::from_env_reader(|key| env::var(key).ok())
    }

    fn from_env_reader<F>(reader: F) -> Result<Self>
    where
        F: Fn(&str) -> Option<String>,
    {
        let gitlab_url =
            read_env_optional(&reader, "GITLAB_URL").unwrap_or_else(|| "https://gitlab.com".to_string());
        let gitlab_token = read_env_required(&reader, "GITLAB_TOKEN")?;

        Ok(Self {
            gitlab_url,
            gitlab_token,
        })
    }
}

#[derive(Debug, Deserialize)]
struct GitLabGroup {
    id: usize,
    name: String,
    #[serde(default)]
    parent_id: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct GitLabProject {
    name: String,
}

struct GroupProjects {
    group_id: usize,
    projects: Vec<GitLabProject>,
}

fn fetch_groups(config: &Config) -> Result<Vec<GitLabGroup>> {
    let client = reqwest::blocking::Client::new();
    let base = config.gitlab_url.trim_end_matches('/');
    let url = format!("{base}/api/v4/groups");
    let mut page = 1usize;
    let mut all = Vec::new();

    loop {
        let resp = client
            .get(&url)
            .header("PRIVATE-TOKEN", &config.gitlab_token)
            .query(&[
                ("per_page", "100"),
                ("page", &page.to_string()),
                ("membership", "true"),
            ])
            .send()?
            .error_for_status()?;

        let next_page = resp
            .headers()
            .get("x-next-page")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .trim()
            .to_string();

        let mut page_groups: Vec<GitLabGroup> = resp.json()?;
        all.append(&mut page_groups);

        if next_page.is_empty() {
            break;
        }

        page = next_page
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid x-next-page header: {next_page}"))?;
    }

    Ok(all)
}

fn fetch_group_projects(config: &Config, group_id: usize) -> Result<Vec<GitLabProject>> {
    let client = reqwest::blocking::Client::new();
    let base = config.gitlab_url.trim_end_matches('/');
    let url = format!("{base}/api/v4/groups/{group_id}/projects");
    let mut page = 1usize;
    let mut all = Vec::new();

    loop {
        let resp = client
            .get(&url)
            .header("PRIVATE-TOKEN", &config.gitlab_token)
            .query(&[
                ("per_page", "100"),
                ("page", &page.to_string()),
                ("simple", "true"),
                ("include_subgroups", "false"),
            ])
            .send()?
            .error_for_status()?;

        let next_page = resp
            .headers()
            .get("x-next-page")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .trim()
            .to_string();

        let mut page_projects: Vec<GitLabProject> = resp.json()?;
        all.append(&mut page_projects);

        if next_page.is_empty() {
            break;
        }

        page = next_page
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid x-next-page header: {next_page}"))?;
    }

    Ok(all)
}

fn fetch_projects_by_group(
    config: &Config,
    groups: &[GitLabGroup],
) -> Result<Vec<GroupProjects>> {
    let mut projects = Vec::with_capacity(groups.len());
    for group in groups {
        let group_projects = fetch_group_projects(config, group.id)?;
        projects.push(GroupProjects {
            group_id: group.id,
            projects: group_projects,
        });
    }
    Ok(projects)
}
fn read_env_optional<F>(reader: &F, key: &str) -> Option<String>
where
    F: Fn(&str) -> Option<String>,
{
    reader(key).filter(|value| !value.trim().is_empty())
}

fn read_env_required<F>(reader: &F, key: &str) -> Result<String>
where
    F: Fn(&str) -> Option<String>,
{
    match read_env_optional(reader, key) {
        Some(value) => Ok(value),
        None => anyhow::bail!("missing required environment variable: {key}"),
    }
}

#[derive(Clone)]
struct Node {
    name: String,
    kind: NodeKind,
    children: Vec<usize>,
    expanded: bool,
}

#[derive(Clone, Copy)]
enum NodeKind {
    Group,
    Project,
}

struct App {
    nodes: Vec<Node>,
    roots: Vec<usize>,
    parent: Vec<Option<usize>>,
    selected: usize,
    config: Config,
    status: Option<String>,
    pending_g: bool,
}

impl App {
    fn sample_with_status(config: Config, status: String) -> Self {
        let mut nodes = Vec::new();

        let dev_platform = push_node(&mut nodes, "dev-platform", NodeKind::Group);
        let data = push_node(&mut nodes, "data", NodeKind::Group);
        let sec = push_node(&mut nodes, "security", NodeKind::Group);

        let dev_backend = push_node(&mut nodes, "backend", NodeKind::Group);
        let dev_frontend = push_node(&mut nodes, "frontend", NodeKind::Group);
        let dev_platform_proj = push_node(&mut nodes, "platform-tools", NodeKind::Project);
        nodes[dev_platform].children.extend([dev_backend, dev_frontend, dev_platform_proj]);

        let api = push_node(&mut nodes, "api", NodeKind::Project);
        let auth = push_node(&mut nodes, "auth", NodeKind::Project);
        nodes[dev_backend].children.extend([api, auth]);

        let web = push_node(&mut nodes, "web", NodeKind::Project);
        let design = push_node(&mut nodes, "design-system", NodeKind::Project);
        nodes[dev_frontend].children.extend([web, design]);

        let data_ingest = push_node(&mut nodes, "ingest", NodeKind::Group);
        let data_models = push_node(&mut nodes, "models", NodeKind::Group);
        let data_tools = push_node(&mut nodes, "data-tools", NodeKind::Project);
        nodes[data].children.extend([data_ingest, data_models, data_tools]);

        let ingest = push_node(&mut nodes, "ingest", NodeKind::Project);
        let pipeline = push_node(&mut nodes, "pipeline", NodeKind::Project);
        nodes[data_ingest].children.extend([ingest, pipeline]);

        let fraud = push_node(&mut nodes, "fraud", NodeKind::Project);
        let churn = push_node(&mut nodes, "churn", NodeKind::Project);
        nodes[data_models].children.extend([fraud, churn]);

        let sec_tools = push_node(&mut nodes, "sec-tools", NodeKind::Project);
        let audits = push_node(&mut nodes, "audits", NodeKind::Project);
        nodes[sec].children.extend([sec_tools, audits]);

        nodes[dev_platform].expanded = true;
        nodes[data].expanded = true;
        nodes[sec].expanded = true;

        let parent = build_parent_map(&nodes);

        Self {
            nodes,
            roots: vec![dev_platform, data, sec],
            parent,
            selected: 0,
            config,
            status: Some(status),
            pending_g: false,
        }
    }

    fn from_gitlab(config: Config) -> Result<Self> {
        let groups = fetch_groups(&config)?;
        let projects = fetch_projects_by_group(&config, &groups)?;
        let total_projects: usize = projects.iter().map(|entry| entry.projects.len()).sum();
        let status = format!("groups: {}, projects: {}", groups.len(), total_projects);
        Ok(Self::from_gitlab_data(groups, projects, config, status))
    }

    fn from_gitlab_data(
        groups: Vec<GitLabGroup>,
        projects_by_group: Vec<GroupProjects>,
        config: Config,
        status: String,
    ) -> Self {
        let mut nodes = Vec::new();
        let mut id_to_node = HashMap::new();
        for group in &groups {
            let node_id = push_node(&mut nodes, &group.name, NodeKind::Group);
            id_to_node.insert(group.id, node_id);
        }

        let mut roots = Vec::new();
        for group in &groups {
            let child_id = match id_to_node.get(&group.id) {
                Some(id) => *id,
                None => continue,
            };
            if let Some(parent_id) = group.parent_id {
                if let Some(parent_node) = id_to_node.get(&parent_id) {
                    nodes[*parent_node].children.push(child_id);
                    continue;
                }
            }
            roots.push(child_id);
        }

        for entry in projects_by_group {
            let Some(parent_node) = id_to_node.get(&entry.group_id).copied() else {
                continue;
            };
            for project in entry.projects {
                let project_node = push_node(&mut nodes, &project.name, NodeKind::Project);
                nodes[parent_node].children.push(project_node);
            }
        }

        for &root in &roots {
            nodes[root].expanded = true;
        }

        let parent = build_parent_map(&nodes);

        Self {
            nodes,
            roots,
            parent,
            selected: 0,
            config,
            status: Some(status),
            pending_g: false,
        }
    }

    fn visible_nodes(&self) -> Vec<VisibleNode> {
        let mut out = Vec::new();
        for &root in &self.roots {
            self.walk_visible(root, 0, &mut out);
        }
        out
    }

    fn walk_visible(&self, node_id: usize, depth: usize, out: &mut Vec<VisibleNode>) {
        out.push(VisibleNode { id: node_id, depth });
        let node = &self.nodes[node_id];
        if node.expanded {
            for &child in &node.children {
                self.walk_visible(child, depth + 1, out);
            }
        }
    }

    fn ensure_selection(&mut self, visible_len: usize) {
        if visible_len == 0 {
            self.selected = 0;
        } else if self.selected >= visible_len {
            self.selected = visible_len - 1;
        }
    }

    fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    fn move_down(&mut self, visible_len: usize) {
        if self.selected + 1 < visible_len {
            self.selected += 1;
        }
    }

    fn move_top(&mut self) {
        self.selected = 0;
    }

    fn move_bottom(&mut self, visible_len: usize) {
        if visible_len > 0 {
            self.selected = visible_len - 1;
        }
    }

    fn collapse_or_parent(&mut self, visible: &[VisibleNode]) {
        if visible.is_empty() {
            return;
        }
        let node_id = visible[self.selected].id;
        if self.nodes[node_id].expanded {
            self.nodes[node_id].expanded = false;
        } else if let Some(parent) = self.parent[node_id] {
            self.select_node(parent, visible);
        }
        self.ensure_selection(self.visible_nodes().len());
    }

    fn expand_or_child(&mut self, visible: &[VisibleNode]) {
        if visible.is_empty() {
            return;
        }
        let node_id = visible[self.selected].id;
        if self.nodes[node_id].children.is_empty() {
            return;
        }
        if !self.nodes[node_id].expanded {
            self.nodes[node_id].expanded = true;
        } else {
            let child = self.nodes[node_id].children[0];
            self.select_node(child, visible);
        }
        self.ensure_selection(self.visible_nodes().len());
    }

    fn select_node(&mut self, node_id: usize, visible: &[VisibleNode]) {
        if let Some(pos) = visible.iter().position(|item| item.id == node_id) {
            self.selected = pos;
        }
    }

    fn set_pending_g(&mut self) {
        self.pending_g = true;
    }

    fn clear_pending_g(&mut self) {
        self.pending_g = false;
    }

    fn consume_pending_g(&mut self) -> bool {
        if self.pending_g {
            self.pending_g = false;
            true
        } else {
            false
        }
    }
}

struct VisibleNode {
    id: usize,
    depth: usize,
}

fn push_node(nodes: &mut Vec<Node>, name: &str, kind: NodeKind) -> usize {
    let id = nodes.len();
    nodes.push(Node {
        name: name.to_string(),
        kind,
        children: Vec::new(),
        expanded: false,
    });
    id
}

fn build_parent_map(nodes: &[Node]) -> Vec<Option<usize>> {
    let mut parent = vec![None; nodes.len()];
    for (idx, node) in nodes.iter().enumerate() {
        for &child in &node.children {
            parent[child] = Some(idx);
        }
    }
    parent
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_nodes_respects_expansion() {
        let mut nodes = Vec::new();
        let root = push_node(&mut nodes, "root", NodeKind::Group);
        let child = push_node(&mut nodes, "child", NodeKind::Project);
        nodes[root].children.push(child);

        let parent = build_parent_map(&nodes);
        let mut app = App {
            nodes,
            roots: vec![root],
            parent,
            selected: 0,
            config: Config {
                gitlab_url: "https://gitlab.com".to_string(),
                gitlab_token: "token".to_string(),
            },
            status: None,
            pending_g: false,
        };

        let visible = app.visible_nodes();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].id, root);

        app.nodes[root].expanded = true;
        let visible = app.visible_nodes();
        assert_eq!(visible.len(), 2);
        assert_eq!(visible[0].id, root);
        assert_eq!(visible[1].id, child);
    }

    #[test]
    fn config_from_env_reader_requires_token_and_defaults_url() {
        let reader = |key: &str| match key {
            "GITLAB_TOKEN" => Some("token".to_string()),
            _ => None,
        };

        let config = Config::from_env_reader(reader).expect("config should load");
        assert_eq!(config.gitlab_url, "https://gitlab.com");
        assert_eq!(config.gitlab_token, "token");
    }

    #[test]
    fn config_from_env_reader_fails_without_token() {
        let reader = |_key: &str| None;
        let result = Config::from_env_reader(reader);
        assert!(result.is_err());
    }

    #[test]
    fn from_gitlab_data_builds_parent_child_relationships() {
        let groups = vec![
            GitLabGroup {
                id: 1,
                name: "root".to_string(),
                parent_id: None,
            },
            GitLabGroup {
                id: 2,
                name: "child".to_string(),
                parent_id: Some(1),
            },
        ];
        let projects = vec![GroupProjects {
            group_id: 1,
            projects: vec![GitLabProject {
                name: "proj".to_string(),
            }],
        }];

        let app = App::from_gitlab_data(
            groups,
            projects,
            Config {
                gitlab_url: "https://gitlab.com".to_string(),
                gitlab_token: "token".to_string(),
            },
            "groups: 2".to_string(),
        );

        assert_eq!(app.roots.len(), 1);
        let root_id = app.roots[0];
        assert_eq!(app.nodes[root_id].name, "root");
        assert_eq!(app.nodes[root_id].children.len(), 2);
        let mut child_ids = app.nodes[root_id].children.clone();
        child_ids.sort_by_key(|id| app.nodes[*id].name.clone());

        let child_id = child_ids[0];
        let project_id = child_ids[1];
        assert_eq!(app.nodes[child_id].name, "child");
        assert_eq!(app.nodes[project_id].name, "proj");
        assert_eq!(app.parent[child_id], Some(root_id));
        assert_eq!(app.parent[project_id], Some(root_id));
    }

    #[test]
    fn vim_navigation_helpers_update_selection() {
        let mut nodes = Vec::new();
        let root = push_node(&mut nodes, "root", NodeKind::Group);
        let child = push_node(&mut nodes, "child", NodeKind::Project);
        nodes[root].children.push(child);
        nodes[root].expanded = true;

        let parent = build_parent_map(&nodes);
        let mut app = App {
            nodes,
            roots: vec![root],
            parent,
            selected: 1,
            config: Config {
                gitlab_url: "https://gitlab.com".to_string(),
                gitlab_token: "token".to_string(),
            },
            status: None,
            pending_g: false,
        };

        app.move_top();
        assert_eq!(app.selected, 0);

        app.move_bottom(2);
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn vim_navigation_pending_g_toggles() {
        let mut app = App {
            nodes: Vec::new(),
            roots: Vec::new(),
            parent: Vec::new(),
            selected: 0,
            config: Config {
                gitlab_url: "https://gitlab.com".to_string(),
                gitlab_token: "token".to_string(),
            },
            status: None,
            pending_g: false,
        };

        assert!(!app.consume_pending_g());
        app.set_pending_g();
        assert!(app.consume_pending_g());
        assert!(!app.consume_pending_g());
    }
}
