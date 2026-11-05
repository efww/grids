# 스크린샷 캡처 기록 (docs/images-v3)

Maguro 0.2.3 README / 도움말용 스크린샷. 모든 제품 이미지는 **포터블 빌드만** 사용하고, AppData·레지스트리·grids.ini·설치본 경로를 쓰지 않는다.

## 공통 규칙

- **실행 파일:** 포터블 maguro.exe (작업 디렉터리 = exe 옆). 설정은 같은 폴더의 maguro.json만.
- **테마:** README 히어로 다크를 제외하면 라이트. 언어는 파일 접미사 .ko / .ja / (없음=en).
- **배경:** 흰 백드롭 창을 대상 창 뒤에 깔고 CopyFromScreen (스크립트: docs/capture/white-backdrop.ps1).
- **크롭:** DwmGetWindowAttribute(..., DWMWA_EXTENDED_FRAME_BOUNDS) 로 그림자·다크 가장자리 없이 프레임만.
- **가장자리 검사:** 왼쪽 어두운 픽셀 ≥3 또는 높이의 5% 초과면 거부. 다른 변은 5% 스트립 초과만 거부.
- **페인트:** ShowWindow는 셸 확장 열거 중 데드락 위험이 있어 쓰지 않는다. 대신 RedrawWindow(RDW_INVALIDATE|RDW_UPDATENOW|RDW_ALLCHILDREN) 후 캡처 (docs/capture/cap-lib.ps1).
- **닫기:** 대상 프로세스 PID의 창에만 WM_CLOSE (다른 Maguro 인스턴스에 닿지 않음).
- **재사용 스크립트:** docs/capture/ — 경로는 환경변수 MAGURO_PORTABLE, MAGURO_CFG_DIR, MAGURO_SHOT_DIR 로만 지정. 개인 경로·실서버·실비밀번호를 스크립트에 넣지 말 것.

## 서버 연결 창 — 가짜 데모 값만 (필수)

servers.png / servers.ko.png / servers.ja.png 는 **실제 호스트·자격 증명을 절대 넣지 않는다.**

| 필드 | 허용 값 |
| --- | --- |
| 표시 이름 | Demo Server |
| 호스트 | sftp.example.com |
| 포트 | 22 |
| 원격 경로 | /home/demo/projects |
| 사용자 | demo |
| 비밀번호 | 비움 (또는 UI가 가리는 점만). 실비밀번호 금지 |
| 종류 | SFTP 선택 |

- **만드는 법:** Ctrl+L → sftp://demo@sftp.example.com/home/demo/projects 입력 → Enter → MaguroServer 대화상자에서 위 필드를 SetEdit(ID 10–15)로 맞춘 뒤 SFTP 종류 클릭 → 캡처.
- **스크립트:** docs/capture/capture-servers-fake.ps1
- **금지:** lab.demo.local, 사내 호스트, 실계정, 즐겨찾기에 심어 둔 Lab SFTP를 그대로 찍기.
- **크기(2026-11-04 재촬영):** EN 400×560 ≈11KB, KO ≈10KB, JA ≈13KB. 가장자리 다크=0.

## 설정 › 패널·보기 (settings_panes*)

- **창:** MaguroSettings, 라이트, EN/KO/JA.
- **페이지:** 네비 인덱스 2 = Panes & view / 패널·보기 / ペインと表示.
- **조작:** Ctrl+, 로 설정 연 뒤 **클라이언트 좌표** WM_LBUTTONDOWN/UP 으로 네비 클릭 (Click-MaguroSettingsPage). 화면 마우스 클릭은 종종 General에 머무름.
- **데모 데이터:** dialogs.json / avorites.ko.json / hero_light.ja.json 시드 + lang 덮어쓰기. 특수 서버 불필요.
- **스크립트:** docs/capture/capture-settings-panes.ps1
- **크기(재촬영):** 986×633, EN≈34KB, KO≈32KB, JA≈47KB. 빈 흰 클라이언트(~7KB)는 RedrawWindow 누락 — 거부.
- **주의:** 캡처 전 셸 확장 열거가 끝날 때까지 ≈7초 대기.

## 이미지별 기록 (2026-11-04 재촬영 기준)

| 파일 | 한국어 설명 | 창/대상 | 테마/언어 | 데모 데이터 | 크기(실측) | 방법/스크립트 | 크롭 | 주의 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| address_toolbar.ja.png | 주소 표시줄·툴바 | MaguroMain 주소줄 | light/ja | address cfg | 12501 B (1212×160) | 메인 캡처 | DWM |  |
| address_toolbar.ko.png | 주소 표시줄·툴바 | MaguroMain 주소줄 | light/ko | address cfg | 12189 B (1212×160) | 메인 캡처 | DWM |  |
| address_toolbar.png | 주소 표시줄·툴바 | MaguroMain 주소줄 | light/en | address cfg | 12283 B (1212×160) | 메인 캡처 | DWM |  |
| copy_progress.ja.png | 복사 진행 | MaguroFastCopy | light/ja | help-demo FastCopy | 3420 B (480×152) | capture-help-dialogs.ps1 / MaguroFastCopy | DWM | Fast copy 진행 창 |
| copy_progress.ko.png | 복사 진행 | MaguroFastCopy | light/ko | help-demo FastCopy | 2527 B (480×152) | capture-help-dialogs.ps1 / MaguroFastCopy | DWM | Fast copy 진행 창 |
| copy_progress.png | 복사 진행 | MaguroFastCopy | light/en | help-demo FastCopy | 2650 B (480×152) | capture-help-dialogs.ps1 / MaguroFastCopy | DWM | Fast copy 진행 창 |
| dialog_jump.ja.png | 대화상자 점프 | MaguroDlgJump 오버레이 | light/ja | help-demo 점프 | 7702 B (420×138) | capture-help-dialogs.ps1 | DWM | 경로만 표시 — KO/JA 동일 가능 |
| dialog_jump.ko.png | 대화상자 점프 | MaguroDlgJump 오버레이 | light/ko | help-demo 점프 | 7702 B (420×138) | capture-help-dialogs.ps1 | DWM | 경로만 표시 — KO/JA 동일 가능 |
| dialog_jump.png | 대화상자 점프 | MaguroDlgJump 오버레이 | light/en | help-demo 점프 | 7848 B (420×138) | capture-help-dialogs.ps1 | DWM | 경로만 표시 — KO/JA 동일 가능 |
| favorites.ja.png | 즐겨찾기 | MaguroMain 사이드바 | light/ja | favorites 시드 | 305322 B (1226×823) | capture-main.ps1 | DWM |  |
| favorites.ko.png | 즐겨찾기 | MaguroMain 사이드바 | light/ko | favorites 시드 | 303594 B (1226×823) | capture-main.ps1 | DWM |  |
| favorites.png | 즐겨찾기 | MaguroMain 사이드바 | light/en | favorites 시드 | 301707 B (1226×823) | capture-main.ps1 | DWM |  |
| feature_basket.ja.png | 파일 바구니 | MaguroBasket | light/ja | 데모 | 28005 B (544×513) | capture-features.ps1 | DWM |  |
| feature_basket.ko.png | 파일 바구니 | MaguroBasket | light/ko | 데모 | 27101 B (544×513) | capture-features.ps1 | DWM |  |
| feature_basket.png | 파일 바구니 | MaguroBasket | light/en | 데모 | 24121 B (546×513) | capture-features.ps1 | DWM |  |
| feature_excel_readonly_tidy.ja.png | Excel 정리 | MaguroExcelTidy | light/ja | readme-excel-demo sheet.xlsx | 7672 B (422×244) | 컨텍스트 메뉴 id=0x8007 (PR #312 shared UI) | DWM | images-v4 재촬영 |
| feature_excel_readonly_tidy.ko.png | Excel 정리 | MaguroExcelTidy | light/ko | readme-excel-demo sheet.xlsx | 5797 B (422×244) | 컨텍스트 메뉴 id=0x8007 (PR #312 shared UI) | DWM | images-v4 재촬영 |
| feature_excel_readonly_tidy.png | Excel 정리 | MaguroExcelTidy | light/en | readme-excel-demo sheet.xlsx | 6217 B (422×244) | 컨텍스트 메뉴 id=0x8007 (PR #312 shared UI) | DWM | images-v4 재촬영 |
| feature_folder_watch.ja.png | 폴더 감시 | MaguroWatch | light/ja | 데모 | 20342 B (746×433) | capture-features.ps1 | DWM |  |
| feature_folder_watch.ko.png | 폴더 감시 | MaguroWatch | light/ko | 데모 | 16051 B (746×433) | capture-features.ps1 | DWM |  |
| feature_folder_watch.png | 폴더 감시 | MaguroWatch | light/en | 데모 | 15631 B (746×433) | capture-features.ps1 | DWM |  |
| feature_win_shift_v.ja.png | Win+Alt+V | MaguroMain 새 탭 | light/ja | 클립보드=Photos 2026 경로 | 26231 B (1226×833) | WM_HOTKEY CLIP_HOTKEY_ID=3 (sidebar off) | DWM | Photos 2026 새 탭 · 사이드바/실경로 없음 |
| feature_win_shift_v.ko.png | Win+Alt+V | MaguroMain 새 탭 | light/ko | 클립보드=Photos 2026 경로 | 25784 B (1226×833) | WM_HOTKEY CLIP_HOTKEY_ID=3 (sidebar off) | DWM | Photos 2026 새 탭 · 사이드바/실경로 없음 |
| feature_win_shift_v.png | Win+Alt+V | MaguroMain 새 탭 | light/en | 클립보드=Photos 2026 경로 | 25455 B (1226×833) | WM_HOTKEY CLIP_HOTKEY_ID=3 (sidebar off) | DWM | Photos 2026 새 탭 · 사이드바/실경로 없음 |
| help.ja.png | 도움말 창 | MaguroHelp/브라우저 | light/ja | — | 29368 B (760×672) | F1 | DWM |  |
| help.ko.png | 도움말 창 | MaguroHelp/브라우저 | light/ko | — | 22566 B (760×672) | F1 | DWM |  |
| help.png | 도움말 창 | MaguroHelp/브라우저 | light/en | — | 24725 B (760×672) | F1 | DWM |  |
| hero_dark.ja.png | 히어로 다크 | MaguroMain | dark/ja | hero_dark 시드 | 301033 B (1226×823) | capture-main.ps1 | DWM |  |
| hero_dark.ko.png | 히어로 다크 | MaguroMain | dark/ko | hero_dark 시드 | 299260 B (1226×823) | capture-main.ps1 | DWM |  |
| hero_dark.png | 히어로 다크 | MaguroMain | dark/en | hero_dark 시드 | 297403 B (1226×823) | capture-main.ps1 | DWM |  |
| hero_light.ja.png | 히어로 라이트 | MaguroMain | light/ja | hero_light 시드 | 305322 B (1226×823) | capture-main.ps1 | DWM |  |
| hero_light.ko.png | 히어로 라이트 | MaguroMain | light/ko | hero_light 시드 | 303594 B (1226×823) | capture-main.ps1 | DWM |  |
| hero_light.png | 히어로 라이트 | MaguroMain | light/en | hero_light 시드 | 301707 B (1226×823) | capture-main.ps1 | DWM |  |
| layout.ja.png | 레이아웃 개요 | MaguroMain | light/ja | layout 시드 | 283018 B (1226×823) | capture-main.ps1 | DWM |  |
| layout.ko.png | 레이아웃 개요 | MaguroMain | light/ko | layout 시드 | 281590 B (1226×823) | capture-main.ps1 | DWM |  |
| layout.png | 레이아웃 개요 | MaguroMain | light/en | layout 시드 | 279800 B (1226×823) | capture-main.ps1 | DWM |  |
| layout_1.png | 레이아웃 1 | MaguroMain | light/en | layout_1 | 291102 B (1226×823) | capture-main.ps1 | DWM |  |
| layout_2h.png | 레이아웃 2H | MaguroMain | light/en | layout_2h | 301707 B (1226×823) | capture-main.ps1 | DWM |  |
| layout_2v.png | 레이아웃 2V | MaguroMain | light/en | layout_2v | 300781 B (1226×823) | capture-main.ps1 | DWM |  |
| layout_3.png | 레이아웃 3 | MaguroMain | light/en | layout_3 | 307085 B (1226×823) | capture-main.ps1 | DWM |  |
| layout_3_stack.png | 레이아웃 3 스택 | MaguroMain | light/en | layout_3_stack | 272125 B (1226×823) | capture-main.ps1 | DWM |  |
| layout_3col.png | 레이아웃 3열 | MaguroMain | light/en | layout_3col | 312616 B (1226×823) | capture-main.ps1 | DWM |  |
| layout_4.png | 레이아웃 4 | MaguroMain | light/en | layout_4 | 279800 B (1226×823) | capture-main.ps1 | DWM |  |
| layout_4_tall.png | 레이아웃 4 세로 | MaguroMain | light/en | layout_4_tall | 320086 B (1226×823) | capture-main.ps1 | DWM |  |
| layout_5.png | 레이아웃 5 | MaguroMain | light/en | layout_5 | 297474 B (1226×823) | capture-main.ps1 | DWM |  |
| layout_preset_112.png | 프리셋 1-1-2 | MaguroMain | light/en | preset | 320600 B (1226×823) | capture-main.ps1 | DWM |  |
| layout_preset_31.png | 프리셋 3-1 | MaguroMain | light/en | preset | 214283 B (1226×823) | capture-main.ps1 | DWM |  |
| pane_colors.ja.png | 패널 색 | MaguroMain | light/ja | tint 시드 | 305322 B (1226×823) | capture-main.ps1 | DWM |  |
| pane_colors.ko.png | 패널 색 | MaguroMain | light/ko | tint 시드 | 303594 B (1226×823) | capture-main.ps1 | DWM |  |
| pane_colors.png | 패널 색 | MaguroMain | light/en | tint 시드 | 301707 B (1226×823) | capture-main.ps1 | DWM |  |
| pane_tabs.ja.png | 패널 탭 | MaguroMain | light/ja | tabs 시드 | 32181 B (1212×420) | capture-features / main | DWM |  |
| pane_tabs.ko.png | 패널 탭 | MaguroMain | light/ko | tabs 시드 | 31779 B (1212×420) | capture-features / main | DWM |  |
| pane_tabs.png | 패널 탭 | MaguroMain | light/en | tabs 시드 | 32205 B (1212×420) | capture-features / main | DWM |  |
| panedrag_after.png | 패널 드래그 후 | MaguroMain | light/en | panedrag | 300889 B (1226×823) | capture-main.ps1 | DWM |  |
| panedrag_before.png | 패널 드래그 전 | MaguroMain | light/en | panedrag | 301707 B (1226×823) | capture-main.ps1 | DWM |  |
| panedrag_dup.png | 패널 드래그 복제 | MaguroMain | light/en | panedrag | 518088 B (1226×823) | capture-main.ps1 | DWM |  |
| preview.ja.png | 미리보기 | MaguroMain | light/ja | Photos 2026 | 304453 B (1226×823) | capture-main.ps1 | DWM | **재촬영 필요.** 속성 머리글(아이콘·이름·크기·종류·날짜·특성)과 처리기 없을 때의 본문(텍스트/코드/CSV/폴더)이 바뀜 |
| preview.ko.png | 미리보기 | MaguroMain | light/ko | Photos 2026 | 302690 B (1226×823) | capture-main.ps1 | DWM | **재촬영 필요.** 속성 머리글과 폴백 본문이 바뀜 |
| preview.png | 미리보기 | MaguroMain | light/en | Photos 2026 | 301707 B (1226×823) | capture-main.ps1 | DWM | **재촬영 필요.** 속성 머리글과 폴백 본문이 바뀜 |
| preview_peek.ja.png | 피크 미리보기 | MaguroPeek | light/ja | photo-01.jpg 시드 | 1162153 B (918×678) | F3 MaguroPeek + TOPMOST | DWM | **재촬영 필요.** F3 머리글이 이름 + 속성/EXIF 두 줄. 실 JPG 시드 |
| preview_peek.ko.png | 피크 미리보기 | MaguroPeek | light/ko | photo-01.jpg 시드 | 1162134 B (918×678) | F3 MaguroPeek + TOPMOST | DWM | **재촬영 필요.** F3 머리글이 이름 + 속성/EXIF 두 줄. 실 JPG 시드 |
| preview_peek.png | 피크 미리보기 | MaguroPeek | light/en | photo-01.jpg 시드 | 1161933 B (918×678) | F3 MaguroPeek + TOPMOST | DWM | **재촬영 필요.** F3 머리글이 이름 + 속성/EXIF 두 줄. 실 JPG 시드 |
| rename.ja.png | 일괄 이름 바꾸기 | MaguroRename | light/ja | rename_camera 시드 | 41082 B (1346×693) | capture-features.ps1 | DWM |  |
| rename.ko.png | 일괄 이름 바꾸기 | MaguroRename | light/ko | rename_camera 시드 | 35251 B (1346×693) | capture-features.ps1 | DWM |  |
| rename.png | 일괄 이름 바꾸기 | MaguroRename | light/en | rename_camera 시드 | 35660 B (1346×693) | capture-features.ps1 | DWM |  |
| search.ja.png | 하위폴더 검색 | MaguroSearch | light/ja | 데모 트리 | 56167 B (1226×823) | 검색 캡처 | DWM |  |
| search.ko.png | 하위폴더 검색 | MaguroSearch | light/ko | 데모 트리 | 54110 B (1226×823) | 검색 캡처 | DWM |  |
| search.png | 하위폴더 검색 | MaguroSearch | light/en | 데모 트리 | 52222 B (1226×823) | 검색 캡처 | DWM |  |
| servers.ja.png | 서버 연결 추가 | MaguroServer | light/ja | **가짜** Demo Server / sftp.example.com | 12940 B (400×560) | capture-servers-fake.ps1 | DWM | 실호스트·실비번 금지 |
| servers.ko.png | 서버 연결 추가 | MaguroServer | light/ko | **가짜** Demo Server / sftp.example.com | 10236 B (400×560) | capture-servers-fake.ps1 | DWM | 실호스트·실비번 금지 |
| servers.png | 서버 연결 추가 | MaguroServer | light/en | **가짜** Demo Server / sftp.example.com | 11651 B (400×560) | capture-servers-fake.ps1 | DWM | 실호스트·실비번 금지 |
| settings.ja.png | 설정 일반 | MaguroSettings General | light/ja | dialogs 시드 | 46373 B (986×633) | Ctrl+, + RedrawWindow | DWM |  |
| settings.ko.png | 설정 일반 | MaguroSettings General | light/ko | dialogs 시드 | 33848 B (986×633) | Ctrl+, + RedrawWindow | DWM |  |
| settings.png | 설정 일반 | MaguroSettings General | light/en | dialogs 시드 | 35243 B (986×633) | Ctrl+, + RedrawWindow | DWM |  |
| settings_ctxmenu.ja.png | 설정 우클릭 메뉴 | MaguroSettings CTX_PAGE=8 | light/ja | 시드 | 38516 B (986×633) | Click-MaguroSettingsPage(8) | DWM | Right-click menu items 페이지인지 눈확인 |
| settings_ctxmenu.ko.png | 설정 우클릭 메뉴 | MaguroSettings CTX_PAGE=8 | light/ko | 시드 | 30605 B (986×633) | Click-MaguroSettingsPage(8) | DWM | Right-click menu items 페이지인지 눈확인 |
| settings_ctxmenu.png | 설정 우클릭 메뉴 | MaguroSettings CTX_PAGE=8 | light/en | 시드 | 32467 B (986×633) | Click-MaguroSettingsPage(8) | DWM | Right-click menu items 페이지인지 눈확인 |
| settings_general.ja.png | 설정 일반(별칭) | MaguroSettings General | light/ja | 동일 | 46373 B (986×633) | Ctrl+, | DWM |  |
| settings_general.ko.png | 설정 일반(별칭) | MaguroSettings General | light/ko | 동일 | 33848 B (986×633) | Ctrl+, | DWM |  |
| settings_general.png | 설정 일반(별칭) | MaguroSettings General | light/en | 동일 | 34694 B (986×633) | Ctrl+, | DWM |  |
| settings_panes.ja.png | 설정 패널·보기 | MaguroSettings page2 | light/ja | dialogs/fav 시드 | 46865 B (986×633) | capture-settings-panes.ps1 | DWM |  |
| settings_panes.ko.png | 설정 패널·보기 | MaguroSettings page2 | light/ko | dialogs/fav 시드 | 31688 B (986×633) | capture-settings-panes.ps1 | DWM |  |
| settings_panes.png | 설정 패널·보기 | MaguroSettings page2 | light/en | dialogs/fav 시드 | 34309 B (986×633) | capture-settings-panes.ps1 | DWM |  |
| tab_list.png | 탭 목록 | MaguroTabList | light/en | tabs.json | 17406 B (562×532) | WM_COMMAND 탭목록 | DWM |  |
| tabs.ja.png | 창 탭·패널 탭 | MaguroMain | light/ja | tabs.json | 61956 B (1226×823) | tabs cfg | DWM |  |
| tabs.ko.png | 창 탭·패널 탭 | MaguroMain | light/ko | tabs.json | 60849 B (1226×823) | tabs cfg | DWM |  |
| tabs.png | 창 탭·패널 탭 | MaguroMain | light/en | tabs.json | 58828 B (1226×823) | tabs cfg | DWM |  |
| images-v5/workspaces.ja.png | 워크스페이스 메뉴 | 톱니 메뉴 → 작업 공간 하위 메뉴 | light/ja | workspaces 시드 Photos/Docs/Work | 20925 B (618×491) | WM_COMMAND 207 → 메뉴 HWND에 VK_DOWN×3, VK_RIGHT PostMessage | 화면 복사(흰 배경) | 슬롯 1–3 이름, 4–9 빈 슬롯 |
| images-v5/workspaces.ko.png | 워크스페이스 메뉴 | 톱니 메뉴 → 작업 공간 하위 메뉴 | light/ko | workspaces 시드 Photos/Docs/Work | 18119 B (525×491) | WM_COMMAND 207 → 메뉴 HWND에 VK_DOWN×3, VK_RIGHT PostMessage | 화면 복사(흰 배경) | 슬롯 1–3 이름, 4–9 빈 슬롯 |
| images-v5/workspaces.png | 워크스페이스 메뉴 | 톱니 메뉴 → 작업 공간 하위 메뉴 | light/en | workspaces 시드 Photos/Docs/Work | 21624 B (622×491) | WM_COMMAND 207 → 메뉴 HWND에 VK_DOWN×3, VK_RIGHT PostMessage | 화면 복사(흰 배경) | 슬롯 1–3 이름, 4–9 빈 슬롯 |

## 톱니바퀴 메뉴 (gear popup) — 재촬영

images-v3 안에는 **열린 톱니바퀴 메뉴**를 찍은 파일이 없다. 메뉴 내용이 바뀌었어도, 아래 샷은 닫힌 톱니 단추만 탭 줄에 보이므로 이 변경만으로 다시 찍지 않는다. 메뉴를 펼친 샷을 새로 만들 때는 묶음 순서대로 찍는다. 세션(이 탭 저장, 세션 열기, 작업 공간), 도구(파일 바구니, 최근 파일, 탐색기 창의 폴더 가져오기, 폴더 감시, WinMerge), 보기, 모양, 동작, 그리고 설정·단축키·도움말·정보. 폴더에서 표시는 톱니 메뉴에 없다.

닫힌 톱니 단추가 프레임에 들어오는 메인 창 샷 (메뉴는 안 열림):

- address_toolbar.png / address_toolbar.ko.png / address_toolbar.ja.png
- hero_light.png / hero_light.ko.png / hero_light.ja.png
- hero_dark.png / hero_dark.ko.png / hero_dark.ja.png
- favorites.png / favorites.ko.png / favorites.ja.png
- layout.png / layout.ko.png / layout.ja.png 와 layout_*.png, layout_preset_*.png
- preview.png / preview.ko.png / preview.ja.png
- pane_colors.png / pane_colors.ko.png / pane_colors.ja.png
- tabs.png / tabs.ko.png / tabs.ja.png
- search.png / search.ko.png / search.ja.png
- feature_win_shift_v.png / feature_win_shift_v.ko.png / feature_win_shift_v.ja.png
- panedrag_before.png / panedrag_after.png / panedrag_dup.png

## 환경 변수 예시

```powershell
$env:MAGURO_PORTABLE = "C:\path\to\maguro-app"
$env:MAGURO_CFG_DIR  = "C:\path\to\readme-cfgs"
$env:MAGURO_SHOT_DIR = "C:\path\to\shots"
powershell -NoProfile -ExecutionPolicy Bypass -File docs\capture\capture-servers-fake.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File docs\capture\capture-settings-panes.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File docs\capture\capture-help-dialogs.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File docs\capture\capture-features.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File docs\capture\capture-main.ps1
```

## UI 통일 1차 이후 다시 찍을 것

버튼·카드·검색 칸이 공유 토큰으로 바뀌었다. 아래 캡처는 다시 찍는다.

- `feature_folder_watch.png`, `feature_folder_watch.ko.png`, `feature_folder_watch.ja.png`
- `feature_basket.png`, `feature_basket.ko.png`, `feature_basket.ja.png`
- `tab_list.png`
- `rename.png`, `rename.ko.png`, `rename.ja.png`
- `copy_progress.png`, `copy_progress.ko.png`, `copy_progress.ja.png` — 푸터 버튼이 28 DIP라 창 높이는 480×148. 위 표의 480×152는 재촬영 전 값이다.

세션 목록(`MaguroSessionList`)은 지금 캡처가 없다. 다음에 그 창을 찍을 때 새 버튼을 담는다.

시스템 대화상자(IFileOperation 진행, 탐색기 메뉴, 공통 파일 대화상자)는 그대로다. 이름 묻기, 즐겨찾기 관리, 정보, Excel, 확인 창, 그리고 팝업·알림(폴더 감시 토스트, 즐겨찾기 없음, 네트워크 실패, 빠른 복사 진행·이름 충돌, 세션 목록, 단축키, 레이아웃 고르기, 선택 칸, 서버 연결의 저장/취소)은 공유 토큰으로 옮겼다. 설정 창의 토글은 모양과 동작이 이전과 같다.

## 변경 이력 (관련)

- **2026-11-04:** 톱니바퀴 메뉴를 세션 / 도구 / 보기 / 모양 / 동작 / 설정으로 나눔. 열린 메뉴 샷은 images-v3에 없음 (위 절). 닫힌 톱니 단추만 있는 메인 창 샷은 다시 찍지 않음.
- **2026-11-05:** docs/retake-15-12 — feature_win_shift_v* 재촬영(Photos 2026 새 탭, 사이드바·Folder Watch·실경로 제거). feature_excel_readonly_tidy* images-v4로 재촬영(PR #312 shared MaguroExcelTidy) + README §12 src를 v4로.
- **2026-11-04:** UI 통일 1차. 폴더 감시, 파일 바구니, 탭 목록, 일괄 이름 바꾸기, 세션 목록의 버튼을 공유 페인터로 옮김. 위 캡처를 다시 찍을 것. 설정 토글은 유지.
- **2026-11-04:** UI 통일 팝업·알림. 감시 토스트, 즐겨찾기 없음, 네트워크 실패, 빠른 복사 진행·충돌, 세션 목록, 단축키, 레이아웃 고르기, 선택 칸, 서버 연결 푸터를 공유 페인터로 옮김. 빠른 복사 진행 창은 480×148. 이름 묻기, 즐겨찾기 관리, 정보, Excel, 확인 창은 이전 배치 그대로다.
- **2026-11-04:** images-v3 전체(90장) 포터블 재촬영. servers*=가짜 sftp.example.com. preview_peek*=F3 MaguroPeek+실 JPG. feature_win_shift_v*=클립보드 경로 새 탭(Folder Watch 없음). settings_ctxmenu*=CTX_PAGE=8 Right-click menu items. workspaces*=MaguroAsk 슬롯명 Desk. docs/capture/* 재사용 스크립트. CAPTURES 크기 실측 갱신.
- **미리보기 창 본문 변경 후 재촬영:** `preview.png`, `preview.ko.png`, `preview.ja.png` (속성 머리글, 처리기 위 머리글, 텍스트/코드/CSV/폴더 본문). `preview_peek.png`, `preview_peek.ko.png`, `preview_peek.ja.png` (F3 제목 칸의 이름 + 속성/EXIF). 위 표의 바이트 크기는 재촬영 전 값이다.
- **2026-11-04 오전:** servers·settings_panes 가짜/페이지 수정 재촬영.
