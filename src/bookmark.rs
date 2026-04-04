use std::io;
use std::path::PathBuf;

use md5::{Digest, Md5};
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
    pub is_empty_folder: bool,
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

/// 북마크 노드를 재귀적으로 탐색하여 URL 항목과 빈 폴더를 수집
fn collect_entries(node: &BookmarkNode, folder_path: &str, entries: &mut Vec<BookmarkEntry>) {
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
                is_empty_folder: false,
            });
        }
    }

    if let Some(children) = &node.children {
        if node.node_type == "folder" && children.is_empty() {
            entries.push(BookmarkEntry {
                folder: folder_path.to_string(),
                name: node.name.clone(),
                url: format!("folder://{current_path}"),
                is_empty_folder: true,
            });
        }
        for child in children {
            collect_entries(child, &current_path, entries);
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
    collect_entries(&bookmarks.roots.bookmark_bar, "", &mut entries);
    collect_entries(&bookmarks.roots.other, "", &mut entries);
    collect_entries(&bookmarks.roots.synced, "", &mut entries);
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

/// Chrome 북마크 checksum 계산 (Chromium bookmark_codec.cc 호환)
/// 각 노드: id(UTF-8) + name(UTF-16LE) + type(UTF-8) + url(UTF-8, url 노드만)
fn hash_node(hasher: &mut Md5, node: &serde_json::Value) {
    // id: UTF-8
    if let Some(id) = node.get("id").and_then(|v| v.as_str()) {
        hasher.update(id.as_bytes());
    }
    // name/title: UTF-16LE raw bytes
    if let Some(name) = node.get("name").and_then(|v| v.as_str()) {
        for ch in name.encode_utf16() {
            hasher.update(ch.to_le_bytes());
        }
    }
    // type: UTF-8
    let node_type = node.get("type").and_then(|v| v.as_str()).unwrap_or("");
    hasher.update(node_type.as_bytes());
    match node_type {
        "url" => {
            // url: UTF-8
            if let Some(url) = node.get("url").and_then(|v| v.as_str()) {
                hasher.update(url.as_bytes());
            }
        }
        "folder" => {
            if let Some(children) = node.get("children").and_then(|v| v.as_array()) {
                for child in children {
                    hash_node(hasher, child);
                }
            }
        }
        _ => {}
    }
}

/// roots 객체로부터 Chrome 호환 checksum 문자열 계산
fn compute_checksum(roots: &serde_json::Value) -> String {
    let mut hasher = Md5::new();
    for key in &["bookmark_bar", "other", "synced"] {
        if let Some(root) = roots.get(*key) {
            hash_node(&mut hasher, root);
        }
    }
    format!("{:x}", hasher.finalize())
}

/// JSON 값에 checksum을 업데이트하고 파일에 저장
fn save_bookmarks(path: &PathBuf, value: &mut serde_json::Value) -> io::Result<()> {
    if let Some(roots) = value.get("roots") {
        let checksum = compute_checksum(roots);
        value["checksum"] = serde_json::Value::String(checksum);
    }
    let output = serde_json::to_string_pretty(value)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    std::fs::write(path, output)?;
    // Chrome이 .bak에서 복원하지 않도록 삭제
    let bak = path.with_extension("bak");
    if bak.exists() {
        let _ = std::fs::remove_file(&bak);
    }
    Ok(())
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

    save_bookmarks(path, &mut value)
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

    save_bookmarks(path, &mut value)
}

/// Bookmarks JSON 파일에서 빈 폴더를 찾아 삭제
pub fn delete_empty_folder_from_file(
    path: &PathBuf,
    parent_folder: &str,
    folder_name: &str,
) -> io::Result<()> {
    let content = std::fs::read_to_string(path)?;
    let mut value: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    /// 폴더 경로를 따라 내려가서 빈 폴더 제거
    fn navigate_and_remove(
        node: &mut serde_json::Value,
        path_parts: &[&str],
        target_name: &str,
    ) -> bool {
        if path_parts.is_empty() {
            if let Some(children) = node.get_mut("children").and_then(|c| c.as_array_mut()) {
                let before = children.len();
                children.retain(|child| {
                    !(child.get("type").and_then(|t| t.as_str()) == Some("folder")
                        && child.get("name").and_then(|n| n.as_str()) == Some(target_name)
                        && child
                            .get("children")
                            .and_then(|c| c.as_array())
                            .is_some_and(|c| c.is_empty()))
                });
                return children.len() < before;
            }
            return false;
        }
        if let Some(children) = node.get_mut("children").and_then(|c| c.as_array_mut()) {
            for child in children.iter_mut() {
                if child.get("type").and_then(|t| t.as_str()) == Some("folder")
                    && child.get("name").and_then(|n| n.as_str()) == Some(path_parts[0])
                    && navigate_and_remove(child, &path_parts[1..], target_name)
                {
                    return true;
                }
            }
        }
        false
    }

    // parent_folder: "Bookmarks Bar/sub" → ["Bookmarks Bar", "sub"]
    let parts: Vec<&str> = parent_folder.split('/').filter(|s| !s.is_empty()).collect();

    if let Some(roots) = value.get_mut("roots") {
        for key in &["bookmark_bar", "other", "synced"] {
            if let Some(root) = roots.get_mut(*key) {
                let root_name = root
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or_default();
                if !parts.is_empty() && parts[0] == root_name {
                    if navigate_and_remove(root, &parts[1..], folder_name) {
                        break;
                    }
                } else if parts.is_empty() && navigate_and_remove(root, &[], folder_name) {
                    break;
                }
            }
        }
    }

    save_bookmarks(path, &mut value)
}

/// Bookmarks JSON에서 URL 항목을 다른 폴더로 이동
pub fn move_bookmark_to_folder(
    path: &PathBuf,
    target_url: &str,
    new_folder: &str,
) -> io::Result<()> {
    let content = std::fs::read_to_string(path)?;
    let mut value: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    // 1단계: 대상 노드를 찾아서 제거하고 복사본 보관
    fn remove_and_capture(
        node: &mut serde_json::Value,
        target_url: &str,
    ) -> Option<serde_json::Value> {
        if let Some(children) = node.get_mut("children").and_then(|c| c.as_array_mut()) {
            let pos = children.iter().position(|child| {
                child.get("type").and_then(|t| t.as_str()) == Some("url")
                    && child.get("url").and_then(|u| u.as_str()) == Some(target_url)
            });
            if let Some(idx) = pos {
                return Some(children.remove(idx));
            }
            for child in children.iter_mut() {
                if let Some(captured) = remove_and_capture(child, target_url) {
                    return Some(captured);
                }
            }
        }
        None
    }

    // 2단계: 폴더 경로로 대상 폴더 노드를 찾거나 생성
    fn find_or_create_folder<'a>(
        node: &'a mut serde_json::Value,
        folder_parts: &[&str],
    ) -> &'a mut serde_json::Value {
        if folder_parts.is_empty() {
            return node;
        }
        let target_name = folder_parts[0];
        let rest = &folder_parts[1..];

        let children = node
            .get_mut("children")
            .and_then(|c| c.as_array_mut())
            .expect("folder node must have children");

        // 이름이 일치하는 폴더 찾기
        let pos = children.iter().position(|child| {
            child.get("type").and_then(|t| t.as_str()) == Some("folder")
                && child.get("name").and_then(|n| n.as_str()) == Some(target_name)
        });

        let idx = if let Some(i) = pos {
            i
        } else {
            // 폴더가 없으면 새로 생성
            let new_folder = serde_json::json!({
                "children": [],
                "name": target_name,
                "type": "folder"
            });
            children.push(new_folder);
            children.len() - 1
        };

        find_or_create_folder(&mut children[idx], rest)
    }

    // 루트에서 대상 노드 제거
    let mut captured = None;
    if let Some(roots) = value.get_mut("roots") {
        for key in &["bookmark_bar", "other", "synced"] {
            if let Some(root) = roots.get_mut(*key) {
                captured = remove_and_capture(root, target_url);
                if captured.is_some() {
                    break;
                }
            }
        }
    }

    let Some(bookmark_node) = captured else {
        return Ok(());
    };

    // 새 폴더 경로 파싱: "root_name/sub1/sub2" → root_name에서 시작
    let parts: Vec<&str> = new_folder.split('/').filter(|s| !s.is_empty()).collect();
    if parts.is_empty() {
        return Ok(());
    }

    // 첫 번째 파트가 루트 키인지 확인
    let root_keys = ["bookmark_bar", "other", "synced"];
    if let Some(roots) = value.get_mut("roots") {
        let (root_node, remaining) = if root_keys.contains(&parts[0]) {
            (roots.get_mut(parts[0]).unwrap(), &parts[1..])
        } else {
            // 루트 키가 아니면 루트 이름으로 매칭 시도
            let found = root_keys.iter().find(|&&key| {
                roots
                    .get(key)
                    .and_then(|r| r.get("name"))
                    .and_then(|n| n.as_str())
                    == Some(parts[0])
            });
            if let Some(&key) = found {
                (roots.get_mut(key).unwrap(), &parts[1..])
            } else {
                // 기본적으로 bookmark_bar에 넣기
                (roots.get_mut("bookmark_bar").unwrap(), &parts[..])
            }
        };

        let target_folder = find_or_create_folder(root_node, remaining);
        if let Some(children) = target_folder
            .get_mut("children")
            .and_then(|c| c.as_array_mut())
        {
            children.push(bookmark_node);
        }
    }

    save_bookmarks(path, &mut value)
}

/// Chrome 프로세스 실행 여부 확인
pub fn is_chrome_running() -> bool {
    let output = if cfg!(target_os = "macos") {
        std::process::Command::new("pgrep")
            .args(["-x", "Google Chrome"])
            .output()
    } else if cfg!(target_os = "linux") {
        std::process::Command::new("pgrep")
            .args(["-x", "chrome"])
            .output()
    } else {
        std::process::Command::new("tasklist")
            .args(["/FI", "IMAGENAME eq chrome.exe", "/NH"])
            .output()
    };
    match output {
        Ok(o) => {
            if cfg!(target_os = "windows") {
                String::from_utf8_lossy(&o.stdout).contains("chrome.exe")
            } else {
                o.status.success()
            }
        }
        Err(_) => false,
    }
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
