use std::io;
use std::path::PathBuf;

use serde::Deserialize;

/// Chrome 북마크 JSON 최상위 구조
#[derive(Deserialize)]
pub struct Bookmarks {
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
#[derive(Clone)]
pub struct BookmarkEntry {
    pub folder: String,
    pub name: String,
    pub url: String,
}

/// URL 검사 결과
#[derive(Debug, Clone)]
pub struct CheckResult {
    pub folder: String,
    pub name: String,
    pub url: String,
    pub status: String,
    pub is_valid: bool,
}

/// 탐색된 Chrome 프로필 정보
pub struct ProfileInfo {
    pub dir_name: String,
    pub display_name: String,
    pub bookmarks_path: PathBuf,
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
                folder: folder_path.to_string(),
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
pub fn discover_profiles() -> Vec<ProfileInfo> {
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

/// 북마크 파일을 읽어 URL 항목 목록으로 파싱
pub fn parse_bookmarks(path: &PathBuf) -> Vec<BookmarkEntry> {
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

/// URL에 HTTP HEAD 요청을 보내 상태 코드와 상태 텍스트 반환
pub async fn check_url(client: &reqwest::Client, url: &str) -> (u16, String) {
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

/// Bookmarks JSON 파일에서 URL로 항목을 찾아 name/url 업데이트
pub fn update_bookmarks_file(
    path: &PathBuf,
    original_url: &str,
    new_name: &str,
    new_url: &str,
) -> io::Result<()> {
    let content = std::fs::read_to_string(path)?;
    let mut value: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    fn update_node(
        node: &mut serde_json::Value,
        original_url: &str,
        new_name: &str,
        new_url: &str,
    ) -> bool {
        if node.get("type").and_then(|t| t.as_str()) == Some("url")
            && node.get("url").and_then(|u| u.as_str()) == Some(original_url)
        {
            node["name"] = serde_json::Value::String(new_name.to_string());
            node["url"] = serde_json::Value::String(new_url.to_string());
            return true;
        }
        if let Some(children) = node.get_mut("children").and_then(|c| c.as_array_mut()) {
            for child in children {
                if update_node(child, original_url, new_name, new_url) {
                    return true;
                }
            }
        }
        false
    }

    if let Some(roots) = value.get_mut("roots") {
        for key in &["bookmark_bar", "other", "synced"] {
            if let Some(root) = roots.get_mut(*key) {
                if update_node(root, original_url, new_name, new_url) {
                    break;
                }
            }
        }
    }

    let output = serde_json::to_string_pretty(&value)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    std::fs::write(path, output)?;
    Ok(())
}

/// Bookmarks JSON 파일에서 URL로 항목을 찾아 삭제
pub fn delete_bookmark_from_file(path: &PathBuf, target_url: &str) -> io::Result<()> {
    let content = std::fs::read_to_string(path)?;
    let mut value: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    fn remove_node(node: &mut serde_json::Value, target_url: &str) -> bool {
        if let Some(children) = node.get_mut("children").and_then(|c| c.as_array_mut()) {
            let before = children.len();
            children.retain(|child| {
                !(child.get("type").and_then(|t| t.as_str()) == Some("url")
                    && child.get("url").and_then(|u| u.as_str()) == Some(target_url))
            });
            if children.len() < before {
                return true;
            }
            for child in children {
                if remove_node(child, target_url) {
                    return true;
                }
            }
        }
        false
    }

    if let Some(roots) = value.get_mut("roots") {
        for key in &["bookmark_bar", "other", "synced"] {
            if let Some(root) = roots.get_mut(*key) {
                if remove_node(root, target_url) {
                    break;
                }
            }
        }
    }

    let output = serde_json::to_string_pretty(&value)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    std::fs::write(path, output)?;
    Ok(())
}

/// OS별 기본 브라우저로 URL 열기
pub fn open_url(url: &str) -> io::Result<()> {
    if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(url).spawn()?;
    } else if cfg!(target_os = "linux") {
        std::process::Command::new("xdg-open").arg(url).spawn()?;
    } else {
        std::process::Command::new("cmd")
            .args(["/C", "start", url])
            .spawn()?;
    }
    Ok(())
}
