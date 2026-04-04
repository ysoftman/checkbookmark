# checkbookmark

Chrome 북마크 URL 유효성 검사 TUI 도구.

Chrome 북마크 파일을 읽어 각 URL에 HTTP HEAD 요청을 동시에 보내고,
살아있는 북마크와 죽은 북마크를 리포트하고 편집한다.

![screenshot](screenshot.png)

## 기능

- Chrome 프로필 자동 탐색 및 TUI 선택
- AccountBookmarks → Bookmarks → Bookmarks.bak 우선순위 폴백
- 헤더에 사용 중인 소스 파일 표시
- 동시 요청 수 설정 가능한 병렬 URL 검사
- 실시간 진행률 표시
- 결과 테이블 (valid: 초록, invalid: 노랑)
- 폴더 경로 표시
- 빈 폴더 자동 감지 및 관리
- status/folder/name/url 기준 정렬 (오름차순/내림차순 토글)
- 실시간 검색 필터
- Space/V 키로 개별/전체 선택 후 일괄 삭제
- 북마크 편집/삭제 (Chrome Bookmarks 파일에 직접 반영)
- 변경 후 Chrome 호환 HTML 파일 자동 내보내기
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
| `dd` | 선택 항목 삭제 (확인 팝업, 다중 선택 지원) |
| `r` | 전체 재검사 |
| `q` | 종료 |

## 주의사항

Chrome이 실행 중이거나 재시작하면 북마크 파일을
직접 수정하더라도 sync에 의해 자동으로 원복된다.
삭제/편집 후 생성되는 HTML 내보내기 파일을
Chrome의 `chrome://bookmarks` > ⋮ > **북마크 가져오기**로
import 해야 변경 사항이 반영된다.

## Chrome 북마크 경로

AccountBookmarks → Bookmarks → Bookmarks.bak 순서로 유효한 파일을 탐색한다.

| OS | 경로 |
|---|---|
| macOS | `~/Library/Application Support/Google/Chrome/{Profile}/AccountBookmarks` |
| Linux | `~/.config/google-chrome/{Profile}/AccountBookmarks` |
| Windows | `%LOCALAPPDATA%\Google\Chrome\User Data\{Profile}\AccountBookmarks` |
