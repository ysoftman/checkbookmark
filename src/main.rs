use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use clap::Parser;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use futures::stream::{self, StreamExt};
use ratatui::prelude::*;
use ratatui::widgets::*;
use serde::Deserialize;

/// CLI 인자 정의
#[derive(Parser)]
#[command(name = "bookmark-check", about = "Check Chrome bookmarks URL validity")]
struct Cli {
    /// Chrome 북마크 파일 경로
    #[arg(short, long)]
    file: Option<PathBuf>,

    /// 동시 요청 수
    #[arg(short, long, default_value_t = 10)]
    concurrency: usize,

    /// 요청 타임아웃(초)
    #[arg(short, long, default_value_t = 3)]
    timeout: u64,
}

/// Chrome 북마크 JSON 최상위 구조
#[derive(Deserialize)]
struct Bookmarks {
    roots: Roots,
}

/// 북마크 루트 노드 (bookmark_bar, other, synced)
#[derive(Deserialize)]
struct Roots {
    bookmark_bar: BookmarkNode,
    other: BookmarkNode,
    synced: BookmarkNode,
}

/// 북마크 노드 (폴더 또는 URL)
#[derive(Deserialize)]
struct BookmarkNode {
    #[serde(default)]
    name: String,
    #[serde(rename = "type")]
    node_type: String,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    children: Option<Vec<BookmarkNode>>,
}

/// 파싱된 북마크 항목
struct BookmarkEntry {
    name: String,
    url: String,
}

/// URL 검사 결과
#[derive(Debug, Clone)]
struct CheckResult {
    name: String,
    url: String,
    status: String,
    is_valid: bool,
}

/// 탐색된 Chrome 프로필 정보
struct ProfileInfo {
    dir_name: String,
    display_name: String,
    bookmarks_path: PathBuf,
}

/// 북마크 노드를 재귀적으로 탐색하여 URL 항목을 수집
fn collect_urls(node: &BookmarkNode, folder_path: &str, entries: &mut Vec<BookmarkEntry>) {
    let current_path = if folder_path.is_empty() {
        node.name.clone()
    } else {
        format!("{}/{}", folder_path, node.name)
    };

    if node.node_type == "url" {
        if let Some(url) = &node.url {
            entries.push(BookmarkEntry {
                name: node.name.clone(),
                url: url.clone(),
            });
        }
    }

    if let Some(children) = &node.children {
        for child in children {
            collect_urls(child, &current_path, entries);
        }
    }
}

/// OS별 Chrome 데이터 기본 디렉토리 반환
fn chrome_base_dir() -> PathBuf {
    let home = dirs::home_dir().expect("Failed to get home directory");
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Google/Chrome")
    } else if cfg!(target_os = "linux") {
        home.join(".config/google-chrome")
    } else {
        home.join(r"AppData\Local\Google\Chrome\User Data")
    }
}

/// Chrome 디렉토리를 스캔하여 Bookmarks 파일이 있는 프로필 목록 반환
fn discover_profiles() -> Vec<ProfileInfo> {
    let base = chrome_base_dir();
    if !base.exists() {
        return Vec::new();
    }

    let mut profiles = Vec::new();
    let Ok(entries) = std::fs::read_dir(&base) else {
        return Vec::new();
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let bookmarks_path = path.join("Bookmarks");
        if !bookmarks_path.exists() {
            continue;
        }

        let dir_name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let display_name = path
            .join("Preferences")
            .pipe_read()
            .and_then(|content| {
                let v: serde_json::Value = serde_json::from_str(&content).ok()?;
                v["profile"]["name"].as_str().map(String::from)
            })
            .unwrap_or_else(|| dir_name.clone());

        profiles.push(ProfileInfo {
            dir_name,
            display_name,
            bookmarks_path,
        });
    }

    profiles.sort_by(|a, b| a.dir_name.cmp(&b.dir_name));
    profiles
}

trait PipeRead {
    fn pipe_read(&self) -> Option<String>;
}

impl PipeRead for PathBuf {
    fn pipe_read(&self) -> Option<String> {
        std::fs::read_to_string(self).ok()
    }
}

/// URL에 HTTP HEAD 요청을 보내 상태 코드와 상태 텍스트 반환
async fn check_url(client: &reqwest::Client, url: &str) -> (u16, String) {
    match client.head(url).send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            (status, resp.status().to_string())
        }
        Err(e) => {
            if e.is_timeout() {
                (0, "TIMEOUT".to_string())
            } else if e.is_connect() {
                (0, "CONN_ERR".to_string())
            } else {
                let msg = e.to_string();
                let short = if msg.len() > 20 {
                    format!("{}...", &msg[..20])
                } else {
                    msg
                };
                (0, short)
            }
        }
    }
}

/// 북마크 파일을 읽어 URL 항목 목록으로 파싱
fn parse_bookmarks(path: &PathBuf) -> Vec<BookmarkEntry> {
    let content = std::fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("ERROR: Failed to read file: {e}");
        std::process::exit(1);
    });

    let bookmarks: Bookmarks = serde_json::from_str(&content).unwrap_or_else(|e| {
        eprintln!("ERROR: Failed to parse bookmarks: {e}");
        std::process::exit(1);
    });

    let mut entries = Vec::new();
    collect_urls(&bookmarks.roots.bookmark_bar, "", &mut entries);
    collect_urls(&bookmarks.roots.other, "", &mut entries);
    collect_urls(&bookmarks.roots.synced, "", &mut entries);
    entries
}

/// TUI 앱 상태
struct App {
    profile_name: String,
    results: Vec<CheckResult>,
    total: usize,
    checked: usize,
    invalid: usize,
    table_state: TableState,
    checking_done: bool,
    sort_by_status: bool,
    concurrency: usize,
    timeout: u64,
}

impl App {
    fn new(profile_name: String, total: usize, concurrency: usize, timeout: u64) -> Self {
        let mut table_state = TableState::default();
        if total > 0 {
            table_state.select(Some(0));
        }
        Self {
            profile_name,
            results: Vec::new(),
            total,
            checked: 0,
            invalid: 0,
            table_state,
            checking_done: false,
            sort_by_status: false,
            concurrency,
            timeout,
        }
    }

    fn sorted_results(&self) -> Vec<&CheckResult> {
        let mut results: Vec<&CheckResult> = self.results.iter().collect();
        if self.sort_by_status {
            // invalid(status 비정상)을 먼저, valid를 나중에 표시
            results.sort_by(|a, b| a.is_valid.cmp(&b.is_valid).then(a.status.cmp(&b.status)));
        }
        results
    }

    fn toggle_sort_by_status(&mut self) {
        self.sort_by_status = !self.sort_by_status;
        self.table_state.select(Some(0));
    }

    fn row_count(&self) -> usize {
        self.sorted_results().len()
    }

    fn scroll_down(&mut self) {
        let len = self.row_count();
        if len == 0 {
            return;
        }
        let i = match self.table_state.selected() {
            Some(i) => (i + 1).min(len - 1),
            None => 0,
        };
        self.table_state.select(Some(i));
    }

    fn scroll_up(&mut self) {
        let i = match self.table_state.selected() {
            Some(i) => i.saturating_sub(1),
            None => 0,
        };
        self.table_state.select(Some(i));
    }

    fn page_down(&mut self) {
        let len = self.row_count();
        if len == 0 {
            return;
        }
        let i = match self.table_state.selected() {
            Some(i) => (i + 20).min(len - 1),
            None => 0,
        };
        self.table_state.select(Some(i));
    }

    fn page_up(&mut self) {
        let i = match self.table_state.selected() {
            Some(i) => i.saturating_sub(20),
            None => 0,
        };
        self.table_state.select(Some(i));
    }

    fn go_top(&mut self) {
        self.table_state.select(Some(0));
    }

    fn go_bottom(&mut self) {
        let len = self.row_count();
        if len > 0 {
            self.table_state.select(Some(len - 1));
        }
    }

    fn handle_key(&mut self, key: &crossterm::event::KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return true,
            KeyCode::Up | KeyCode::Char('k') => self.scroll_up(),
            KeyCode::Down | KeyCode::Char('j') => self.scroll_down(),
            KeyCode::PageDown => self.page_down(),
            KeyCode::PageUp => self.page_up(),
            KeyCode::Char('d') if ctrl => self.page_down(),
            KeyCode::Char('u') if ctrl => self.page_up(),
            KeyCode::Char('g') => self.go_top(),
            KeyCode::Char('G') => self.go_bottom(),
            KeyCode::Char('s') => self.toggle_sort_by_status(),
            _ => {}
        }
        false
    }
}

/// TUI 프로필 선택 화면
fn run_profile_selector(profiles: &[ProfileInfo]) -> io::Result<Option<usize>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut selected: usize = 0;
    let result = loop {
        terminal.draw(|f| {
            let area = f.area();
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .horizontal_margin(0)
                .vertical_margin(0)
                .constraints([Constraint::Min(3), Constraint::Length(3)])
                .split(area);

            let items: Vec<Row> = profiles
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let style = if i == selected {
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Cyan)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };
                    Row::new(vec![
                        Cell::from(format!("{}", i + 1)),
                        Cell::from(p.display_name.as_str()),
                        Cell::from(p.dir_name.as_str()),
                    ])
                    .style(style)
                })
                .collect();

            let table = Table::new(
                items,
                [
                    Constraint::Length(4),
                    Constraint::Percentage(50),
                    Constraint::Percentage(50),
                ],
            )
            .header(
                Row::new(vec!["#", "Profile", "Directory"])
                    .style(Style::default().add_modifier(Modifier::BOLD))
                    .bottom_margin(1),
            )
            .block(
                Block::default()
                    .title(" Chrome Profiles ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Cyan)),
            )
            .highlight_symbol(">> ");

            f.render_widget(table, chunks[0]);

            let help = Paragraph::new(" [↑/↓/j/k] Navigate  [Enter] Select  [q] Quit")
                .style(Style::default().fg(Color::DarkGray))
                .block(Block::default().borders(Borders::ALL));
            f.render_widget(help, chunks[1]);
        })?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break None,
                    KeyCode::Up | KeyCode::Char('k') => {
                        selected = selected.saturating_sub(1);
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        selected = (selected + 1).min(profiles.len() - 1);
                    }
                    KeyCode::Enter => break Some(selected),
                    _ => {}
                }
            }
        }
    };

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    Ok(result)
}

/// TUI 메인 화면 렌더링
fn render_app(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .horizontal_margin(0)
        .vertical_margin(0)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(3),
        ])
        .split(area);

    // 상단: 프로필 정보 + 설정값 + 진행률
    let settings = format!(
        "Concurrency: {}  Timeout: {}s",
        app.concurrency, app.timeout
    );
    let header_text = if app.checking_done {
        let valid = app.total - app.invalid;
        let sort_status = if app.sort_by_status {
            "  [Sort: Status]"
        } else {
            ""
        };
        format!(
            " Profile: {}  |  Total: {}  Valid: {}  Invalid: {}  |  {}{}",
            app.profile_name, app.total, valid, app.invalid, settings, sort_status
        )
    } else {
        format!(
            " Profile: {}  |  Checking: {}/{}  |  {}",
            app.profile_name, app.checked, app.total, settings
        )
    };

    let progress_ratio = if app.total > 0 {
        app.checked as f64 / app.total as f64
    } else {
        1.0
    };

    let gauge = Gauge::default()
        .block(
            Block::default()
                .title(header_text)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)),
        )
        .gauge_style(
            Style::default()
                .fg(if app.checking_done {
                    Color::Green
                } else {
                    Color::Yellow
                })
                .bg(Color::DarkGray),
        )
        .ratio(progress_ratio);
    f.render_widget(gauge, chunks[0]);

    // 중앙: 결과 테이블
    let sorted: Vec<CheckResult> = app.sorted_results().into_iter().cloned().collect();
    let rows: Vec<Row> = sorted
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let status_style = if r.is_valid {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::Red)
            };
            Row::new(vec![
                Cell::from(format!("{}", i + 1)),
                Cell::from(r.status.as_str()).style(status_style),
                Cell::from(r.name.as_str()),
                Cell::from(r.url.as_str()),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(6),
            Constraint::Length(18),
            Constraint::Percentage(30),
            Constraint::Percentage(55),
        ],
    )
    .header(
        Row::new(vec!["#", "STATUS", "NAME", "URL"])
            .style(
                Style::default()
                    .add_modifier(Modifier::BOLD)
                    .fg(Color::Cyan),
            )
            .bottom_margin(1),
    )
    .block(
        Block::default()
            .title(" Results ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan)),
    )
    .row_highlight_style(Style::default().bg(Color::DarkGray))
    .highlight_symbol(">> ");

    f.render_stateful_widget(table, chunks[1], &mut app.table_state);

    // 하단: 도움말
    let help = Paragraph::new(
        " [↑/↓/j/k] Navigate  [PgUp/PgDn/C-u/C-d] Page  [g/G] Top/Bottom  [s] Sort Status  [q] Quit",
    )
    .style(Style::default().fg(Color::DarkGray))
    .block(Block::default().borders(Borders::ALL));
    f.render_widget(help, chunks[2]);
}

/// 검사 중 실시간 TUI 업데이트 루프
async fn run_check_tui(
    app: Arc<Mutex<App>>,
    entries: Vec<BookmarkEntry>,
    client: Arc<reqwest::Client>,
    concurrency: usize,
) -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let total = entries.len();
    let checked_count = Arc::new(AtomicUsize::new(0));

    // 백그라운드에서 URL 검사 실행
    let app_clone = app.clone();
    let checked_clone = checked_count.clone();
    let check_handle = tokio::spawn(async move {
        stream::iter(entries)
            .map(|entry| {
                let client = client.clone();
                let app = app_clone.clone();
                let checked = checked_clone.clone();
                async move {
                    let (status_code, status_text) = check_url(&client, &entry.url).await;
                    let is_valid = (200..400).contains(&status_code);
                    let done = checked.fetch_add(1, Ordering::Relaxed) + 1;

                    let result = CheckResult {
                        name: entry.name,
                        url: entry.url,
                        status: status_text,
                        is_valid,
                    };

                    let mut app = app.lock().unwrap();
                    app.results.push(result);
                    app.checked = done;
                    if !is_valid {
                        app.invalid += 1;
                    }
                }
            })
            .buffer_unordered(concurrency)
            .collect::<Vec<()>>()
            .await;
    });

    // TUI 렌더링 + 이벤트 루프
    'outer: loop {
        {
            let mut app = app.lock().unwrap();
            terminal.draw(|f| render_app(f, &mut app))?;
        }

        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                let mut app = app.lock().unwrap();
                if app.handle_key(&key) {
                    break 'outer;
                }
            }
        }

        let done = checked_count.load(Ordering::Relaxed);
        if done >= total {
            {
                let mut locked = app.lock().unwrap();
                locked.checking_done = true;
            }
            // 검사 완료 후 이벤트 루프
            loop {
                {
                    let mut locked = app.lock().unwrap();
                    terminal.draw(|f| render_app(f, &mut locked))?;
                }
                if event::poll(Duration::from_millis(100))? {
                    if let Event::Key(key) = event::read()? {
                        if key.kind != KeyEventKind::Press {
                            continue;
                        }
                        let mut locked = app.lock().unwrap();
                        if locked.handle_key(&key) {
                            break 'outer;
                        }
                    }
                }
            }
        }
    }

    check_handle.abort();
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;

    let app = app.lock().unwrap();
    if app.invalid > 0 {
        std::process::exit(1);
    }

    Ok(())
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let client = Arc::new(
        reqwest::Client::builder()
            .timeout(Duration::from_secs(cli.timeout))
            .redirect(reqwest::redirect::Policy::limited(10))
            .danger_accept_invalid_certs(true)
            .build()
            .expect("Failed to create HTTP client"),
    );

    // 북마크 파일 경로 결정
    let bookmark_files: Vec<(String, PathBuf)> = if let Some(file) = cli.file {
        vec![("custom".to_string(), file)]
    } else {
        let profiles = discover_profiles();
        if profiles.is_empty() {
            eprintln!("ERROR: No Chrome profiles found");
            std::process::exit(1);
        }

        // TUI 프로필 선택
        match run_profile_selector(&profiles) {
            Ok(Some(idx)) => {
                let p = &profiles[idx];
                vec![(p.display_name.clone(), p.bookmarks_path.clone())]
            }
            Ok(None) => std::process::exit(0),
            Err(e) => {
                eprintln!("ERROR: {e}");
                std::process::exit(1);
            }
        }
    };

    for (profile_name, path) in &bookmark_files {
        if !path.exists() {
            eprintln!("ERROR: Bookmarks file not found: {}", path.display());
            continue;
        }

        let entries = parse_bookmarks(path);
        let total = entries.len();
        let app = Arc::new(Mutex::new(App::new(
            profile_name.clone(),
            total,
            cli.concurrency,
            cli.timeout,
        )));
        if run_check_tui(app, entries, client.clone(), cli.concurrency)
            .await
            .is_err()
        {
            eprintln!("ERROR: TUI failed");
        }
    }
}
