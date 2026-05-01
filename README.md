# checkbookmark

Chrome 북마크 URL 유효성 검사 TUI 도구.

Chrome 북마크 파일을 읽어 각 URL에 HTTP HEAD 요청을 동시에 보내고,
살아있는 북마크와 죽은 북마크를 리포트하고 편집한다.

![screenshot](screenshot.png)

## 설치

```bash
cargo install --path .
```

## 사용법

```bash
# TUI 프로필 선택 후 검사
checkbookmark

# 북마크 파일 직접 지정
checkbookmark -f /path/to/Bookmarks

# 동시 요청 수, 타임아웃 설정
checkbookmark -c 20 -t 5
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
| `Space` | 현재 행 선택/해제 |
| `V` | 보이는 행 전체 선택/해제 |
| `o` | 선택 URL 브라우저 열기 |
| `e` | 선택 항목 편집 |
| `Ctrl+s` | 편집 팝업에서 저장 (팝업 유지) |
| `Enter` | 편집 팝업에서 저장 후 닫기 |
| `dd` | 선택 항목 삭제 stage (확인 팝업, 다중 선택 지원) |
| `:w` | staged 삭제 저장 + HTML export |
| `r` | 전체 재검사 |
| `q` | 종료 |

## 주의사항

Chrome이 실행 중이거나 재시작하면 북마크 파일을
직접 수정하더라도 sync에 의해 자동으로 원복된다.
`:w` 저장 후 생성되는 HTML 내보내기 파일을
Chrome의 `chrome://bookmarks` > ⋮ > **북마크 가져오기**로
import 해야 변경 사항이 반영된다.

## Chrome 북마크 경로

AccountBookmarks → Bookmarks → Bookmarks.bak 순서로 유효한 파일을 탐색한다.

| OS | 경로 |
|---|---|
| macOS | `~/Library/Application Support/Google/Chrome/{Profile}/AccountBookmarks` |
| Linux | `~/.config/google-chrome/{Profile}/AccountBookmarks` |
| Windows | `%LOCALAPPDATA%\Google\Chrome\User Data\{Profile}\AccountBookmarks` |
