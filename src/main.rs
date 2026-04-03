mod bookmark;
mod tui;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use clap::Parser;

use bookmark::{discover_profiles, parse_bookmarks};
use tui::{App, run_check_tui, run_profile_selector};

/// CLI 인자 정의
#[derive(Parser)]
#[command(name = "cbm", about = "Check Chrome bookmarks URL validity")]
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
            path.clone(),
        )));
        if run_check_tui(app, entries, client.clone(), cli.concurrency)
            .await
            .is_err()
        {
            eprintln!("ERROR: TUI failed");
        }
    }
}
