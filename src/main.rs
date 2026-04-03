use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use clap::Parser;
use colored::Colorize;
use futures::stream::{self, StreamExt};
use serde::Deserialize;
use unicode_width::UnicodeWidthStr;

/// CLI 인자 정의
#[derive(Parser)]
#[command(name = "bookmark-check", about = "Check Chrome bookmarks URL validity")]
struct Cli {
    /// Chrome 북마크 파일 경로
    #[arg(short, long)]
    file: Option<PathBuf>,

    /// Chrome 프로필 이름 (예: "Default", "Profile 2")
    #[arg(short, long)]
    profile: Option<String>,

    /// 모든 프로필 검사
    #[arg(short, long)]
    all: bool,

    /// 동시 요청 수
    #[arg(short, long, default_value_t = 10)]
    concurrency: usize,

    /// 요청 타임아웃(초)
    #[arg(short, long, default_value_t = 3)]
    timeout: u64,

    /// 유효하지 않은 URL만 표시
    #[arg(long)]
    invalid_only: bool,

    /// URL 검사 없이 북마크 목록만 표시
    #[arg(short, long)]
    list: bool,
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
#[derive(Debug)]
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

    // type이 "url"인 노드에서 URL 추출
    if node.node_type == "url" {
        if let Some(url) = &node.url {
            entries.push(BookmarkEntry {
                name: node.name.clone(),
                url: url.clone(),
            });
        }
    }

    // 하위 노드가 있으면 재귀 탐색
    if let Some(children) = &node.children {
        for child in children {
            collect_urls(child, &current_path, entries);
        }
    }
}

/// OS별 Chrome 데이터 기본 디렉토리 반환
fn chrome_base_dir() -> PathBuf {
    let home = dirs::home_dir().expect("Failed to get home directory");
    // cfg! 매크로: 컴파일 타임에 대상 OS를 확인하여 true/false 반환
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Google/Chrome")
    } else if cfg!(target_os = "linux") {
        home.join(".config/google-chrome")
    } else {
        // Windows
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

        // Bookmarks 파일이 있는 디렉토리만 프로필로 인식
        let bookmarks_path = path.join("Bookmarks");
        if !bookmarks_path.exists() {
            continue;
        }

        let dir_name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        // Preferences 파일에서 사용자가 설정한 프로필 이름 읽기
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

/// PathBuf에 파일 읽기 헬퍼 메서드 추가
trait PipeRead {
    fn pipe_read(&self) -> Option<String>;
}

impl PipeRead for PathBuf {
    fn pipe_read(&self) -> Option<String> {
        std::fs::read_to_string(self).ok()
    }
}

/// 프로필 목록을 출력하고 사용자에게 번호로 선택받음
fn select_profile(profiles: &[ProfileInfo]) -> usize {
    println!("{}", "Available Chrome profiles:".cyan().bold());
    for (i, p) in profiles.iter().enumerate() {
        println!(
            "  {}) {} ({})",
            i + 1,
            p.display_name.bold(),
            p.dir_name.dimmed()
        );
    }
    print!("\n{} ", "Select profile [1]:".cyan().bold());
    io::stdout().flush().unwrap();

    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    let input = input.trim();

    // 빈 입력이면 첫 번째 프로필 선택
    if input.is_empty() {
        return 0;
    }

    match input.parse::<usize>() {
        Ok(n) if n >= 1 && n <= profiles.len() => n - 1,
        _ => {
            eprintln!("{} Invalid selection", "ERROR:".red().bold());
            std::process::exit(1);
        }
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
                // 에러 메시지가 길면 20자로 잘라서 표시
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
        eprintln!("{} Failed to read file: {e}", "ERROR:".red().bold());
        std::process::exit(1);
    });

    let bookmarks: Bookmarks = serde_json::from_str(&content).unwrap_or_else(|e| {
        eprintln!("{} Failed to parse bookmarks: {e}", "ERROR:".red().bold());
        std::process::exit(1);
    });

    let mut entries = Vec::new();
    collect_urls(&bookmarks.roots.bookmark_bar, "", &mut entries);
    collect_urls(&bookmarks.roots.other, "", &mut entries);
    collect_urls(&bookmarks.roots.synced, "", &mut entries);
    entries
}

/// 문자열을 표시 너비 기준으로 자르고 초과 시 "..." 추가
fn truncate(s: &str, max: usize) -> String {
    let width = UnicodeWidthStr::width(s);
    if width <= max {
        s.to_string()
    } else {
        let mut w = 0;
        let truncated: String = s
            .chars()
            .take_while(|c| {
                w += unicode_width::UnicodeWidthChar::width(*c).unwrap_or(0);
                w <= max - 3
            })
            .collect();
        format!("{truncated}...")
    }
}

/// 문자열을 표시 너비 기준으로 고정 폭 패딩 적용
fn pad_width(s: &str, width: usize) -> String {
    let display_w = UnicodeWidthStr::width(s);
    if display_w >= width {
        s.to_string()
    } else {
        format!("{}{}", s, " ".repeat(width - display_w))
    }
}

/// 북마크 목록만 테이블 형식으로 출력
fn print_list(entries: &[BookmarkEntry]) {
    if entries.is_empty() {
        println!("  No bookmarks found.");
        return;
    }

    let num_w = entries.len().to_string().len().max(1);
    let name_w = 40;

    println!(
        "  {:<num_w$}  {:<name_w$}  {}",
        "#".bold(),
        "NAME".bold(),
        "URL".bold(),
    );
    println!("  {}", "-".repeat(num_w + 2 + name_w + 2 + 60));

    for (i, entry) in entries.iter().enumerate() {
        let idx = format!("{}", i + 1);
        let name = truncate(&entry.name, name_w);
        let name_padded = pad_width(&name, name_w);
        println!(
            "  {:<num_w$}  {}  {}",
            idx.dimmed(),
            name_padded,
            entry.url.dimmed(),
        );
    }
}

/// 검사 결과를 테이블 형식으로 출력
fn print_table(results: &[CheckResult], invalid_only: bool) {
    let results: Vec<&CheckResult> = if invalid_only {
        results.iter().filter(|r| !r.is_valid).collect()
    } else {
        results.iter().collect()
    };

    if results.is_empty() {
        println!("  No results to display.");
        return;
    }

    let num_w = results.len().to_string().len().max(1);
    let status_w = 16;
    let name_w = 40;

    // 테이블 헤더
    println!(
        "  {:<num_w$}  {:<status_w$}  {:<name_w$}  {}",
        "#".bold(),
        "STATUS".bold(),
        "NAME".bold(),
        "URL".bold(),
    );
    println!(
        "  {}",
        "-".repeat(num_w + 2 + status_w + 2 + name_w + 2 + 60)
    );

    // 테이블 본문: 유효하면 초록, 무효하면 빨간색
    for (i, r) in results.iter().enumerate() {
        let idx = format!("{}", i + 1);
        let status = if r.is_valid {
            format!("{:<status_w$}", r.status).green().to_string()
        } else {
            format!("{:<status_w$}", r.status).red().to_string()
        };
        let name = truncate(&r.name, name_w);
        let name_padded = pad_width(&name, name_w);
        let url = &r.url;

        println!(
            "  {:<num_w$}  {}  {}  {}",
            idx.dimmed(),
            status,
            name_padded,
            url.dimmed(),
        );
    }
}

/// 모든 URL을 병렬로 검사하고 결과 목록 반환
async fn check_entries(
    entries: Vec<BookmarkEntry>,
    client: Arc<reqwest::Client>,
    concurrency: usize,
) -> Vec<CheckResult> {
    let total = entries.len();
    let checked = Arc::new(AtomicUsize::new(0));
    let results = Arc::new(Mutex::new(Vec::with_capacity(total)));

    stream::iter(entries)
        .map(|entry| {
            let client = client.clone();
            let checked = checked.clone();
            let results = results.clone();
            async move {
                let (status_code, status_text) = check_url(&client, &entry.url).await;
                let done = checked.fetch_add(1, Ordering::Relaxed) + 1;
                let is_valid = (200..400).contains(&status_code);

                // 진행 상황을 같은 줄에 덮어쓰며 표시
                eprint!("\r  Checking... {done}/{total}");

                results.lock().unwrap().push(CheckResult {
                    name: entry.name,
                    url: entry.url,
                    status: status_text,
                    is_valid,
                });
            }
        })
        .buffer_unordered(concurrency)
        .collect::<Vec<()>>()
        .await;

    // 진행 표시 줄 지우기
    eprint!("\r{}\r", " ".repeat(40));

    Arc::try_unwrap(results).unwrap().into_inner().unwrap()
}

/// 프로그레스 바와 함께 최종 요약 출력
fn print_summary(total: usize, invalid: usize) {
    let valid = total - invalid;
    let bar_width = 30;
    let valid_bars = if total > 0 {
        (valid as f64 / total as f64 * bar_width as f64).round() as usize
    } else {
        0
    };
    let invalid_bars = bar_width - valid_bars;

    // 초록(valid) + 빨강(invalid) 프로그레스 바
    let bar = format!(
        "{}{}",
        "#".repeat(valid_bars).green(),
        "#".repeat(invalid_bars).red()
    );

    println!("  [{bar}] {valid}/{total} valid");
    println!();
    println!("  {}  {:>5}", "Total".bold(), total.to_string().bold());
    println!(
        "  {}  {:>5}",
        "Valid".green().bold(),
        valid.to_string().green()
    );
    println!(
        "  {} {:>5}",
        "Invalid".red().bold(),
        invalid.to_string().red()
    );
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

    // 북마크 파일 경로 결정: 직접 지정 > 자동 탐색
    let bookmark_files: Vec<(String, PathBuf)> = if let Some(file) = cli.file {
        vec![("custom".to_string(), file)]
    } else {
        let profiles = discover_profiles();
        if profiles.is_empty() {
            eprintln!("{} No Chrome profiles found", "ERROR:".red().bold());
            std::process::exit(1);
        }

        if cli.all {
            // --all: 모든 프로필 검사
            profiles
                .into_iter()
                .map(|p| (p.display_name, p.bookmarks_path))
                .collect()
        } else if let Some(profile_name) = cli.profile {
            // --profile: 이름으로 프로필 검색
            let found = profiles
                .into_iter()
                .find(|p| p.dir_name == profile_name || p.display_name == profile_name);
            match found {
                Some(p) => vec![(p.display_name, p.bookmarks_path)],
                None => {
                    eprintln!(
                        "{} Profile '{}' not found",
                        "ERROR:".red().bold(),
                        profile_name
                    );
                    std::process::exit(1);
                }
            }
        } else {
            // 옵션 없음: 대화형 프로필 선택
            let idx = select_profile(&profiles);
            let p = &profiles[idx];
            vec![(p.display_name.clone(), p.bookmarks_path.clone())]
        }
    };

    let mut total_all = 0;
    let mut invalid_all = 0;

    // 각 프로필의 북마크 파일을 순서대로 검사
    for (profile_name, path) in &bookmark_files {
        if !path.exists() {
            eprintln!(
                "{} Bookmarks file not found: {}",
                "ERROR:".red().bold(),
                path.display()
            );
            continue;
        }

        println!();
        println!(
            "  {} {} ({})",
            "Profile:".cyan().bold(),
            profile_name.bold(),
            path.display().to_string().dimmed()
        );
        println!();

        let entries = parse_bookmarks(path);
        let total = entries.len();

        if cli.list {
            print_list(&entries);
            println!();
            total_all += total;
            continue;
        }

        let results = check_entries(entries, client.clone(), cli.concurrency).await;
        let invalid = results.iter().filter(|r| !r.is_valid).count();

        print_table(&results, cli.invalid_only);
        println!();

        total_all += total;
        invalid_all += invalid;
    }

    // 전체 요약 출력
    println!("{}", "=".repeat(70));
    println!();
    if cli.list {
        println!("  {}  {}", "Total".bold(), total_all.to_string().bold());
        println!();
    } else {
        print_summary(total_all, invalid_all);
        println!();

        // 무효한 URL이 있으면 exit code 1
        if invalid_all > 0 {
            std::process::exit(1);
        }
    }
}
