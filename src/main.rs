use std::{
    collections::HashMap,
    env,
    io,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use std::sync::mpsc;
use std::thread;

use anyhow::Result;
use arboard::Clipboard as SystemClipboardHandle;
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Terminal,
};
use serde::{Deserialize, Serialize};

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
    let mut loader = Some(start_loader(config.clone()));
    let mut app = None;
    let mut clipboard = build_clipboard();
    let mut browser = SystemBrowser;
    loop {
        if let Some(handle) = loader.as_mut() {
            match handle.receiver.try_recv() {
                Ok(result) => {
                    app = Some(match result {
                        Ok(app) => app,
                        Err(err) => App::sample_with_status(config.clone(), format!("load error: {err}")),
                    });
                    loader = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    app = Some(App::sample_with_status(
                        config.clone(),
                        "load error: channel closed".to_string(),
                    ));
                    loader = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }

        let mut pending_action = None;
        if let Some(app_ref) = app.as_mut() {
            let visible = app_ref.visible_nodes();
            app_ref.ensure_selection(visible.len());
            app_ref.tick_toast();

            terminal.draw(|frame| ui(frame, app_ref, &visible))?;

            if event::poll(Duration::from_millis(200))? {
                if let Event::Key(key) = event::read()? {
                    let action = if let Some(mut cb) = clipboard.take() {
                        let action =
                            app_ref.handle_key(key.code, &visible, Some(&mut *cb), &mut browser)?;
                        clipboard = Some(cb);
                        action
                    } else {
                        app_ref.handle_key(key.code, &visible, None, &mut browser)?
                    };
                    pending_action = Some(action);
                }
            }
        } else if let Some(handle) = loader.as_mut() {
            terminal.draw(|frame| ui_loading(frame, handle.tick))?;
            handle.tick = handle.tick.wrapping_add(1);

            if event::poll(Duration::from_millis(200))? {
                if let Event::Key(key) = event::read()? {
                    if key.code == KeyCode::Char('q') {
                        return Ok(());
                    }
                }
            }
        } else {
            return Ok(());
        }

        if let Some(action) = pending_action {
            match action {
                KeyAction::Quit => return Ok(()),
                KeyAction::Reload => {
                    loader = Some(start_loader(config.clone()));
                    app = None;
                }
                KeyAction::None => {}
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

    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(chunks[0]);

    let list = List::new(items)
        .block(Block::default().title("GitLab Tree").borders(Borders::ALL))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    let mut state = ListState::default();
    if !visible.is_empty() {
        state.select(Some(app.selected));
    }
    frame.render_stateful_widget(list, main_chunks[0], &mut state);

    let details_lines = if visible.is_empty() {
        vec!["No selection".to_string()]
    } else {
        let node_id = visible[app.selected].id;
        format_node_details(&app.nodes[node_id])
    };
    let details = Paragraph::new(details_lines.join("\n"))
        .block(Block::default().title("Details").borders(Borders::ALL));
    frame.render_widget(details, main_chunks[1]);

    let token_state = if app.config.gitlab_token.is_empty() {
        "token: unset"
    } else {
        "token: set"
    };
    let mut footer = format!(
        "q quit | r refresh | up/down move | right expand | left collapse | y yank | o open | / search | {} | {}",
        app.config.gitlab_url, token_state
    );
    if let Some(status) = &app.status {
        footer.push_str(&format!(" | {status}"));
    }
    if let Some(query) = &app.search_query {
        let label = if app.search_mode { "search*" } else { "search" };
        footer.push_str(&format!(" | {label}: {query}"));
    }
    let help = Paragraph::new(footer);
    frame.render_widget(help, chunks[1]);

    if let Some(toast) = &app.toast {
        render_toast(frame, toast);
    }
}

fn ui_loading(frame: &mut ratatui::Frame, tick: usize) {
    let block = Block::default().title("GitLab Tree").borders(Borders::ALL);
    let message = loading_message(tick);
    let paragraph = Paragraph::new(message).block(block);
    frame.render_widget(paragraph, frame.size());
}

fn format_node_details(node: &Node) -> Vec<String> {
    let kind = match node.kind {
        NodeKind::Group => "Group",
        NodeKind::Project => "Project",
    };
    let mut lines = vec![
        format!("Name: {}", node.name),
        format!("Kind: {kind}"),
        format!("Path: {}", node.path),
        format!("Visibility: {}", node.visibility),
        format!("URL: {}", node.url),
    ];
    if let Some(last_activity) = &node.last_activity {
        lines.push(format!("Last activity: {last_activity}"));
    }
    lines
}

fn render_toast(frame: &mut ratatui::Frame, toast: &Toast) {
    let area = frame.size();
    let width = (toast.message.len() as u16).saturating_add(4);
    let height = 3;
    let x = area.width.saturating_sub(width + 1);
    let y = 1;
    let rect = Rect::new(x, y, width.min(area.width), height.min(area.height));
    let block = Block::default().title("Notice").borders(Borders::ALL);
    let paragraph = Paragraph::new(toast.message.clone()).block(block);
    frame.render_widget(paragraph, rect);
}

struct LoadHandle {
    receiver: mpsc::Receiver<Result<App>>,
    tick: usize,
}

fn start_loader(config: Config) -> LoadHandle {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let result = App::from_gitlab(config);
        let _ = sender.send(result);
    });
    LoadHandle { receiver, tick: 0 }
}

fn loading_message(tick: usize) -> String {
    let frames = ["|", "/", "-", "\\"];
    let frame = frames[tick % frames.len()];
    format!("{frame} loading GitLab data...")
}

#[derive(Clone)]
struct Config {
    gitlab_url: String,
    gitlab_token: String,
    filters: ApiFilters,
    cache_path: PathBuf,
    cache_ttl: Duration,
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
        let filters = ApiFilters::from_env_reader(&reader)?;
        let cache_ttl_seconds =
            read_env_u64_optional(&reader, "GITLAB_CACHE_TTL_SECONDS")?.unwrap_or(300);
        let cache_path = read_env_optional(&reader, "GITLAB_CACHE_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(default_cache_path);

        Ok(Self {
            gitlab_url,
            gitlab_token,
            filters,
            cache_path,
            cache_ttl: Duration::from_secs(cache_ttl_seconds),
        })
    }
}

#[derive(Clone, Debug, Default)]
struct ApiFilters {
    all_available: Option<bool>,
    owned: Option<bool>,
    top_level_only: Option<bool>,
    include_subgroups: Option<bool>,
    visibility: Option<String>,
    per_page: u16,
}

impl ApiFilters {
    fn from_env_reader<F>(reader: &F) -> Result<Self>
    where
        F: Fn(&str) -> Option<String>,
    {
        let per_page = read_env_u16_optional(reader, "GITLAB_PER_PAGE")?.unwrap_or(100);
        Ok(Self {
            all_available: read_env_bool_optional(reader, "GITLAB_ALL_AVAILABLE")?,
            owned: read_env_bool_optional(reader, "GITLAB_OWNED")?,
            top_level_only: read_env_bool_optional(reader, "GITLAB_TOP_LEVEL_ONLY")?,
            include_subgroups: read_env_bool_optional(reader, "GITLAB_INCLUDE_SUBGROUPS")?,
            visibility: read_env_optional(reader, "GITLAB_VISIBILITY"),
            per_page,
        })
    }
}

trait ClipboardSink {
    fn set_text(&mut self, text: String) -> Result<()>;
}

#[derive(Debug, PartialEq, Eq)]
enum ClipboardBackend {
    Arboard,
    WlCopy,
    Xclip,
    None,
}

trait ClipboardProbe {
    fn arboard_ok(&self) -> bool;
    fn has_wayland(&self) -> bool;
    fn has_display(&self) -> bool;
    fn command_exists(&self, command: &str) -> bool;
}

trait BrowserOpener {
    fn open(&mut self, url: &str) -> Result<()>;
}

struct CommandClipboard {
    command: String,
    args: Vec<String>,
}

impl CommandClipboard {
    fn new(command: &str, args: &[&str]) -> Self {
        Self {
            command: command.to_string(),
            args: args.iter().map(|arg| arg.to_string()).collect(),
        }
    }
}

impl ClipboardSink for CommandClipboard {
    fn set_text(&mut self, text: String) -> Result<()> {
        let mut child = Command::new(&self.command)
            .args(&self.args)
            .stdin(Stdio::piped())
            .spawn()
            .map_err(|err| anyhow::anyhow!("clipboard command failed: {err}"))?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(text.as_bytes())?;
        }
        let status = child.wait()?;
        if !status.success() {
            anyhow::bail!("clipboard command exited with {status}");
        }
        Ok(())
    }
}

struct SystemClipboard {
    inner: SystemClipboardHandle,
}

impl SystemClipboard {
    fn new() -> Result<Self> {
        let inner = SystemClipboardHandle::new()
            .map_err(|err| anyhow::anyhow!("clipboard init failed: {err}"))?;
        Ok(Self { inner })
    }
}

impl ClipboardSink for SystemClipboard {
    fn set_text(&mut self, text: String) -> Result<()> {
        self.inner
            .set_text(text)
            .map_err(|err| anyhow::anyhow!("clipboard write failed: {err}"))?;
        Ok(())
    }
}

struct SystemClipboardProbe;

impl ClipboardProbe for SystemClipboardProbe {
    fn arboard_ok(&self) -> bool {
        SystemClipboardHandle::new().is_ok()
    }

    fn has_wayland(&self) -> bool {
        env::var_os("WAYLAND_DISPLAY").is_some()
    }

    fn has_display(&self) -> bool {
        env::var_os("DISPLAY").is_some()
    }

    fn command_exists(&self, command: &str) -> bool {
        Command::new("sh")
            .arg("-c")
            .arg(format!("command -v {command} >/dev/null 2>&1"))
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }
}

fn select_clipboard_backend(probe: &dyn ClipboardProbe) -> ClipboardBackend {
    if probe.arboard_ok() {
        ClipboardBackend::Arboard
    } else if probe.has_wayland() && probe.command_exists("wl-copy") {
        ClipboardBackend::WlCopy
    } else if probe.has_display() && probe.command_exists("xclip") {
        ClipboardBackend::Xclip
    } else {
        ClipboardBackend::None
    }
}

fn build_clipboard() -> Option<Box<dyn ClipboardSink>> {
    let probe = SystemClipboardProbe;
    match select_clipboard_backend(&probe) {
        ClipboardBackend::Arboard => SystemClipboard::new()
            .ok()
            .map(|clipboard| Box::new(clipboard) as Box<dyn ClipboardSink>),
        ClipboardBackend::WlCopy => Some(Box::new(CommandClipboard::new("wl-copy", &[]))),
        ClipboardBackend::Xclip => Some(Box::new(CommandClipboard::new(
            "xclip",
            &["-selection", "clipboard"],
        ))),
        ClipboardBackend::None => None,
    }
}

struct SystemBrowser;

impl BrowserOpener for SystemBrowser {
    fn open(&mut self, url: &str) -> Result<()> {
        open::that(url).map_err(|err| anyhow::anyhow!("open failed: {err}"))?;
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct GitLabGroup {
    id: usize,
    name: String,
    web_url: String,
    full_path: String,
    visibility: String,
    #[serde(default)]
    parent_id: Option<usize>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct GitLabProject {
    name: String,
    web_url: String,
    path_with_namespace: String,
    visibility: String,
    last_activity_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct GroupProjects {
    group_id: usize,
    projects: Vec<GitLabProject>,
}

#[derive(Debug, Deserialize)]
struct GitLabUser {
    username: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct PersonalProjects {
    username: String,
    web_url: String,
    projects: Vec<GitLabProject>,
}

#[derive(Debug, Deserialize, Serialize)]
struct CacheData {
    created_at: u64,
    groups: Vec<GitLabGroup>,
    projects_by_group: Vec<GroupProjects>,
    personal: Option<PersonalProjects>,
}

struct CacheStore {
    path: PathBuf,
    ttl: Duration,
}

impl CacheStore {
    fn new(path: PathBuf, ttl: Duration) -> Self {
        Self { path, ttl }
    }

    fn load(&self) -> Result<Option<CacheData>> {
        if !self.path.exists() {
            return Ok(None);
        }
        let data = match std::fs::read(&self.path) {
            Ok(data) => data,
            Err(_) => return Ok(None),
        };
        let cache: CacheData = match serde_json::from_slice(&data) {
            Ok(cache) => cache,
            Err(_) => return Ok(None),
        };
        if cache_is_valid(cache.created_at, self.ttl, SystemTime::now()) {
            Ok(Some(cache))
        } else {
            Ok(None)
        }
    }

    fn store(&self, cache: &CacheData) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_vec_pretty(cache)?;
        std::fs::write(&self.path, data)?;
        Ok(())
    }
}

fn cache_is_valid(created_at: u64, ttl: Duration, now: SystemTime) -> bool {
    let Ok(now) = now.duration_since(UNIX_EPOCH) else {
        return false;
    };
    let now = now.as_secs();
    let ttl = ttl.as_secs();
    now.saturating_sub(created_at) <= ttl
}

fn fetch_groups(config: &Config) -> Result<Vec<GitLabGroup>> {
    let client = reqwest::blocking::Client::new();
    let base = config.gitlab_url.trim_end_matches('/');
    let url = format!("{base}/api/v4/groups");
    let mut page = 1usize;
    let mut all = Vec::new();

    loop {
        let mut query: Vec<(&str, String)> = vec![
            ("per_page", config.filters.per_page.to_string()),
            ("page", page.to_string()),
            ("membership", "true".to_string()),
        ];
        if let Some(value) = config.filters.all_available {
            query.push(("all_available", value.to_string()));
        }
        if let Some(value) = config.filters.owned {
            query.push(("owned", value.to_string()));
        }
        if let Some(value) = config.filters.top_level_only {
            query.push(("top_level_only", value.to_string()));
        }
        if let Some(value) = &config.filters.visibility {
            query.push(("visibility", value.to_string()));
        }

        let resp = client
            .get(&url)
            .header("PRIVATE-TOKEN", &config.gitlab_token)
            .query(&query)
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
        let mut query: Vec<(&str, String)> = vec![
            ("per_page", config.filters.per_page.to_string()),
            ("page", page.to_string()),
            ("simple", "true".to_string()),
        ];
        if let Some(value) = config.filters.include_subgroups {
            query.push(("include_subgroups", value.to_string()));
        }
        if let Some(value) = &config.filters.visibility {
            query.push(("visibility", value.to_string()));
        }

        let resp = client
            .get(&url)
            .header("PRIVATE-TOKEN", &config.gitlab_token)
            .query(&query)
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

fn fetch_current_user(config: &Config) -> Result<GitLabUser> {
    let client = reqwest::blocking::Client::new();
    let base = config.gitlab_url.trim_end_matches('/');
    let url = format!("{base}/api/v4/user");
    let user = client
        .get(&url)
        .header("PRIVATE-TOKEN", &config.gitlab_token)
        .send()?
        .error_for_status()?
        .json::<GitLabUser>()?;
    Ok(user)
}

fn fetch_owned_projects(config: &Config) -> Result<Vec<GitLabProject>> {
    let client = reqwest::blocking::Client::new();
    let base = config.gitlab_url.trim_end_matches('/');
    let url = format!("{base}/api/v4/projects");
    let mut page = 1usize;
    let mut all = Vec::new();

    loop {
        let mut query: Vec<(&str, String)> = vec![
            ("per_page", config.filters.per_page.to_string()),
            ("page", page.to_string()),
            ("simple", "true".to_string()),
            ("owned", "true".to_string()),
        ];
        if let Some(value) = &config.filters.visibility {
            query.push(("visibility", value.to_string()));
        }

        let resp = client
            .get(&url)
            .header("PRIVATE-TOKEN", &config.gitlab_token)
            .query(&query)
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

fn fetch_personal_projects(config: &Config) -> Result<PersonalProjects> {
    let user = fetch_current_user(config)?;
    let projects = fetch_owned_projects(config)?;
    let base = config.gitlab_url.trim_end_matches('/');
    let web_url = format!("{base}/{}", user.username);
    Ok(PersonalProjects {
        username: user.username,
        web_url,
        projects,
    })
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

fn read_env_bool_optional<F>(reader: &F, key: &str) -> Result<Option<bool>>
where
    F: Fn(&str) -> Option<String>,
{
    let Some(value) = read_env_optional(reader, key) else {
        return Ok(None);
    };
    let normalized = value.to_lowercase();
    match normalized.as_str() {
        "1" | "true" | "yes" | "on" => Ok(Some(true)),
        "0" | "false" | "no" | "off" => Ok(Some(false)),
        _ => anyhow::bail!("invalid boolean for {key}: {value}"),
    }
}

fn read_env_u16_optional<F>(reader: &F, key: &str) -> Result<Option<u16>>
where
    F: Fn(&str) -> Option<String>,
{
    let Some(value) = read_env_optional(reader, key) else {
        return Ok(None);
    };
    let parsed = value
        .parse::<u16>()
        .map_err(|_| anyhow::anyhow!("invalid integer for {key}: {value}"))?;
    Ok(Some(parsed))
}

fn read_env_u64_optional<F>(reader: &F, key: &str) -> Result<Option<u64>>
where
    F: Fn(&str) -> Option<String>,
{
    let Some(value) = read_env_optional(reader, key) else {
        return Ok(None);
    };
    let parsed = value
        .parse::<u64>()
        .map_err(|_| anyhow::anyhow!("invalid integer for {key}: {value}"))?;
    Ok(Some(parsed))
}

fn default_cache_path() -> PathBuf {
    let base = dirs::cache_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("gitlab-tree").join("cache.json")
}

#[derive(Clone)]
struct Node {
    name: String,
    kind: NodeKind,
    children: Vec<usize>,
    expanded: bool,
    url: String,
    path: String,
    visibility: String,
    last_activity: Option<String>,
}

#[derive(Clone, Copy)]
enum NodeKind {
    Group,
    Project,
}

struct Toast {
    message: String,
    remaining: u8,
}

#[derive(Clone, Copy)]
enum KeyAction {
    None,
    Quit,
    Reload,
}

struct App {
    nodes: Vec<Node>,
    roots: Vec<usize>,
    parent: Vec<Option<usize>>,
    selected: usize,
    config: Config,
    status: Option<String>,
    pending_g: bool,
    toast: Option<Toast>,
    search_query: Option<String>,
    search_mode: bool,
}

impl App {
    const TOAST_TTL: u8 = 10;

    fn sample_with_status(config: Config, status: String) -> Self {
        let mut nodes = Vec::new();

        let dev_platform = push_node(
            &mut nodes,
            "dev-platform",
            NodeKind::Group,
            "https://gitlab.example.com/dev-platform",
            "dev-platform",
            "private",
            None,
        );
        let data = push_node(
            &mut nodes,
            "data",
            NodeKind::Group,
            "https://gitlab.example.com/data",
            "data",
            "private",
            None,
        );
        let sec = push_node(
            &mut nodes,
            "security",
            NodeKind::Group,
            "https://gitlab.example.com/security",
            "security",
            "private",
            None,
        );

        let dev_backend = push_node(
            &mut nodes,
            "backend",
            NodeKind::Group,
            "https://gitlab.example.com/dev-platform/backend",
            "dev-platform/backend",
            "private",
            None,
        );
        let dev_frontend = push_node(
            &mut nodes,
            "frontend",
            NodeKind::Group,
            "https://gitlab.example.com/dev-platform/frontend",
            "dev-platform/frontend",
            "private",
            None,
        );
        let dev_platform_proj = push_node(
            &mut nodes,
            "platform-tools",
            NodeKind::Project,
            "https://gitlab.example.com/dev-platform/platform-tools",
            "dev-platform/platform-tools",
            "private",
            None,
        );
        nodes[dev_platform].children.extend([dev_backend, dev_frontend, dev_platform_proj]);

        let api = push_node(
            &mut nodes,
            "api",
            NodeKind::Project,
            "https://gitlab.example.com/dev-platform/backend/api",
            "dev-platform/backend/api",
            "private",
            None,
        );
        let auth = push_node(
            &mut nodes,
            "auth",
            NodeKind::Project,
            "https://gitlab.example.com/dev-platform/backend/auth",
            "dev-platform/backend/auth",
            "private",
            None,
        );
        nodes[dev_backend].children.extend([api, auth]);

        let web = push_node(
            &mut nodes,
            "web",
            NodeKind::Project,
            "https://gitlab.example.com/dev-platform/frontend/web",
            "dev-platform/frontend/web",
            "private",
            None,
        );
        let design = push_node(
            &mut nodes,
            "design-system",
            NodeKind::Project,
            "https://gitlab.example.com/dev-platform/frontend/design-system",
            "dev-platform/frontend/design-system",
            "private",
            None,
        );
        nodes[dev_frontend].children.extend([web, design]);

        let data_ingest = push_node(
            &mut nodes,
            "ingest",
            NodeKind::Group,
            "https://gitlab.example.com/data/ingest",
            "data/ingest",
            "private",
            None,
        );
        let data_models = push_node(
            &mut nodes,
            "models",
            NodeKind::Group,
            "https://gitlab.example.com/data/models",
            "data/models",
            "private",
            None,
        );
        let data_tools = push_node(
            &mut nodes,
            "data-tools",
            NodeKind::Project,
            "https://gitlab.example.com/data/data-tools",
            "data/data-tools",
            "private",
            None,
        );
        nodes[data].children.extend([data_ingest, data_models, data_tools]);

        let ingest = push_node(
            &mut nodes,
            "ingest",
            NodeKind::Project,
            "https://gitlab.example.com/data/ingest/ingest",
            "data/ingest/ingest",
            "private",
            None,
        );
        let pipeline = push_node(
            &mut nodes,
            "pipeline",
            NodeKind::Project,
            "https://gitlab.example.com/data/ingest/pipeline",
            "data/ingest/pipeline",
            "private",
            None,
        );
        nodes[data_ingest].children.extend([ingest, pipeline]);

        let fraud = push_node(
            &mut nodes,
            "fraud",
            NodeKind::Project,
            "https://gitlab.example.com/data/models/fraud",
            "data/models/fraud",
            "private",
            None,
        );
        let churn = push_node(
            &mut nodes,
            "churn",
            NodeKind::Project,
            "https://gitlab.example.com/data/models/churn",
            "data/models/churn",
            "private",
            None,
        );
        nodes[data_models].children.extend([fraud, churn]);

        let sec_tools = push_node(
            &mut nodes,
            "sec-tools",
            NodeKind::Project,
            "https://gitlab.example.com/security/sec-tools",
            "security/sec-tools",
            "private",
            None,
        );
        let audits = push_node(
            &mut nodes,
            "audits",
            NodeKind::Project,
            "https://gitlab.example.com/security/audits",
            "security/audits",
            "private",
            None,
        );
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
            toast: None,
            search_query: None,
            search_mode: false,
        }
    }

    fn from_gitlab(config: Config) -> Result<Self> {
        let cache = CacheStore::new(config.cache_path.clone(), config.cache_ttl);
        if let Some(cache) = cache.load()? {
            let total_projects: usize =
                cache.projects_by_group.iter().map(|entry| entry.projects.len()).sum();
            let personal_count = cache.personal.as_ref().map(|entry| entry.projects.len()).unwrap_or(0);
            let status = format!(
                "cache hit | groups: {}, projects: {}, personal: {}",
                cache.groups.len(),
                total_projects,
                personal_count
            );
            return Ok(Self::from_gitlab_data(
                cache.groups,
                cache.projects_by_group,
                cache.personal,
                config,
                status,
            ));
        }

        let groups = fetch_groups(&config)?;
        let projects = fetch_projects_by_group(&config, &groups)?;
        let personal = fetch_personal_projects(&config).ok();
        let total_projects: usize = projects.iter().map(|entry| entry.projects.len()).sum();
        let personal_count = personal.as_ref().map(|entry| entry.projects.len()).unwrap_or(0);
        let status = format!(
            "groups: {}, projects: {}, personal: {}",
            groups.len(),
            total_projects,
            personal_count
        );
        let cache_data = CacheData {
            created_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            groups: groups.clone(),
            projects_by_group: projects.clone(),
            personal: personal.clone(),
        };
        let _ = cache.store(&cache_data);
        Ok(Self::from_gitlab_data(
            groups,
            projects,
            personal,
            config,
            status,
        ))
    }

    fn from_gitlab_data(
        groups: Vec<GitLabGroup>,
        projects_by_group: Vec<GroupProjects>,
        personal: Option<PersonalProjects>,
        config: Config,
        status: String,
    ) -> Self {
        let mut nodes = Vec::new();
        let mut id_to_node = HashMap::new();
        for group in &groups {
            let node_id = push_node(
                &mut nodes,
                &group.name,
                NodeKind::Group,
                &group.web_url,
                &group.full_path,
                &group.visibility,
                None,
            );
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
                let project_node = push_node(
                    &mut nodes,
                    &project.name,
                    NodeKind::Project,
                    &project.web_url,
                    &project.path_with_namespace,
                    &project.visibility,
                    project.last_activity_at.clone(),
                );
                nodes[parent_node].children.push(project_node);
            }
        }

        if let Some(personal) = personal {
            let root = push_node(
                &mut nodes,
                &personal.username,
                NodeKind::Group,
                &personal.web_url,
                &personal.username,
                "private",
                None,
            );
            for project in personal.projects {
                let project_node = push_node(
                    &mut nodes,
                    &project.name,
                    NodeKind::Project,
                    &project.web_url,
                    &project.path_with_namespace,
                    &project.visibility,
                    project.last_activity_at.clone(),
                );
                nodes[root].children.push(project_node);
            }
            roots.push(root);
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
            toast: None,
            search_query: None,
            search_mode: false,
        }
    }

    fn visible_nodes(&self) -> Vec<VisibleNode> {
        let mut out = Vec::new();
        for &root in &self.roots {
            self.walk_visible(root, 0, &mut out);
        }
        if let Some(query) = &self.search_query {
            filter_visible_nodes(&out, &self.nodes, query)
        } else {
            out
        }
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

    fn yank_selected<C: ClipboardSink + ?Sized>(
        &mut self,
        visible: &[VisibleNode],
        clipboard: &mut C,
    ) -> Result<String> {
        if visible.is_empty() {
            anyhow::bail!("no selection");
        }
        let node_id = visible[self.selected].id;
        let url = self.nodes[node_id].url.clone();
        clipboard.set_text(url.clone())?;
        Ok(url)
    }

    fn open_selected<B: BrowserOpener + ?Sized>(
        &mut self,
        visible: &[VisibleNode],
        browser: &mut B,
    ) -> Result<String> {
        if visible.is_empty() {
            anyhow::bail!("no selection");
        }
        let node_id = visible[self.selected].id;
        let url = self.nodes[node_id].url.clone();
        browser.open(&url)?;
        Ok(url)
    }

    fn set_status(&mut self, message: String) {
        self.status = Some(message);
    }

    fn set_toast(&mut self, message: String) {
        self.toast = Some(Toast {
            message,
            remaining: Self::TOAST_TTL,
        });
    }

    fn tick_toast(&mut self) {
        if let Some(toast) = self.toast.as_mut() {
            if toast.remaining > 0 {
                toast.remaining -= 1;
            }
            if toast.remaining == 0 {
                self.toast = None;
            }
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

    fn start_search(&mut self) {
        self.search_mode = true;
        self.search_query = Some(String::new());
    }

    fn exit_search_mode(&mut self) {
        self.search_mode = false;
        if let Some(query) = &self.search_query {
            if query.is_empty() {
                self.search_query = None;
            }
        }
    }

    fn clear_search(&mut self) {
        self.search_query = None;
        self.search_mode = false;
    }

    fn push_search_char(&mut self, ch: char) {
        if let Some(query) = &mut self.search_query {
            query.push(ch);
        } else {
            self.search_query = Some(ch.to_string());
        }
    }

    fn pop_search_char(&mut self) {
        if let Some(query) = &mut self.search_query {
            query.pop();
            if query.is_empty() && !self.search_mode {
                self.search_query = None;
            }
        }
    }

    fn handle_key(
        &mut self,
        key: KeyCode,
        visible: &[VisibleNode],
        clipboard: Option<&mut dyn ClipboardSink>,
        browser: &mut dyn BrowserOpener,
    ) -> Result<KeyAction> {
        if self.search_mode {
            match key {
                KeyCode::Esc => self.clear_search(),
                KeyCode::Enter => self.exit_search_mode(),
                KeyCode::Backspace => self.pop_search_char(),
                KeyCode::Char(ch) => self.push_search_char(ch),
                _ => {}
            }
            return Ok(KeyAction::None);
        }

        let action = match key {
            KeyCode::Char('q') => KeyAction::Quit,
            KeyCode::Char('r') => KeyAction::Reload,
            KeyCode::Up => {
                self.move_up();
                KeyAction::None
            }
            KeyCode::Down => {
                self.move_down(visible.len());
                KeyAction::None
            }
            KeyCode::Left => {
                self.collapse_or_parent(visible);
                KeyAction::None
            }
            KeyCode::Right => {
                self.expand_or_child(visible);
                KeyAction::None
            }
            KeyCode::Char('k') => {
                self.move_up();
                KeyAction::None
            }
            KeyCode::Char('j') => {
                self.move_down(visible.len());
                KeyAction::None
            }
            KeyCode::Char('h') => {
                self.collapse_or_parent(visible);
                KeyAction::None
            }
            KeyCode::Char('l') => {
                self.expand_or_child(visible);
                KeyAction::None
            }
            KeyCode::Char('g') => {
                if self.consume_pending_g() {
                    self.move_top();
                } else {
                    self.set_pending_g();
                }
                KeyAction::None
            }
            KeyCode::Char('G') => {
                self.move_bottom(visible.len());
                KeyAction::None
            }
            KeyCode::Char('y') => {
                if let Some(clipboard) = clipboard {
                    match self.yank_selected(visible, clipboard) {
                        Ok(url) => {
                            self.set_status(format!("copied {url}"));
                            self.set_toast("Copied URL".to_string());
                        }
                        Err(err) => self.set_status(format!("copy failed: {err}")),
                    }
                } else {
                    self.set_status("clipboard unavailable".to_string());
                }
                KeyAction::None
            }
            KeyCode::Char('o') => {
                match self.open_selected(visible, browser) {
                    Ok(url) => self.set_status(format!("opened {url}")),
                    Err(err) => self.set_status(format!("open failed: {err}")),
                }
                KeyAction::None
            }
            KeyCode::Char('/') => {
                self.start_search();
                KeyAction::None
            }
            KeyCode::Esc => {
                self.clear_search();
                KeyAction::None
            }
            _ => KeyAction::None,
        };

        if key != KeyCode::Char('g') {
            self.clear_pending_g();
        }

        Ok(action)
    }
}

#[derive(Clone, Copy)]
struct VisibleNode {
    id: usize,
    depth: usize,
}

fn push_node(
    nodes: &mut Vec<Node>,
    name: &str,
    kind: NodeKind,
    url: &str,
    path: &str,
    visibility: &str,
    last_activity: Option<String>,
) -> usize {
    let id = nodes.len();
    nodes.push(Node {
        name: name.to_string(),
        kind,
        children: Vec::new(),
        expanded: false,
        url: url.to_string(),
        path: path.to_string(),
        visibility: visibility.to_string(),
        last_activity,
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

fn filter_visible_nodes(
    visible: &[VisibleNode],
    nodes: &[Node],
    query: &str,
) -> Vec<VisibleNode> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return visible.to_vec();
    }
    visible
        .iter()
        .copied()
        .filter(|node| fuzzy_match(&needle, &nodes[node.id].name.to_lowercase()))
        .collect()
}

fn fuzzy_match(needle: &str, haystack: &str) -> bool {
    let mut needle_chars = needle.chars();
    let mut current = needle_chars.next();
    for ch in haystack.chars() {
        match current {
            Some(target) if ch == target => {
                current = needle_chars.next();
                if current.is_none() {
                    return true;
                }
            }
            None => return true,
            _ => {}
        }
    }
    current.is_none()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyCode;
    use std::time::{Duration, UNIX_EPOCH};

    fn test_config() -> Config {
        Config {
            gitlab_url: "https://gitlab.com".to_string(),
            gitlab_token: "token".to_string(),
            filters: ApiFilters::default(),
            cache_path: default_cache_path(),
            cache_ttl: Duration::from_secs(300),
        }
    }

    #[test]
    fn visible_nodes_respects_expansion() {
        let mut nodes = Vec::new();
        let root = push_node(
            &mut nodes,
            "root",
            NodeKind::Group,
            "https://example.com/root",
            "root",
            "private",
            None,
        );
        let child = push_node(
            &mut nodes,
            "child",
            NodeKind::Project,
            "https://example.com/child",
            "root/child",
            "private",
            None,
        );
        nodes[root].children.push(child);

        let parent = build_parent_map(&nodes);
        let mut app = App {
            nodes,
            roots: vec![root],
            parent,
            selected: 0,
            config: test_config(),
            status: None,
            pending_g: false,
            toast: None,
            search_query: None,
            search_mode: false,
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
        assert_eq!(config.filters.per_page, 100);
        assert!(config.filters.all_available.is_none());
        assert_eq!(config.cache_ttl.as_secs(), 300);
        assert!(config
            .cache_path
            .ends_with(PathBuf::from("gitlab-tree").join("cache.json")));
    }

    #[test]
    fn config_from_env_reader_fails_without_token() {
        let reader = |_key: &str| None;
        let result = Config::from_env_reader(reader);
        assert!(result.is_err());
    }

    #[test]
    fn config_from_env_reader_parses_filters() {
        let reader = |key: &str| match key {
            "GITLAB_TOKEN" => Some("token".to_string()),
            "GITLAB_ALL_AVAILABLE" => Some("true".to_string()),
            "GITLAB_OWNED" => Some("0".to_string()),
            "GITLAB_TOP_LEVEL_ONLY" => Some("yes".to_string()),
            "GITLAB_INCLUDE_SUBGROUPS" => Some("on".to_string()),
            "GITLAB_VISIBILITY" => Some("private".to_string()),
            "GITLAB_PER_PAGE" => Some("50".to_string()),
            "GITLAB_CACHE_TTL_SECONDS" => Some("120".to_string()),
            "GITLAB_CACHE_PATH" => Some("/tmp/gitlab-tree-cache.json".to_string()),
            _ => None,
        };

        let config = Config::from_env_reader(reader).expect("config should load");
        assert_eq!(config.filters.per_page, 50);
        assert_eq!(config.filters.all_available, Some(true));
        assert_eq!(config.filters.owned, Some(false));
        assert_eq!(config.filters.top_level_only, Some(true));
        assert_eq!(config.filters.include_subgroups, Some(true));
        assert_eq!(config.filters.visibility.as_deref(), Some("private"));
        assert_eq!(config.cache_ttl.as_secs(), 120);
        assert_eq!(
            config.cache_path,
            PathBuf::from("/tmp/gitlab-tree-cache.json")
        );
    }

    #[test]
    fn config_from_env_reader_rejects_invalid_bool() {
        let reader = |key: &str| match key {
            "GITLAB_TOKEN" => Some("token".to_string()),
            "GITLAB_OWNED" => Some("maybe".to_string()),
            _ => None,
        };

        let result = Config::from_env_reader(reader);
        assert!(result.is_err());
    }

    #[test]
    fn loading_message_cycles_frames() {
        assert_eq!(loading_message(0), "| loading GitLab data...");
        assert_eq!(loading_message(1), "/ loading GitLab data...");
        assert_eq!(loading_message(2), "- loading GitLab data...");
        assert_eq!(loading_message(3), "\\ loading GitLab data...");
        assert_eq!(loading_message(4), "| loading GitLab data...");
    }

    #[test]
    fn format_node_details_includes_metadata() {
        let node = Node {
            name: "root".to_string(),
            kind: NodeKind::Group,
            children: Vec::new(),
            expanded: false,
            url: "https://example.com/root".to_string(),
            path: "root".to_string(),
            visibility: "private".to_string(),
            last_activity: None,
        };

        let lines = format_node_details(&node);
        assert!(lines.iter().any(|line| line == "Name: root"));
        assert!(lines.iter().any(|line| line == "Kind: Group"));
        assert!(lines.iter().any(|line| line == "Path: root"));
        assert!(lines.iter().any(|line| line == "Visibility: private"));
        assert!(lines.iter().any(|line| line == "URL: https://example.com/root"));
        assert!(!lines.iter().any(|line| line.starts_with("Last activity:")));
    }

    #[test]
    fn format_node_details_includes_last_activity_when_present() {
        let node = Node {
            name: "proj".to_string(),
            kind: NodeKind::Project,
            children: Vec::new(),
            expanded: false,
            url: "https://example.com/root/proj".to_string(),
            path: "root/proj".to_string(),
            visibility: "internal".to_string(),
            last_activity: Some("2024-01-01T00:00:00Z".to_string()),
        };

        let lines = format_node_details(&node);
        assert!(lines
            .iter()
            .any(|line| line == "Last activity: 2024-01-01T00:00:00Z"));
    }

    #[test]
    fn filter_visible_nodes_matches_query_case_insensitive() {
        let nodes = vec![
            Node {
                name: "API".to_string(),
                kind: NodeKind::Project,
                children: Vec::new(),
                expanded: false,
                url: "https://example.com/api".to_string(),
                path: "root/api".to_string(),
                visibility: "private".to_string(),
                last_activity: None,
            },
            Node {
                name: "web".to_string(),
                kind: NodeKind::Project,
                children: Vec::new(),
                expanded: false,
                url: "https://example.com/web".to_string(),
                path: "root/web".to_string(),
                visibility: "private".to_string(),
                last_activity: None,
            },
        ];
        let visible = vec![
            VisibleNode { id: 0, depth: 0 },
            VisibleNode { id: 1, depth: 0 },
        ];

        let filtered = filter_visible_nodes(&visible, &nodes, "api");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, 0);
    }

    #[test]
    fn filter_visible_nodes_matches_fuzzy_subsequence() {
        let nodes = vec![Node {
            name: "gitlab".to_string(),
            kind: NodeKind::Project,
            children: Vec::new(),
            expanded: false,
            url: "https://example.com/gitlab".to_string(),
            path: "root/gitlab".to_string(),
            visibility: "private".to_string(),
            last_activity: None,
        }];
        let visible = vec![VisibleNode { id: 0, depth: 0 }];

        let filtered = filter_visible_nodes(&visible, &nodes, "glb");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, 0);
    }

    #[test]
    fn handle_key_returns_reload_on_r() {
        let mut nodes = Vec::new();
        let root = push_node(
            &mut nodes,
            "root",
            NodeKind::Group,
            "https://example.com/root",
            "root",
            "private",
            None,
        );
        let parent = build_parent_map(&nodes);
        let mut app = App {
            nodes,
            roots: vec![root],
            parent,
            selected: 0,
            config: test_config(),
            status: None,
            pending_g: false,
            toast: None,
            search_query: None,
            search_mode: false,
        };

        let visible = app.visible_nodes();
        let mut browser = MockBrowser { opened: None };
        let action = app
            .handle_key(KeyCode::Char('r'), &visible, None, &mut browser)
            .expect("handle key");

        matches!(action, KeyAction::Reload);
        if let KeyAction::Reload = action {
        } else {
            panic!("expected reload action");
        }
    }

    #[test]
    fn select_clipboard_prefers_arboard() {
        let probe = MockClipboardProbe {
            arboard_ok: true,
            has_wayland: true,
            has_display: true,
            has_wl_copy: true,
            has_xclip: true,
        };

        assert_eq!(
            select_clipboard_backend(&probe),
            ClipboardBackend::Arboard
        );
    }

    #[test]
    fn select_clipboard_uses_wl_copy_on_wayland() {
        let probe = MockClipboardProbe {
            arboard_ok: false,
            has_wayland: true,
            has_display: true,
            has_wl_copy: true,
            has_xclip: true,
        };

        assert_eq!(
            select_clipboard_backend(&probe),
            ClipboardBackend::WlCopy
        );
    }

    #[test]
    fn select_clipboard_uses_xclip_on_x11() {
        let probe = MockClipboardProbe {
            arboard_ok: false,
            has_wayland: false,
            has_display: true,
            has_wl_copy: false,
            has_xclip: true,
        };

        assert_eq!(
            select_clipboard_backend(&probe),
            ClipboardBackend::Xclip
        );
    }

    #[test]
    fn select_clipboard_none_when_unavailable() {
        let probe = MockClipboardProbe {
            arboard_ok: false,
            has_wayland: false,
            has_display: false,
            has_wl_copy: false,
            has_xclip: false,
        };

        assert_eq!(
            select_clipboard_backend(&probe),
            ClipboardBackend::None
        );
    }

    #[test]
    fn from_gitlab_data_builds_parent_child_relationships() {
        let groups = vec![
            GitLabGroup {
                id: 1,
                name: "root".to_string(),
                web_url: "https://example.com/root".to_string(),
                full_path: "root".to_string(),
                visibility: "private".to_string(),
                parent_id: None,
            },
            GitLabGroup {
                id: 2,
                name: "child".to_string(),
                web_url: "https://example.com/root/child".to_string(),
                full_path: "root/child".to_string(),
                visibility: "private".to_string(),
                parent_id: Some(1),
            },
        ];
        let projects = vec![GroupProjects {
            group_id: 1,
            projects: vec![GitLabProject {
                name: "proj".to_string(),
                web_url: "https://example.com/root/proj".to_string(),
                path_with_namespace: "root/proj".to_string(),
                visibility: "private".to_string(),
                last_activity_at: Some("2024-01-01T00:00:00Z".to_string()),
            }],
        }];

        let app = App::from_gitlab_data(
            groups,
            projects,
            None,
            test_config(),
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
    fn from_gitlab_data_adds_personal_projects_root() {
        let personal = PersonalProjects {
            username: "alice".to_string(),
            web_url: "https://example.com/alice".to_string(),
            projects: vec![GitLabProject {
                name: "notes".to_string(),
                web_url: "https://example.com/alice/notes".to_string(),
                path_with_namespace: "alice/notes".to_string(),
                visibility: "private".to_string(),
                last_activity_at: None,
            }],
        };

        let app = App::from_gitlab_data(
            Vec::new(),
            Vec::new(),
            Some(personal),
            test_config(),
            "personal: 1".to_string(),
        );

        assert_eq!(app.roots.len(), 1);
        let root_id = app.roots[0];
        assert_eq!(app.nodes[root_id].name, "alice");
        assert_eq!(app.nodes[root_id].children.len(), 1);
        let project_id = app.nodes[root_id].children[0];
        assert_eq!(app.nodes[project_id].name, "notes");
        assert_eq!(app.parent[project_id], Some(root_id));
    }

    #[test]
    fn vim_navigation_helpers_update_selection() {
        let mut nodes = Vec::new();
        let root = push_node(
            &mut nodes,
            "root",
            NodeKind::Group,
            "https://example.com/root",
            "root",
            "private",
            None,
        );
        let child = push_node(
            &mut nodes,
            "child",
            NodeKind::Project,
            "https://example.com/child",
            "root/child",
            "private",
            None,
        );
        nodes[root].children.push(child);
        nodes[root].expanded = true;

        let parent = build_parent_map(&nodes);
        let mut app = App {
            nodes,
            roots: vec![root],
            parent,
            selected: 1,
            config: test_config(),
            status: None,
            pending_g: false,
            toast: None,
            search_query: None,
            search_mode: false,
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
            config: test_config(),
            status: None,
            pending_g: false,
            toast: None,
            search_query: None,
            search_mode: false,
        };

        assert!(!app.consume_pending_g());
        app.set_pending_g();
        assert!(app.consume_pending_g());
        assert!(!app.consume_pending_g());
    }

    #[test]
    fn yank_selected_copies_url() {
        let mut nodes = Vec::new();
        let root = push_node(
            &mut nodes,
            "root",
            NodeKind::Group,
            "https://example.com/root",
            "root",
            "private",
            None,
        );
        let parent = build_parent_map(&nodes);
        let mut app = App {
            nodes,
            roots: vec![root],
            parent,
            selected: 0,
            config: test_config(),
            status: None,
            pending_g: false,
            toast: None,
            search_query: None,
            search_mode: false,
        };

        let visible = app.visible_nodes();
        let mut clipboard = MockClipboard { text: None };
        let url = app
            .yank_selected(&visible, &mut clipboard)
            .expect("yank should succeed");

        assert_eq!(url, "https://example.com/root");
        assert_eq!(clipboard.text.as_deref(), Some("https://example.com/root"));
    }

    #[test]
    fn open_selected_opens_url() {
        let mut nodes = Vec::new();
        let root = push_node(
            &mut nodes,
            "root",
            NodeKind::Group,
            "https://example.com/root",
            "root",
            "private",
            None,
        );
        let parent = build_parent_map(&nodes);
        let mut app = App {
            nodes,
            roots: vec![root],
            parent,
            selected: 0,
            config: test_config(),
            status: None,
            pending_g: false,
            toast: None,
            search_query: None,
            search_mode: false,
        };

        let visible = app.visible_nodes();
        let mut browser = MockBrowser { opened: None };
        let url = app
            .open_selected(&visible, &mut browser)
            .expect("open should succeed");

        assert_eq!(url, "https://example.com/root");
        assert_eq!(browser.opened.as_deref(), Some("https://example.com/root"));
    }

    #[test]
    fn toast_expires_after_ticks() {
        let mut app = App {
            nodes: Vec::new(),
            roots: Vec::new(),
            parent: Vec::new(),
            selected: 0,
            config: test_config(),
            status: None,
            pending_g: false,
            toast: None,
            search_query: None,
            search_mode: false,
        };

        app.set_toast("Copied URL".to_string());
        for _ in 0..App::TOAST_TTL {
            app.tick_toast();
        }

        assert!(app.toast.is_none());
    }

    #[test]
    fn cache_is_valid_respects_ttl() {
        let ttl = Duration::from_secs(10);
        let now = UNIX_EPOCH + Duration::from_secs(100);
        assert!(cache_is_valid(95, ttl, now));
        assert!(!cache_is_valid(80, ttl, now));
    }

    #[test]
    fn cache_store_round_trip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("cache.json");
        let store = CacheStore::new(path, Duration::from_secs(60));
        let created_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let data = CacheData {
            created_at,
            groups: vec![GitLabGroup {
                id: 1,
                name: "root".to_string(),
                web_url: "https://example.com/root".to_string(),
                full_path: "root".to_string(),
                visibility: "private".to_string(),
                parent_id: None,
            }],
            projects_by_group: vec![GroupProjects {
                group_id: 1,
                projects: vec![GitLabProject {
                    name: "proj".to_string(),
                    web_url: "https://example.com/root/proj".to_string(),
                    path_with_namespace: "root/proj".to_string(),
                    visibility: "private".to_string(),
                    last_activity_at: Some("2024-01-01T00:00:00Z".to_string()),
                }],
            }],
            personal: None,
        };

        store.store(&data).expect("store cache");
        let loaded = store.load().expect("load cache");
        let loaded = loaded.expect("cache should be present");

        assert_eq!(loaded.groups.len(), 1);
        assert_eq!(loaded.groups[0].name, "root");
        assert_eq!(loaded.projects_by_group.len(), 1);
        assert_eq!(loaded.projects_by_group[0].projects[0].name, "proj");
    }

    struct MockClipboardProbe {
        arboard_ok: bool,
        has_wayland: bool,
        has_display: bool,
        has_wl_copy: bool,
        has_xclip: bool,
    }

    impl ClipboardProbe for MockClipboardProbe {
        fn arboard_ok(&self) -> bool {
            self.arboard_ok
        }

        fn has_wayland(&self) -> bool {
            self.has_wayland
        }

        fn has_display(&self) -> bool {
            self.has_display
        }

        fn command_exists(&self, command: &str) -> bool {
            match command {
                "wl-copy" => self.has_wl_copy,
                "xclip" => self.has_xclip,
                _ => false,
            }
        }
    }

    struct MockClipboard {
        text: Option<String>,
    }

    impl ClipboardSink for MockClipboard {
        fn set_text(&mut self, text: String) -> Result<()> {
            self.text = Some(text);
            Ok(())
        }
    }

    struct MockBrowser {
        opened: Option<String>,
    }

    impl BrowserOpener for MockBrowser {
        fn open(&mut self, url: &str) -> Result<()> {
            self.opened = Some(url.to_string());
            Ok(())
        }
    }
}
