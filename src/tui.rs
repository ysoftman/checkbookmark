use std::collections::HashSet;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossterm::cursor::Show;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use futures::stream::{self, StreamExt};
use ratatui::prelude::*;
use ratatui::widgets::*;
use unicode_width::UnicodeWidthStr;

use crate::bookmark::*;

/// TUI 앱 상태
pub struct App {
    pub profile_name: String,
    pub results: Vec<CheckResult>,
    pub total: usize,
    pub checked: usize,
    pub invalid: usize,
    pub table_state: TableState,
    pub checking_done: bool,
    pub sort_mode: SortMode,
    pub sort_ascending: bool,
    pub concurrency: usize,
    pub timeout: u64,
    pub search_mode: bool,
    pub search_query: String,
    pub refresh_requested: bool,
    pub pending_d: bool,
    pub confirm_delete: bool,
    pub delete_target_url: String,
    pub bookmarks_path: PathBuf,
    pub edit_mode: bool,
    pub edit_field: EditField,
    pub edit_folder: String,
    pub edit_name: String,
    pub edit_url: String,
    pub edit_cursor: usize,
    pub edit_original_url: String,
    pub edit_original_folder: String,
    pub selected_urls: HashSet<String>,
    pub chrome_warning: bool,
    pub popup_message: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
pub enum SortMode {
    None,
    Status,
    Folder,
    Name,
    Url,
}

#[derive(Clone, Copy, PartialEq)]
pub enum EditField {
    Folder,
    Name,
    Url,
}

impl App {
    pub fn new(
        profile_name: String,
        total: usize,
        concurrency: usize,
        timeout: u64,
        bookmarks_path: PathBuf,
    ) -> Self {
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
            sort_mode: SortMode::None,
            sort_ascending: true,
            concurrency,
            timeout,
            search_mode: false,
            search_query: String::new(),
            refresh_requested: false,
            pending_d: false,
            confirm_delete: false,
            delete_target_url: String::new(),
            bookmarks_path,
            edit_mode: false,
            edit_field: EditField::Folder,
            edit_folder: String::new(),
            edit_name: String::new(),
            edit_url: String::new(),
            edit_cursor: 0,
            edit_original_url: String::new(),
            edit_original_folder: String::new(),
            selected_urls: HashSet::new(),
            chrome_warning: false,
            popup_message: None,
        }
    }

    pub fn sorted_results(&self) -> Vec<&CheckResult> {
        let query = self.search_query.to_lowercase();
        let mut results: Vec<&CheckResult> = if query.is_empty() {
            self.results.iter().collect()
        } else {
            self.results
                .iter()
                .filter(|r| {
                    r.folder.to_lowercase().contains(&query)
                        || r.name.to_lowercase().contains(&query)
                        || r.url.to_lowercase().contains(&query)
                        || r.status.to_lowercase().contains(&query)
                })
                .collect()
        };
        let asc = self.sort_ascending;
        match self.sort_mode {
            SortMode::None => {}
            SortMode::Status => {
                results.sort_by(|a, b| {
                    let cmp = a.is_valid.cmp(&b.is_valid).then(a.status.cmp(&b.status));
                    if asc { cmp } else { cmp.reverse() }
                });
            }
            SortMode::Folder => {
                results.sort_by(|a, b| {
                    let cmp = a.folder.to_lowercase().cmp(&b.folder.to_lowercase());
                    if asc { cmp } else { cmp.reverse() }
                });
            }
            SortMode::Name => {
                results.sort_by(|a, b| {
                    let cmp = a.name.to_lowercase().cmp(&b.name.to_lowercase());
                    if asc { cmp } else { cmp.reverse() }
                });
            }
            SortMode::Url => {
                results.sort_by(|a, b| {
                    let cmp = a.url.to_lowercase().cmp(&b.url.to_lowercase());
                    if asc { cmp } else { cmp.reverse() }
                });
            }
        }
        results
    }

    fn set_sort_mode(&mut self, mode: SortMode) {
        if self.sort_mode == mode {
            self.sort_ascending = !self.sort_ascending;
        } else {
            self.sort_mode = mode;
            self.sort_ascending = true;
        }
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

    pub fn handle_key(&mut self, key: &crossterm::event::KeyEvent) -> bool {
        // 팝업 메시지 닫기
        if self.popup_message.is_some() {
            self.popup_message = None;
            return false;
        }
        // Chrome 실행 경고
        if self.chrome_warning {
            self.chrome_warning = false;
            return false;
        }
        // 삭제 확인 모드
        if self.confirm_delete {
            match key.code {
                KeyCode::Char('y') => {
                    self.confirm_delete = false;
                    self.do_delete();
                }
                _ => {
                    self.confirm_delete = false;
                    self.delete_target_url.clear();
                }
            }
            return false;
        }
        if self.edit_mode {
            return self.handle_edit_key(key);
        }
        if self.search_mode {
            return self.handle_search_key(key);
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        // dd 시퀀스 처리
        if self.pending_d {
            self.pending_d = false;
            if key.code == KeyCode::Char('d') && !ctrl {
                self.confirm_delete_selected();
                return false;
            }
            if ctrl && key.code == KeyCode::Char('d') {
                self.page_down();
                return false;
            }
            return false;
        }

        match key.code {
            KeyCode::Char('q') => return true,
            KeyCode::Esc => {
                if !self.search_query.is_empty() {
                    self.search_query.clear();
                    self.table_state.select(Some(0));
                }
            }
            KeyCode::Up | KeyCode::Char('k') => self.scroll_up(),
            KeyCode::Down | KeyCode::Char('j') => self.scroll_down(),
            KeyCode::PageDown => self.page_down(),
            KeyCode::PageUp => self.page_up(),
            KeyCode::Char('d') if ctrl => self.page_down(),
            KeyCode::Char('u') if ctrl => self.page_up(),
            KeyCode::Char('d') => self.pending_d = true,
            KeyCode::Char('g') => self.go_top(),
            KeyCode::Char('G') => self.go_bottom(),
            KeyCode::Char('s') => self.set_sort_mode(SortMode::Status),
            KeyCode::Char('f') => self.set_sort_mode(SortMode::Folder),
            KeyCode::Char('n') => self.set_sort_mode(SortMode::Name),
            KeyCode::Char('u') if !ctrl => self.set_sort_mode(SortMode::Url),
            KeyCode::Char(' ') => self.toggle_select(),
            KeyCode::Char('V') => self.toggle_select_all(),
            KeyCode::Char('o') => self.open_selected_url(),
            KeyCode::Char('e') => self.enter_edit_mode(),
            KeyCode::Char('/') => {
                self.search_mode = true;
                self.search_query.clear();
            }
            KeyCode::Char('r') => self.request_refresh(),
            _ => {}
        }
        false
    }

    fn handle_search_key(&mut self, key: &crossterm::event::KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc => {
                self.search_mode = false;
                self.search_query.clear();
                self.table_state.select(Some(0));
            }
            KeyCode::Enter => {
                self.search_mode = false;
                self.table_state.select(Some(0));
            }
            KeyCode::Backspace => {
                self.search_query.pop();
                self.table_state.select(Some(0));
            }
            KeyCode::Char(c) => {
                self.search_query.push(c);
                self.table_state.select(Some(0));
            }
            _ => {}
        }
        false
    }

    fn edit_buf(&self) -> &str {
        match self.edit_field {
            EditField::Folder => &self.edit_folder,
            EditField::Name => &self.edit_name,
            EditField::Url => &self.edit_url,
        }
    }

    fn edit_buf_mut(&mut self) -> &mut String {
        match self.edit_field {
            EditField::Folder => &mut self.edit_folder,
            EditField::Name => &mut self.edit_name,
            EditField::Url => &mut self.edit_url,
        }
    }

    /// 커서 위치(문자 인덱스)를 바이트 오프셋으로 변환
    fn cursor_byte_offset(&self) -> usize {
        let buf = self.edit_buf();
        buf.char_indices()
            .nth(self.edit_cursor)
            .map(|(i, _)| i)
            .unwrap_or(buf.len())
    }

    fn handle_edit_key(&mut self, key: &crossterm::event::KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc => {
                self.edit_mode = false;
            }
            KeyCode::Tab => {
                self.edit_field = match self.edit_field {
                    EditField::Folder => EditField::Name,
                    EditField::Name => EditField::Url,
                    EditField::Url => EditField::Folder,
                };
            }
            KeyCode::BackTab => {
                self.edit_field = match self.edit_field {
                    EditField::Folder => EditField::Url,
                    EditField::Name => EditField::Folder,
                    EditField::Url => EditField::Name,
                };
                let len = self.edit_buf().chars().count();
                self.edit_cursor = len;
            }
            KeyCode::Enter => {
                self.save_edit();
                self.edit_mode = false;
            }
            KeyCode::Left => {
                self.edit_cursor = self.edit_cursor.saturating_sub(1);
            }
            KeyCode::Right => {
                let len = self.edit_buf().chars().count();
                if self.edit_cursor < len {
                    self.edit_cursor += 1;
                }
            }
            KeyCode::Home => {
                self.edit_cursor = 0;
            }
            KeyCode::End => {
                self.edit_cursor = self.edit_buf().chars().count();
            }
            KeyCode::Backspace => {
                if self.edit_cursor > 0 {
                    let offset = self.cursor_byte_offset();
                    let prev_char_len = self.edit_buf()[..offset]
                        .chars()
                        .last()
                        .map(|c| c.len_utf8())
                        .unwrap_or(0);
                    self.edit_buf_mut()
                        .replace_range((offset - prev_char_len)..offset, "");
                    self.edit_cursor -= 1;
                }
            }
            KeyCode::Delete => {
                let len = self.edit_buf().chars().count();
                if self.edit_cursor < len {
                    let offset = self.cursor_byte_offset();
                    let char_len = self.edit_buf()[offset..]
                        .chars()
                        .next()
                        .map(|c| c.len_utf8())
                        .unwrap_or(0);
                    self.edit_buf_mut()
                        .replace_range(offset..(offset + char_len), "");
                }
            }
            KeyCode::Char(c) => {
                let offset = self.cursor_byte_offset();
                self.edit_buf_mut().insert(offset, c);
                self.edit_cursor += 1;
            }
            _ => {}
        }
        false
    }

    fn enter_edit_mode(&mut self) {
        if is_chrome_running() {
            self.chrome_warning = true;
            return;
        }
        let idx = match self.table_state.selected() {
            Some(i) => i,
            None => return,
        };
        let results = self.sorted_results();
        let entry = results
            .get(idx)
            .map(|r| (r.folder.clone(), r.name.clone(), r.url.clone()));
        if let Some((folder, name, url)) = entry {
            self.edit_cursor = folder.chars().count();
            self.edit_folder = folder;
            self.edit_name = name;
            self.edit_url = url.clone();
            self.edit_original_url = url;
            self.edit_original_folder = self.edit_folder.clone();
            self.edit_field = EditField::Folder;
            self.edit_mode = true;
        }
    }

    fn save_edit(&mut self) {
        let original_url = self.edit_original_url.clone();
        let original_folder = self.edit_original_folder.clone();
        let new_folder = self.edit_folder.clone();
        let new_name = self.edit_name.clone();
        let new_url = self.edit_url.clone();

        if let Some(r) = self.results.iter_mut().find(|r| r.url == original_url) {
            r.folder = new_folder.clone();
            r.name = new_name.clone();
            r.url = new_url.clone();
        }

        let _ = update_bookmarks_file(&self.bookmarks_path, &original_url, &new_name, &new_url);

        if original_folder != new_folder {
            let _ = move_bookmark_to_folder(&self.bookmarks_path, &new_url, &new_folder);
        }

        self.export_html_popup();
    }

    fn toggle_select(&mut self) {
        let idx = match self.table_state.selected() {
            Some(i) => i,
            None => return,
        };
        let results = self.sorted_results();
        if let Some(r) = results.get(idx) {
            let url = r.url.clone();
            if self.selected_urls.contains(&url) {
                self.selected_urls.remove(&url);
            } else {
                self.selected_urls.insert(url);
            }
        }
        self.scroll_down();
    }

    fn toggle_select_all(&mut self) {
        let visible_urls: Vec<String> = self
            .sorted_results()
            .iter()
            .map(|r| r.url.clone())
            .collect();
        let all_selected = visible_urls.iter().all(|u| self.selected_urls.contains(u));
        if all_selected {
            for u in &visible_urls {
                self.selected_urls.remove(u);
            }
        } else {
            for u in visible_urls {
                self.selected_urls.insert(u);
            }
        }
    }

    fn confirm_delete_selected(&mut self) {
        if is_chrome_running() {
            self.chrome_warning = true;
            return;
        }
        if !self.selected_urls.is_empty() {
            self.delete_target_url = format!("{} bookmarks selected", self.selected_urls.len());
            self.confirm_delete = true;
            return;
        }
        let idx = match self.table_state.selected() {
            Some(i) => i,
            None => return,
        };
        let results = self.sorted_results();
        if let Some(r) = results.get(idx) {
            self.delete_target_url = r.url.clone();
            self.confirm_delete = true;
        }
    }

    fn do_delete(&mut self) {
        let target = std::mem::take(&mut self.delete_target_url);
        if target.is_empty() {
            return;
        }
        let idx = self.table_state.selected().unwrap_or(0);

        if !self.selected_urls.is_empty() {
            let urls: Vec<String> = self.selected_urls.drain().collect();
            for url in &urls {
                self.delete_single_entry(url);
            }
            let url_set: HashSet<&String> = urls.iter().collect();
            self.results.retain(|r| !url_set.contains(&r.url));
            self.total = self.total.saturating_sub(urls.len());
        } else {
            self.delete_single_entry(&target);
            self.results.retain(|r| r.url != target);
            self.total = self.total.saturating_sub(1);
        }

        let len = self.row_count();
        if len > 0 {
            if idx >= len {
                self.table_state.select(Some(len - 1));
            }
        } else {
            self.table_state.select(None);
        }

        self.export_html_popup();
    }

    fn delete_single_entry(&self, url: &str) {
        if url.starts_with("folder://") {
            // 빈 폴더 삭제: url = "folder://parent/folder_name"
            if let Some(r) = self.results.iter().find(|r| r.url == url) {
                let _ = delete_empty_folder_from_file(&self.bookmarks_path, &r.folder, &r.name);
            }
        } else {
            let _ = delete_bookmark_from_file(&self.bookmarks_path, url);
        }
    }

    fn export_html_popup(&mut self) {
        match export_bookmarks_html(&self.bookmarks_path) {
            Ok(path) => {
                self.popup_message = Some(format!("Bookmark file exported: {}", path.display()));
            }
            Err(e) => {
                self.popup_message = Some(format!("Export failed: {e}"));
            }
        }
    }

    fn request_refresh(&mut self) {
        self.refresh_requested = true;
    }

    pub fn reset(&mut self) {
        self.results.clear();
        self.checked = 0;
        self.invalid = 0;
        self.checking_done = false;
        self.sort_mode = SortMode::None;
        self.sort_ascending = true;
        self.search_query.clear();
        self.search_mode = false;
        self.refresh_requested = false;
        self.selected_urls.clear();
        self.table_state.select(Some(0));
    }

    fn selected_url(&self) -> Option<String> {
        let idx = self.table_state.selected()?;
        let results = self.sorted_results();
        results.get(idx).map(|r| r.url.clone())
    }

    fn open_selected_url(&self) {
        if let Some(url) = self.selected_url() {
            let _ = open_url(&url);
        }
    }
}

/// hint 텍스트에서 [] 안의 단축키는 LightRed, 나머지는 Gray로 스타일링
fn styled_hint(text: &str) -> Vec<Span<'_>> {
    let mut spans = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('[') {
        if start > 0 {
            spans.push(Span::styled(
                &rest[..start],
                Style::default().fg(Color::Gray),
            ));
        }
        if let Some(end) = rest[start..].find(']') {
            spans.push(Span::styled(
                &rest[start..start + end + 1],
                Style::default().fg(Color::LightRed),
            ));
            rest = &rest[start + end + 1..];
        } else {
            break;
        }
    }
    if !rest.is_empty() {
        spans.push(Span::styled(rest, Style::default().fg(Color::Gray)));
    }
    spans
}

/// 중앙 팝업 영역 계산
fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let popup_width = area.width * percent_x / 100;
    let x = (area.width.saturating_sub(popup_width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, popup_width, height)
}

/// TUI 프로필 선택 화면
pub fn run_profile_selector(profiles: &[ProfileInfo]) -> io::Result<Option<usize>> {
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
                    .bottom_margin(0),
            )
            .block(
                Block::default()
                    .title(format!(
                        " checkbookmark v{} | Chrome Profiles",
                        env!("CARGO_PKG_VERSION")
                    ))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Cyan)),
            )
            .highlight_symbol(">> ");

            f.render_widget(table, chunks[0]);

            let help = Paragraph::new(Line::from(styled_hint(
                " [↑/↓/j/k] Navigate  [Enter] Select  [q] Quit",
            )))
            .block(Block::default().borders(Borders::ALL));
            f.render_widget(help, chunks[1]);
        })?;

        if event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
        {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Char('q') => break None,
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
    };

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, Show)?;
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

    // 상단: 프로필 정보 + 소스 파일 + 설정값 + 진행률
    let settings = format!(
        "Concurrency: {}  Timeout: {}s",
        app.concurrency, app.timeout
    );
    let version = env!("CARGO_PKG_VERSION");
    let source_file = app
        .bookmarks_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let header_spans: Vec<Span> = if app.checking_done {
        let valid = app.total - app.invalid;
        vec![
            Span::raw(format!(
                " checkbookmark v{version}  |  Profile: {}  |  Source: {source_file}  |  Total: {}  ",
                app.profile_name, app.total
            )),
            Span::styled(format!("Valid: {valid}"), Style::default().fg(Color::Green)),
            Span::raw("  "),
            Span::styled(
                format!("Invalid: {}", app.invalid),
                Style::default().fg(Color::Yellow),
            ),
            Span::raw(format!("  |  {settings}")),
        ]
    } else {
        vec![Span::raw(format!(
            " checkbookmark v{version}  |  Profile: {}  |  Source: {source_file}  |  Checking: {}/{}  |  {}",
            app.profile_name, app.checked, app.total, settings
        ))]
    };

    let progress_ratio = if app.total > 0 {
        (app.checked as f64 / app.total as f64).min(1.0)
    } else {
        1.0
    };

    let gauge = Gauge::default()
        .block(
            Block::default()
                .title(Line::from(header_spans))
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
            let is_selected = app.selected_urls.contains(&r.url);
            let is_empty_folder = r.url.starts_with("folder://");
            let status_style = if is_empty_folder {
                Style::default().fg(Color::DarkGray)
            } else if r.is_valid {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::Yellow)
            };
            let row_num = if is_selected {
                format!(" ✓{}", i + 1)
            } else {
                format!("{}", i + 1)
            };
            let display_url = if is_empty_folder {
                "(empty folder)"
            } else {
                r.url.as_str()
            };
            let row = Row::new(vec![
                Cell::from(row_num),
                Cell::from(r.status.as_str()).style(status_style),
                Cell::from(r.folder.as_str()),
                Cell::from(r.name.as_str()),
                Cell::from(display_url),
            ]);
            if is_selected {
                row.style(Style::default().fg(Color::Magenta))
            } else {
                row
            }
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(5),
            Constraint::Length(16),
            Constraint::Percentage(12),
            Constraint::Percentage(25),
            Constraint::Percentage(48),
        ],
    )
    .header({
        let arrow = if app.sort_ascending { " ▲" } else { " ▼" };
        let status_label = if app.sort_mode == SortMode::Status {
            format!("STATUS{arrow}")
        } else {
            "STATUS".to_string()
        };
        let name_label = if app.sort_mode == SortMode::Name {
            format!("NAME{arrow}")
        } else {
            "NAME".to_string()
        };
        let url_label = if app.sort_mode == SortMode::Url {
            format!("URL{arrow}")
        } else {
            "URL".to_string()
        };
        Row::new(vec![
            "#".to_string(),
            status_label,
            if app.sort_mode == SortMode::Folder {
                format!("FOLDER{arrow}")
            } else {
                "FOLDER".to_string()
            },
            name_label,
            url_label,
        ])
        .style(
            Style::default()
                .add_modifier(Modifier::BOLD)
                .fg(Color::Cyan),
        )
        .bottom_margin(0)
    })
    .block(
        Block::default()
            .title(" Results ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan)),
    )
    .row_highlight_style(
        Style::default()
            .bg(Color::Rgb(40, 40, 80))
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol(">> ");

    f.render_stateful_widget(table, chunks[1], &mut app.table_state);

    // 편집 모드: 중앙에 팝업
    if app.edit_mode {
        let popup_area = centered_rect(60, 12, area);
        // 2칸 문자(한글 등) 경계 깨짐 방지를 위해 좌우 1칸 넓게 Clear
        let clear_area = Rect::new(
            popup_area.x.saturating_sub(1),
            popup_area.y,
            (popup_area.width + 2).min(area.width.saturating_sub(popup_area.x.saturating_sub(1))),
            popup_area.height,
        );
        f.render_widget(Clear, clear_area);

        let edit_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Length(3),
            ])
            .split(popup_area);

        let folder_style = if app.edit_field == EditField::Folder {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::White)
        };
        let folder_input = Paragraph::new(app.edit_folder.as_str())
            .style(folder_style)
            .block(
                Block::default()
                    .title(" Folder ")
                    .borders(Borders::ALL)
                    .border_style(folder_style),
            );
        f.render_widget(folder_input, edit_chunks[0]);

        let name_style = if app.edit_field == EditField::Name {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::White)
        };
        let name_input = Paragraph::new(app.edit_name.as_str())
            .style(name_style)
            .block(
                Block::default()
                    .title(" Name ")
                    .borders(Borders::ALL)
                    .border_style(name_style),
            );
        f.render_widget(name_input, edit_chunks[1]);

        let url_style = if app.edit_field == EditField::Url {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::White)
        };
        let url_input = Paragraph::new(app.edit_url.as_str())
            .style(url_style)
            .block(
                Block::default()
                    .title(" URL ")
                    .borders(Borders::ALL)
                    .border_style(url_style),
            );
        f.render_widget(url_input, edit_chunks[2]);

        let hint_spans = styled_hint(" [Tab] Switch field  [Enter] Save  [Esc] Cancel  ");
        let hint = Paragraph::new(Line::from(hint_spans)).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Gray)),
        );
        f.render_widget(hint, edit_chunks[3]);

        // 활성 필드에 커서 표시
        let (cursor_area, buf) = match app.edit_field {
            EditField::Folder => (edit_chunks[0], app.edit_folder.as_str()),
            EditField::Name => (edit_chunks[1], app.edit_name.as_str()),
            EditField::Url => (edit_chunks[2], app.edit_url.as_str()),
        };
        // 커서 위치까지의 표시 너비를 계산
        let prefix: String = buf.chars().take(app.edit_cursor).collect();
        let cursor_x = cursor_area.x + 1 + UnicodeWidthStr::width(prefix.as_str()) as u16;
        let cursor_y = cursor_area.y + 1;
        f.set_cursor_position((cursor_x, cursor_y));
    }

    // 삭제 확인 팝업
    if app.confirm_delete {
        let popup_area = centered_rect(50, 6, area);
        // 2칸 문자(한글 등) 경계 깨짐 방지를 위해 좌우 1칸 넓게 Clear
        let clear_area = Rect::new(
            popup_area.x.saturating_sub(1),
            popup_area.y,
            (popup_area.width + 2).min(area.width.saturating_sub(popup_area.x.saturating_sub(1))),
            popup_area.height,
        );
        f.render_widget(Clear, clear_area);

        let confirm_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Length(3)])
            .split(popup_area);

        let msg = Paragraph::new(format!(" Delete: {}", app.delete_target_url))
            .style(Style::default().fg(Color::Yellow))
            .block(
                Block::default()
                    .title(" Confirm Delete ")
                    .title_style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Red)),
            );
        f.render_widget(msg, confirm_chunks[0]);

        let hint_spans = styled_hint(" [y] Yes  [any other key] Cancel  ");
        let hint = Paragraph::new(Line::from(hint_spans)).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Gray)),
        );
        f.render_widget(hint, confirm_chunks[1]);
    }

    // Chrome 실행 경고 팝업
    if app.chrome_warning {
        let popup_area = centered_rect(55, 5, area);
        let clear_area = Rect::new(
            popup_area.x.saturating_sub(1),
            popup_area.y,
            (popup_area.width + 2).min(area.width.saturating_sub(popup_area.x.saturating_sub(1))),
            popup_area.height,
        );
        f.render_widget(Clear, clear_area);

        let warn_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Length(2)])
            .split(popup_area);

        let msg = Paragraph::new(" Chrome is running. Close Chrome before editing bookmarks.")
            .style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
            .block(
                Block::default()
                    .title(" ⚠ Chrome Running ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Red)),
            );
        f.render_widget(msg, warn_chunks[0]);

        let hint = Paragraph::new(Line::from(styled_hint(" [any key] Close  "))).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Gray)),
        );
        f.render_widget(hint, warn_chunks[1]);
    }

    // 팝업 메시지
    if let Some(msg) = &app.popup_message {
        let popup_area = centered_rect(55, 5, area);
        let clear_area = Rect::new(
            popup_area.x.saturating_sub(1),
            popup_area.y,
            (popup_area.width + 2).min(area.width.saturating_sub(popup_area.x.saturating_sub(1))),
            popup_area.height,
        );
        f.render_widget(Clear, clear_area);

        let msg_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Length(2)])
            .split(popup_area);

        let text = Paragraph::new(format!(" {msg}"))
            .style(
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            )
            .block(
                Block::default()
                    .title(" Export Complete ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Green)),
            );
        f.render_widget(text, msg_chunks[0]);

        let hint = Paragraph::new(Line::from(styled_hint(" [any key] Close  "))).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Gray)),
        );
        f.render_widget(hint, msg_chunks[1]);
    }

    // 하단: 검색 모드 또는 도움말
    if app.search_mode {
        let filtered_count = app.sorted_results().len();
        let match_info = if app.search_query.is_empty() {
            String::new()
        } else {
            format!("  ({filtered_count} matches)")
        };
        let search_text = format!(" /{}{}", app.search_query, match_info);
        let help = Paragraph::new(search_text)
            .style(Style::default().fg(Color::Yellow))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Yellow)),
            );
        f.render_widget(help, chunks[2]);
    } else {
        let search_info = if !app.search_query.is_empty() {
            let filtered_count = app.sorted_results().len();
            format!(
                "  [Filter: \"{}\" {} matches]",
                app.search_query, filtered_count
            )
        } else {
            String::new()
        };
        let select_info = if !app.selected_urls.is_empty() {
            format!("  [Selected: {}]", app.selected_urls.len())
        } else {
            String::new()
        };
        let help_text = format!(
            " [↑/↓/j/k] Navigate  [Space] Select  [V] Select All  [dd] Delete  [s/f/n/u] Sort  [o] Open  [e] Edit  [/] Filter  [r] Refresh  [q] Quit{select_info}{search_info}"
        );
        let help = Paragraph::new(Line::from(styled_hint(&help_text)))
            .block(Block::default().borders(Borders::ALL));
        f.render_widget(help, chunks[2]);
    }
}

/// 백그라운드 URL 검사 태스크 생성
fn spawn_check_task(
    entries: Vec<BookmarkEntry>,
    app: Arc<Mutex<App>>,
    client: Arc<reqwest::Client>,
    checked_count: Arc<AtomicUsize>,
    concurrency: usize,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        stream::iter(entries)
            .map(|entry| {
                let client = client.clone();
                let app = app.clone();
                let checked = checked_count.clone();
                async move {
                    let (status_text, is_valid) = if entry.is_empty_folder {
                        ("EMPTY_FOLDER".to_string(), false)
                    } else {
                        let (status_code, text) = check_url(&client, &entry.url).await;
                        (text, (200..400).contains(&status_code))
                    };
                    let done = checked.fetch_add(1, Ordering::Relaxed) + 1;

                    let result = CheckResult {
                        folder: entry.folder,
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
    })
}

/// 검사 중 실시간 TUI 업데이트 루프
pub async fn run_check_tui(
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

    let mut check_handle = spawn_check_task(
        entries.clone(),
        app.clone(),
        client.clone(),
        checked_count.clone(),
        concurrency,
    );

    // TUI 렌더링 + 이벤트 루프
    'outer: loop {
        // refresh 요청 확인
        {
            let mut locked = app.lock().unwrap();
            if locked.refresh_requested {
                locked.bookmarks_path = resolve_bookmarks_path(&locked.bookmarks_path);
                let fresh_entries = parse_bookmarks(&locked.bookmarks_path);
                locked.total = fresh_entries.len();
                locked.reset();
                drop(locked);
                check_handle.abort();
                checked_count.store(0, Ordering::Relaxed);
                check_handle = spawn_check_task(
                    fresh_entries,
                    app.clone(),
                    client.clone(),
                    checked_count.clone(),
                    concurrency,
                );
                continue;
            }
        }

        {
            let mut app = app.lock().unwrap();
            terminal.draw(|f| render_app(f, &mut app))?;
        }

        if event::poll(Duration::from_millis(50))?
            && let Event::Key(key) = event::read()?
        {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            let mut app = app.lock().unwrap();
            if app.handle_key(&key) {
                break 'outer;
            }
        }

        let done = checked_count.load(Ordering::Relaxed);
        if done >= total && !app.lock().unwrap().refresh_requested {
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
                if event::poll(Duration::from_millis(100))?
                    && let Event::Key(key) = event::read()?
                {
                    if key.kind != KeyEventKind::Press {
                        continue;
                    }
                    let mut locked = app.lock().unwrap();
                    if locked.handle_key(&key) {
                        break 'outer;
                    }
                    if locked.refresh_requested {
                        break; // 내부 루프 탈출 → 외부 루프에서 refresh 처리
                    }
                }
            }
        }
    }

    check_handle.abort();
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, Show)?;

    let app = app.lock().unwrap();
    if app.invalid > 0 {
        std::process::exit(1);
    }

    Ok(())
}
