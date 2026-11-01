<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/maguro-lockup-dark.png">
    <source media="(prefers-color-scheme: light)" srcset="assets/maguro-lockup-light.png">
    <img alt="Maguro" src="assets/maguro-lockup-light.png" width="192">
  </picture>
</p>

<h1 align="center">Maguro</h1>

<p align="center">빠르고 가벼운 Windows용 다중 패널 파일 관리자.</p>

> **Maguro는 efww의 [efww/grids](https://github.com/efww/grids) 포크입니다.** exe 이름(`maguro.exe`), 창 클래스, 설정 폴더, 단축키(Win+Shift+E)가 달라 원본 Grids와 설정을 건드리지 않고 함께 쓸 수 있습니다.

<p align="center">
  <a href="https://github.com/Samek86/maguro"><img alt="플랫폼: Windows" src="https://img.shields.io/badge/platform-Windows-0078D4?logo=windows&logoColor=white"></a>
  <a href="LICENSE"><img alt="라이선스: MIT" src="https://img.shields.io/badge/license-MIT-blue"></a>
  <a href="https://github.com/Samek86/maguro"><img alt="언어: Rust" src="https://img.shields.io/badge/language-Rust-000000?logo=rust&logoColor=white"></a>
  <a href="https://github.com/Samek86/maguro/commits"><img alt="최근 커밋" src="https://img.shields.io/github/last-commit/Samek86/maguro"></a>
</p>

<p align="center">
  <a href="README.md">English</a> | <a href="README.ja.md">日本語</a> | 한국어
</p>

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/images/hero_dark.ko.png">
    <img alt="Maguro — 라이트 테마" src="docs/images/hero_light.ko.png">
  </picture>
</p>

모든 패널은 진짜 Windows 탐색기 뷰입니다. 전체 우클릭 메뉴, 끌어서 놓기, 썸네일, Windows 복사/이동 대화상자가 그대로 동작합니다. Maguro는 기본적으로 패널 하나로 열리고, 필요할 때 레이아웃을 늘리면 됩니다.

## 기능

### 1. 레이아웃 갤러리 — 모든 프리셋

패널 하나로 시작한 뒤 내장 격자(최대 4×4)를 고를 수 있습니다. 경계를 끌어 크기를 바꿉니다. 이름 있는 세션에는 레이아웃·폴더·열·색 라벨이 들어가고, 세션을 열면 지금 탭 옆에 창 탭이 추가됩니다.

<p align="center"><img width="760" alt="다중 패널 레이아웃" src="docs/images/layout.ko.png"></p>

<table>
<tr>
<td><img alt="1 패널" src="docs/images/layout_1.png"></td>
<td><img alt="가로 2" src="docs/images/layout_2h.png"></td>
<td><img alt="세로 2" src="docs/images/layout_2v.png"></td>
</tr>
<tr>
<td><img alt="3 패널" src="docs/images/layout_3.png"></td>
<td><img alt="세로 쌓기 3" src="docs/images/layout_3_stack.png"></td>
<td><img alt="3열" src="docs/images/layout_3col.png"></td>
</tr>
<tr>
<td><img alt="4 패널" src="docs/images/layout_4.png"></td>
<td><img alt="세로형 4" src="docs/images/layout_4_tall.png"></td>
<td><img alt="5 패널" src="docs/images/layout_5.png"></td>
</tr>
<tr>
<td><img alt="1+1+2" src="docs/images/layout_preset_112.png"></td>
<td><img alt="3+1" src="docs/images/layout_preset_31.png"></td>
<td></td>
</tr>
</table>

### 2. 창 탭과 패널 안 중첩 탭

창 전체의 크롬 스타일 탭에 더해, 각 패널 안에도 탭이 있습니다. 탭 이름은 활성 폴더에서 오고, 툴팁에 전체 경로가 보입니다. 탭 복제, 새 창으로 이동, 세션 저장도 됩니다.

<p align="center"><img width="760" alt="창 탭과 패널 탭" src="docs/images/tabs.ko.png"></p>

### 3. 탭 이름 고정·잠금·색

탭을 더블클릭하거나 우클릭 → 이름 바꾸기로 원하는 이름을 고정할 수 있습니다. 잠그면 실수로 닫기 어렵습니다. 색을 입히면 관련 작업을 한눈에 구분할 수 있습니다.

### 4. 패널 번호를 끌어 자리 바꾸기 — Ctrl+끌기로 복제

패널 번호 배지를 잡아 다른 패널에 놓으면 자리가 바뀝니다. **Ctrl**을 누른 채 끌면 바꾸지 않고 그 패널을 복제합니다.

<p align="center">
  <img width="360" alt="끌기 전" src="docs/images/panedrag_before.png">
  <img width="360" alt="자리 바꾼 뒤" src="docs/images/panedrag_after.png">
</p>
<p align="center"><img width="560" alt="Ctrl+끌기로 패널 복제" src="docs/images/panedrag_dup.png"></p>

### 5. 패널별 색

패널마다 강조색을 줄 수 있습니다. 소스와 목적지를 헷갈리기 쉬운 다중 패널에서 특히 유용합니다.

<p align="center"><img width="760" alt="패널별 색" src="docs/images/pane_colors.ko.png"></p>

### 6. 속도를 위한 설계

이 포터블 데모에서 콜드 스타트 실측은 **607 ms**입니다. 네트워크 폴더로 붙여넣기·끌기는 robocopy를 쓸 수 있습니다(설정 → 파일 작업, 기본 끔). 우클릭 → 빠른 붙여넣기는 항상 쓰고, Maguro 자체 진행률과 덮어쓰기/건너뛰기/취소를 제공합니다.

**Everything 연동 패널 검색.** 패널에서 Ctrl+F로 입력하는 대로 걸러집니다. 하위 폴더를 켜면 [voidtools Everything](https://www.voidtools.com/)에 물어 바로 결과를 받고, Everything이 없으면 디스크를 직접 걷습니다.

<p align="center"><img width="760" alt="Everything 연동 패널 검색" src="docs/images/search.ko.png"></p>

### 7. 즐겨찾기 — 폴더와 파일, 그룹으로

사이드바는 위·아래 절반입니다. 위는 즐겨찾기, 아래는 폴더 트리. 즐겨찾기에는 탐색기 빠른 액세스 고정과 직접 만든 그룹(폴더·파일 모두, 끌어 정렬)이 들어갑니다. Ctrl+B는 빠른 즐겨찾기 목록을 엽니다. 패널에 파일·폴더를 놓으면 즐겨찾기에 추가됩니다.

<p align="center"><img width="760" alt="그룹으로 나눈 즐겨찾기" src="docs/images/favorites.ko.png"></p>

### 8. 빠른 미리보기

Space 또는 F3으로 그림, PDF, Office, HTML, Markdown, 코드, CSV, ZIP, HWP/HWPX(한컴) 등을 미리봅니다. Ctrl+휠로 확대(10–800%), 끌어서 이동, 휠로 스크롤, ←/→로 파일 이동.

<p align="center">
  <img width="480" alt="패널 미리보기" src="docs/images/preview.ko.png">
  <img width="240" alt="피크 미리보기" src="docs/images/preview_peek.ko.png">
</p>

### 9. 구성 가능한 우클릭 메뉴

설정 → 우클릭 메뉴 항목에서 Maguro 고유 명령의 표시/숨김을 고를 수 있습니다. 메뉴를 짧게 유지해도, 전부 되살려도 됩니다.

<p align="center"><img width="560" alt="우클릭 메뉴 설정" src="docs/images/settings_ctxmenu.ko.png"></p>

### 10. 이름 미리보기가 있는 일괄 이름 바꾸기

파일을 고른 뒤 Ctrl+Shift+R. 찾기/바꾸기, 번호 매기기, 확장자, 프리셋을 바꿀 때마다 확정 전 새 이름이 목록으로 갱신됩니다.

<p align="center"><img width="560" alt="이름 미리보기 일괄 바꾸기" src="docs/images/rename.ko.png"></p>

### 11. 서버 연결

즐겨찾기 패널(또는 그룹)을 우클릭 → **서버 연결 추가…**. SMB·WebDAV는 일반 Windows 폴더로 열립니다. SFTP·FTP는 rclone과 WinFsp로 마운트한 뒤 미리보기·우클릭 메뉴·빠른 복사까지 다른 폴더처럼 씁니다. 비밀번호는 Windows 자격 증명 관리자에만 두고 `maguro.json`에는 쓰지 않습니다.

<p align="center"><img width="420" alt="값이 채워진 서버 연결 대화상자" src="docs/images/servers.ko.png"></p>

### 테마와 강조색

Windows를 따르는 라이트/다크, 테마마다 강조색 여섯(기본 세룰리안 `#0A9BD6`). 설정 창은 English·日本語·한국어를 지원합니다.

<table>
<tr>
<td><img alt="설정" src="docs/images/settings.ko.png"></td>
<td><img alt="도움말" src="docs/images/help.ko.png"></td>
</tr>
<tr>
<td colspan="2"><img alt="다크 테마" src="docs/images/hero_dark.ko.png"></td>
</tr>
</table>

📦 **작은 단일 exe.** 설치할 것이 없습니다.

## 설치

[Releases](https://github.com/Samek86/maguro/releases/latest)에서 최신 `maguro.exe`를 받아 아무 폴더에 두고 실행합니다. 릴리스는 zip이 아니라 exe와 `maguro.exe.sha256`을 올립니다.

코드 서명이 없어 처음 실행 때 SmartScreen이 경고할 수 있습니다. **추가 정보 → 실행**을 고르세요.

**요구 사항:** Windows 10 (1903 이상) 또는 Windows 11, x64. SFTP/FTP는 [rclone](https://rclone.org/)과 [WinFsp](https://winfsp.dev/)도 필요합니다(연결 추가 때 Maguro가 설치를 안내할 수 있음).

설정은 `%APPDATA%\Maguro\maguro.json`에 있습니다. `maguro.exe` 옆에 `portable.txt`를 두면 설정이 exe 옆에 생깁니다. 이름 있는 세션은 exe 옆 `maguro-sessions` 폴더의 `.grids` 파일입니다.

소스에서 빌드하려면 [CONTRIBUTING.md](CONTRIBUTING.md)를 보세요.

## 단축키

기본값이며, 설정 → 단축키에서 모두 바꿀 수 있습니다.

| 구분 | 키 | 동작 |
| --- | --- | --- |
| **탭** | Ctrl+T | 새 탭 |
|  | Ctrl+W | 탭 닫기 |
|  | Ctrl+Tab / Ctrl+Shift+Tab | 다음 / 이전 탭 |
|  | Ctrl+Shift+T | 닫은 탭 다시 열기 |
| **패널** | Alt+1…9 | 패널 포커스 |
|  | Ctrl+\ | 레이아웃 고르기 |
| **미리보기** | Space / F3 | 미리보기 |
|  | Esc / Space / F3 | 미리보기 닫기 |
| **파일** | Ctrl+Shift+C | 전체 경로 복사 |
|  | Ctrl+Shift+R | 일괄 이름 바꾸기 |
|  | Num+ 또는 Ctrl+Shift+A / Num− / Num* | 패턴으로 선택 / 해제 / 반전 |
|  | Ctrl+Alt+N | 새 텍스트 파일 |
|  | Ctrl+Shift+N, F7 | 새 폴더 |
|  | Ctrl+Alt+C | WinMerge로 두 패널 비교 |
|  | F2, Del, Ctrl+C / V, F5 | 탐색기와 같음. F5는 활성 패널 다시 로드, 실패한 네트워크 폴더도 다시 시도 |
| **기타** | Ctrl+B | 빠른 즐겨찾기 |
|  | Ctrl+S | 이 탭 저장 |
|  | Ctrl+, | 설정 |
|  | F1 / Ctrl+/ | 도움말 / 단축키 목록 |
|  | Ctrl+N | 새 창 |
|  | Ctrl+Alt+G | 다른 앱의 열기/저장 대화 상자에서 Maguro 폴더로 이동 |
|  | Win+Shift+E | Maguro를 앞으로 (전역) |

## 알려진 문제

**Maguro를 관리자로 실행하지 마세요.** PowerToys Mouse Crosshairs 같은 도구는 권한이 높은 Maguro 창 위에서 커서를 따라가지 못합니다(UIPI). Maguro는 일반으로 실행하거나, Maguro와 PowerToys를 둘 다 관리자로 실행하세요.

## 설정

탭 줄의 톱니바퀴가 보기 메뉴를 엽니다. 설정은 그 메뉴에 있습니다(`Ctrl+,`).

언어(자동, English, 그리고 언어 파일마다), 글꼴, 라이트/다크, 강조색, 단축키는 설정에서 바꿉니다. 이전 Grids 설정 이전, 파일 저장 방식, `maguro.json` 키 전체 표는 [docs/settings.ko.md](docs/settings.ko.md)에 있습니다.

## 언어 파일

English는 프로그램에 들어가 있고 대체값입니다. 한국어·日本語·French·German은 `lang/lang.ko.ini` 등으로 배포됩니다. 처음 실행과 버전이 바뀔 때마다 설정 옆 `lang` 폴더에 그 네 파일을 다시 씁니다. `lang.xx.user.ini`나 직접 추가한 `lang.xx.ini`는 건드리지 않습니다.

파일은 UTF-8입니다. 줄은 `key = text`. `[meta]` `name`은 언어 목록 라벨, `[meta]` `font`는 선택 UI 글꼴. 없는 키는 English를 씁니다. `lang.xx.user.ini`가 위에 덮어씁니다.

## 로드맵

- [ ] 코드 서명 빌드 — SmartScreen이 처음 실행을 막지 않도록
- [x] English·한국어·日本語 밖의 인터페이스 언어

## 감사

Maguro는 efww의 [efww/grids](https://github.com/efww/grids) 포크입니다.

[Q-Dir](https://www.softwareok.com/?seite=Freeware/Q-Dir)과 [voidtools Everything](https://www.voidtools.com/)에서 영감을 받았습니다. Maguro는 독립 프로젝트이며 Q-Dir 및 제작자와 무관합니다.

## 라이선스

[MIT](LICENSE) · © 2026 efww, Samek86