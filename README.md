# bookmark-check

Chrome 북마크 URL 유효성 검사 CLI 도구.

Chrome 북마크 파일을 읽어 각 URL에 HTTP HEAD 요청을
동시에 보내고, 살아있는 북마크와 죽은 북마크를 리포트한다.

## 기능

- Chrome 프로필 자동 탐색
- 여러 프로필이 있을 경우 대화형 선택
- 동시 요청 수 설정 가능한 병렬 URL 검사
- 색상이 적용된 테이블 형식 출력
- valid/invalid 비율 프로그레스 바 요약
- macOS, Linux, Windows 지원

## 설치

```bash
cargo install --path .
```

## 사용법

```bash
# 대화형 프로필 선택
bookmark-check

# 모든 프로필 한번에 검사
bookmark-check --all

# 프로필 이름 지정
bookmark-check -p "Default"
bookmark-check -p "Profile 2"

# 북마크 파일 직접 지정
bookmark-check -f /path/to/Bookmarks

# 유효하지 않은 URL만 표시
bookmark-check --invalid-only

# 동시 요청 수, 타임아웃 설정
bookmark-check -c 20 -t 5
```

## 옵션

| 옵션 | 축약 | 기본값 | 설명 |
|---|---|---|---|
| `--file <FILE>` | `-f` | 자동 탐색 | 북마크 파일 경로 |
| `--profile <PROFILE>` | `-p` | 대화형 선택 | 프로필 이름 또는 디렉토리명 |
| `--all` | `-a` | false | 모든 프로필 검사 |
| `--concurrency <N>` | `-c` | 10 | 동시 요청 수 |
| `--timeout <SECS>` | `-t` | 3 | 요청 타임아웃(초) |
| `--invalid-only` | | false | 유효하지 않은 URL만 표시 |

## 출력 예시

```text
  Profile: Personal (Default)

  #   STATUS            NAME                 URL
  -----------------------------------------------
  1   200 OK            Google               https://www.google.com
  2   200 OK            GitHub               https://github.com
  3   TIMEOUT           Old Service          http://dead-link.example.com
  4   404 Not Found     Deleted Page         https://example.com/gone

==================================================

  [##############################] 120/150 valid

  Total    150
  Valid    120
  Invalid   30
```

- **초록색**: 유효 (HTTP 200-399)
- **빨간색**: 무효 (HTTP 400+, 타임아웃, 연결 오류)

## 종료 코드

- `0`: 모든 URL이 유효
- `1`: 무효한 URL이 1개 이상 존재

## Chrome 북마크 경로

| OS | 경로 |
|---|---|
| macOS | `~/Library/Application Support/Google/Chrome/{Profile}/Bookmarks` |
| Linux | `~/.config/google-chrome/{Profile}/Bookmarks` |
| Windows | `%LOCALAPPDATA%\Google\Chrome\User Data\{Profile}\Bookmarks` |
