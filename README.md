# bookmark-check

Chrome 북마크 URL 유효성 검사 TUI 도구.

Chrome 북마크 파일을 읽어 각 URL에 HTTP HEAD 요청을
동시에 보내고, 살아있는 북마크와 죽은 북마크를 리포트한다.

## 기능

- Chrome 프로필 자동 탐색 및 TUI 선택
- 동시 요청 수 설정 가능한 병렬 URL 검사
- 실시간 진행률 표시
- 결과 테이블 (valid: 초록, invalid: 빨강)
- status 기준 정렬 토글
- vim 스타일 키보드 탐색
- macOS, Linux, Windows 지원

## 설치

```bash
cargo install --path .
```

## 사용법

```bash
# TUI 프로필 선택 후 검사
bookmark-check

# 북마크 파일 직접 지정
bookmark-check -f /path/to/Bookmarks

# 동시 요청 수, 타임아웃 설정
bookmark-check -c 20 -t 5
```

## 옵션

| 옵션 | 축약 | 기본값 | 설명 |
|---|---|---|---|
| `--file <FILE>` | `-f` | 자동 탐색 | 북마크 파일 경로 |
| `--concurrency <N>` | `-c` | 10 | 동시 요청 수 |
| `--timeout <SECS>` | `-t` | 3 | 요청 타임아웃(초) |

## 키 바인딩

| 키 | 동작 |
|---|---|
| `↑` / `k` | 위로 이동 |
| `↓` / `j` | 아래로 이동 |
| `PgUp` / `Ctrl+u` | 페이지 위로 |
| `PgDn` / `Ctrl+d` | 페이지 아래로 |
| `g` | 맨 위로 |
| `G` | 맨 아래로 |
| `s` | status 기준 정렬 토글 |
| `q` / `Esc` | 종료 |

## 종료 코드

- `0`: 모든 URL이 유효
- `1`: 무효한 URL이 1개 이상 존재

## Chrome 북마크 경로

| OS | 경로 |
|---|---|
| macOS | `~/Library/Application Support/Google/Chrome/{Profile}/Bookmarks` |
| Linux | `~/.config/google-chrome/{Profile}/Bookmarks` |
| Windows | `%LOCALAPPDATA%\Google\Chrome\User Data\{Profile}\Bookmarks` |
