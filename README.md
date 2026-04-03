# cbm

Chrome 북마크 URL 유효성 검사 TUI 도구.

Chrome 북마크 파일을 읽어 각 URL에 HTTP HEAD 요청을 동시에 보내고,
살아있는 북마크와 죽은 북마크를 리포트하고 편집한다.

## 기능

- Chrome 프로필 자동 탐색 및 TUI 선택
- 동시 요청 수 설정 가능한 병렬 URL 검사
- 실시간 진행률 표시
- 결과 테이블 (valid: 초록, invalid: 노랑)
- 폴더 경로 표시
- status/folder/name/url 기준 정렬 (오름차순/내림차순 토글)
- 실시간 검색 필터
- 북마크 편집/삭제 (Chrome Bookmarks 파일에 직접 반영)
- 선택 URL 브라우저 열기
- 전체 재검사 (refresh)
- vim 스타일 키보드 탐색
- macOS, Linux, Windows 지원

## 설치

```bash
cargo install --path .
```

## 사용법

```bash
# TUI 프로필 선택 후 검사
cbm

# 북마크 파일 직접 지정
cbm -f /path/to/Bookmarks

# 동시 요청 수, 타임아웃 설정
cbm -c 20 -t 5
```

## 옵션

| 옵션 | 축약 | 기본값 | 설명 |
|---|---|---|---|
| `--file <FILE>` | `-f` | 자동 탐색 | 북마크 파일 경로 |
| `--concurrency <N>` | `-c` | 100 | 동시 요청 수 |
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
| `s` | status 정렬 (반복: 오름/내림차순) |
| `f` | folder 정렬 (반복: 오름/내림차순) |
| `n` | name 정렬 (반복: 오름/내림차순) |
| `u` | url 정렬 (반복: 오름/내림차순) |
| `/` | 검색 필터 (name/url/status/folder) |
| `Esc` | 검색 필터 해제 / 팝업 취소 |
| `o` | 선택 URL 브라우저 열기 |
| `e` | 선택 항목 편집 |
| `dd` | 선택 항목 삭제 (확인 팝업) |
| `r` | 전체 재검사 |
| `q` | 종료 |

## Chrome 북마크 경로

| OS | 경로 |
|---|---|
| macOS | `~/Library/Application Support/Google/Chrome/{Profile}/Bookmarks` |
| Linux | `~/.config/google-chrome/{Profile}/Bookmarks` |
| Windows | `%LOCALAPPDATA%\Google\Chrome\User Data\{Profile}\Bookmarks` |
