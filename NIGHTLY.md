# Grids Nightly AI 업데이트 인덱스

Claude가 제안하고 gpt-6.1-sol이 동의한 개선을 사람 확인 없이 구현해, 몇 개씩 묶어 공개한 기록입니다. 번호는 계속 이어집니다.
정식 릴리즈와 별개이며, 최신 exe는 [nightly-ai 릴리즈](https://github.com/efww/grids/releases/tag/nightly-ai)에 있습니다.

## 릴리즈 R1 · 2026-10-21 10:56 KST · 업데이트 1개

| # | 구현 시각(KST) | 분류 | 업데이트 | 사용자에게 달라지는 것 | 변경 |
|---|---|---|---|---|---|
| #1 | 2026-10-21 10:55 | UX | Folder path suggestions in the address bar | When you type a path in a pane's address bar (Ctrl+L), a list of matching folders drops down, the same as in Explorer. Pick one with the arrow keys and press Enter to go there, so a mistyped path no longer just beeps. | 1 file changed, 3 insertions(+), 1 deletion(-) |

## 릴리즈 R2 · 2026-10-21 11:48 KST · 업데이트 3개

| # | 구현 시각(KST) | 분류 | 업데이트 | 사용자에게 달라지는 것 | 변경 |
|---|---|---|---|---|---|
| #12 | 2026-10-21 11:44 | 기능 | Ctrl+V 파일 복사를 백그라운드 스레드에서 실행 | 파일을 Ctrl+C 후 Ctrl+V로 대량으로 붙여넣으면 Windows 복사 진행 창만 따로 돌고, 그동안 Grids의 다른 창 탐색, 트리, 레이아웃 조작이 계속 된다. 하단 상태줄에는 '복사 중…'이 표시되고, 복사 중에 Grids를 닫아도 복사는 끝까지 진행된다. | 2 files changed, 168 insertions(+), 10 deletions(-) |
| #20 | 2026-10-21 11:47 | UX | Show the active pane's folder in the window title | The title bar, taskbar button and Alt+Tab now show the active pane's folder, such as "Downloads - Grids", and update when you switch panes or folders. | 1 file changed, 11 insertions(+) |
| #18 | 2026-10-21 11:48 | 디자인 | 필터가 적용된 패널의 필터 칸에 선택 색 배지 표시 | 필터가 걸린 패널은 필터 칸 왼쪽 돋보기가 선택 색 배지와 필터 아이콘으로 바뀐다. 그래서 활성 패널이 아니어도 어느 패널에서 파일이 가려져 있는지 한눈에 보인다. 필터를 지우면 바로 돋보기로 돌아가고, 선택 색을 바꾸면 모든 패널의 배지가 함께 바뀐다. | 1 file changed, 23 insertions(+), 8 deletions(-) |

## 릴리즈 R3 · 2026-10-21 12:00 KST · 업데이트 3개

| # | 구현 시각(KST) | 분류 | 업데이트 | 사용자에게 달라지는 것 | 변경 |
|---|---|---|---|---|---|
| #23 | 2026-10-21 11:58 | UX | Ctrl+1~9로 해당 번호 패널로 바로 이동 | Ctrl+1부터 Ctrl+9까지 누르면 해당 번호 패널이 바로 활성화되고 포커스가 그 목록으로 옮겨 갑니다. 번호는 ⇄ 메뉴의 '패널 N'과 같습니다. 단축키 창과 README에도 새 키가 나옵니다. | 3 files changed, 9 insertions(+) |
| #17 | 2026-10-21 11:59 | UX | Keep the current folder's name visible in narrow address bars | In small panes the address bar now shows the end of the path, the folder you're in, instead of the drive and the first few folders. You can tell 9 or 16 panes apart at a glance. Typing in the address bar and Ctrl+L work as before. | 1 file changed, 23 insertions(+), 3 deletions(-) |
| #21 | 2026-10-21 12:00 | UX | Open and use the favourites list from the keyboard (Ctrl+B, ↑/↓, Enter, Del), and put focus back where it was on Esc | Ctrl+B opens the favourites list. ↑/↓ moves through it, Enter opens the folder in the active pane, Del removes the highlighted entry, and Esc closes the list and puts the cursor back where it was. The shortcut also appears in the tooltip, the shortcut list and the README. | 3 files changed, 59 insertions(+), 22 deletions(-) |

## 릴리즈 R4 · 2026-10-21 12:06 KST · 업데이트 1개

| # | 구현 시각(KST) | 분류 | 업데이트 | 사용자에게 달라지는 것 | 변경 |
|---|---|---|---|---|---|
| #26 | 2026-10-21 12:06 | 디자인 | 다크 테마에서 툴팁·파일 정보 말풍선도 어둡게 | 다크 테마에서 버튼에 마우스를 올렸을 때 뜨는 이름 말풍선, 파일 위에 뜨는 정보 말풍선, 트리의 잘린 폴더 이름 말풍선이 흰 상자 대신 어두운 색으로 나옵니다. 라이트 테마로 바꾸면 원래대로 돌아옵니다. | 1 file changed, 16 insertions(+), 1 deletion(-) |

## 릴리즈 R5 · 2026-10-21 12:29 KST · 업데이트 3개

| # | 구현 시각(KST) | 분류 | 업데이트 | 사용자에게 달라지는 것 | 변경 |
|---|---|---|---|---|---|
| #28 | 2026-10-21 12:22 | 기능 | 활성 패널 잠깐 크게 보기 토글 (Ctrl+Shift+M / 패널 머리줄 더블클릭) | Ctrl+Shift+M을 누르거나 패널 머리줄의 빈 곳을 더블클릭하면, 지금 쓰는 패널이 그리드 전체를 차지하도록 커집니다. 한 번 더 누르거나 다른 패널로 옮기면(Ctrl+Tab, Ctrl+1~9, ⇄, 다른 곳에서 폴더 열기) 맞춰 둔 칸 비율 그대로 원래 배치로 돌아옵니다. 크게 보는 동안에는 상태줄에 복원 방법이 표시됩니다. | 3 files changed, 66 insertions(+), 7 deletions(-) |
| #30 | 2026-10-21 12:26 | 기능 | ⇄ 메뉴에서 선택한 파일을 다른 패널로 복사·이동 (Total Commander F5/F6식, 선택 전체가 아니면 보내지 않음) | 파일을 고른 뒤 ⇄ 버튼을 누르면 '선택한 3개 복사 ▸ / 이동 ▸' 아래에 다른 패널들이 나오고, 하나를 고르면 그 패널 폴더로 바로 복사되거나 옮겨진다. 복사하는 동안에도 Grids를 계속 쓸 수 있다. 보낼 수 없는 선택이면 메뉴에 회색 글씨로 이유가 나온다(너무 많음, 일반 파일 아님). | 3 files changed, 115 insertions(+), 14 deletions(-) |
| #27 | 2026-10-21 12:29 | 기능 | 최근 폴더 목록: Alt+↓ 또는 뒤로/앞으로 버튼 오른쪽 클릭 (모든 패널 공통, 재시작·테마·언어 변경에도 순서 유지) | 패널의 ← 또는 → 버튼을 오른쪽 클릭하거나 Alt+↓를 누르면, 모든 패널에서 최근에 드나든 폴더가 최신 순으로 메뉴에 나온다. 하나를 고르면 지금 패널이 그 폴더로 간다. Grids를 다시 켜거나 테마·언어를 바꿔도 목록과 순서가 그대로다. | 3 files changed, 121 insertions(+), 5 deletions(-) |

## 릴리즈 R6 · 2026-10-21 12:44 KST · 업데이트 2개

| # | 구현 시각(KST) | 분류 | 업데이트 | 사용자에게 달라지는 것 | 변경 |
|---|---|---|---|---|---|
| #29 | 2026-10-21 12:31 | 디자인 | 패널 머리줄 왼쪽에 패널 번호 표시 (주소 칸이 좁아지면 생략) | 각 패널 머리줄 맨 왼쪽에 1, 2, 3… 번호가 작게 보이고, 활성 패널에서는 선택 색 위 글자색으로 나옵니다. 패널을 아주 좁게 줄이면 번호가 사라지고 버튼과 주소 칸은 지금과 똑같이 놓입니다. | 4 files changed, 117 insertions(+), 12 deletions(-) |
| #31 | 2026-10-21 12:39 | 기능 | 선택한 폴더의 크기를 상태줄에 백그라운드로 계산해 표시 (작업자 1개·세대 취소·복사 중 일시정지·누락 시 '일부만') | 패널에서 폴더를 고르면 아래 상태줄에 '2개 선택 (3.4 GB…)'처럼 크기가 나오고, 계산이 진행되면서 숫자가 늘다가 끝나면 '…'이 사라집니다. 읽지 못한 항목이 있으면 '(3.4 GB, 일부만)'으로 알려 주고, 계산은 뒤에서 돌기 때문에 그동안 탐색과 붙여넣기가 그대로 됩니다. | 4 files changed, 409 insertions(+), 24 deletions(-) |

## 릴리즈 R7 · 2026-10-21 13:03 KST · 업데이트 3개

| # | 구현 시각(KST) | 분류 | 업데이트 | 사용자에게 달라지는 것 | 변경 |
|---|---|---|---|---|---|
| #39 | 2026-10-21 12:53 | UX | Ctrl+Shift+C: 목록에서 선택한 항목(없으면 현재 폴더)의 전체 경로를 클립보드로 복사 + 상태줄 맨 앞에 확인 표시 | 패널 목록에서 파일을 고르고 Ctrl+Shift+C를 누르면 "C:\…\파일.pdf" 형식의 전체 경로가 복사되고, 상태줄 맨 앞에 '경로 2개 복사함'이 3초 동안 뜹니다. 아무것도 고르지 않았으면 지금 보고 있는 폴더의 경로가 복사됩니다. | 3 files changed, 61 insertions(+), 3 deletions(-) |
| #32 | 2026-10-21 12:54 | 기능 | 주소창에서 상대 경로·파일 열기·명령(cmd, wt, powershell) 실행 | 주소창에 cmd, wt, powershell을 치고 Enter를 누르면 그 패널이 보고 있는 폴더에서 터미널이 열립니다. `..`이나 하위 폴더 이름만 쳐도 이동하고, 파일 경로를 치면 그 파일이 열립니다. 맞는 게 없으면 예전처럼 경고음만 납니다. | 4 files changed, 97 insertions(+), 13 deletions(-) |
| #36 | 2026-10-21 13:02 | 기능 | 폴더를 패널 머리줄에 끌어다 놓으면 그 패널에서 열기 (Q-Dir·Directory Opus식, 효과는 LINK→COPY→NONE 하나만 돌려줌) | 다른 패널, 왼쪽 트리, 탐색기, 바탕화면에서 폴더를 어떤 패널의 윗줄(주소·버튼 줄)로 끌고 가면 그 패널의 윗줄과 테두리가 선택 색으로 바뀝니다. 놓으면 그 패널이 그 폴더로 이동합니다. 파일을 놓으면 그 파일이 든 폴더가 열리고, 원본은 복사도 이동도 되지 않습니다. | 3 files changed, 143 insertions(+), 7 deletions(-) |

## 릴리즈 R8 · 2026-10-21 13:34 KST · 업데이트 2개

| # | 구현 시각(KST) | 분류 | 업데이트 | 사용자에게 달라지는 것 | 변경 |
|---|---|---|---|---|---|
| #46 | 2026-10-21 13:18 | UX | Alt+Shift+방향키로 상하좌우 이웃 패널로 이동 (Windows Terminal·tmux·VS Code 창 이동식, 빈 슬롯은 실제 번호로 매핑) | Alt+Shift+→를 누르면 바로 오른쪽 패널로, Alt+Shift+↓를 누르면 바로 아래 패널로 커서가 옮겨 간다. 패널을 16개 띄워도 번호를 세거나 Ctrl+Tab을 여러 번 누를 필요 없이, 화면에 보이는 방향대로 이동할 수 있다. | 3 files changed, 63 insertions(+) |
| #50 | 2026-10-21 13:20 | 기능 | 빈 곳 더블클릭을 두 번의 '목록 배경' 클릭으로 직접 판별해 상위 폴더로 이동 (MSAA 목록 역할 판정 + 조회 전후 탐색 세대 재확인, 세대는 마지막에 읽음) | Windows 11 자세히 보기를 포함해 모든 패널에서 파일 목록 아래쪽 같은 빈 곳을 더블클릭하면 다시 상위 폴더로 올라갑니다. 파일·그룹 머리·접기 버튼을 누른 경우, Ctrl·Shift를 누른 채 클릭한 경우, 클릭하는 사이 폴더가 바뀐 경우에는 올라가지 않습니다. | 2 files changed, 162 insertions(+), 12 deletions(-) |

## 릴리즈 R9 · 2026-10-21 13:56 KST · 업데이트 1개

| # | 구현 시각(KST) | 분류 | 업데이트 | 사용자에게 달라지는 것 | 변경 |
|---|---|---|---|---|---|
| #49 | 2026-10-21 13:41 | UX | F6 / Shift+F6 cycles focus through the active pane's file list, address bar, filter and folder tree (like Explorer and browsers), and keys Grids handles while the tree has focus no longer navigate a pane twice | Each press of F6 moves focus through the active pane's file list → address bar → filter → left folder tree, and Shift+F6 goes the other way. Arrow keys in the tree then take the active pane straight to that folder. Pressing Ctrl+Tab or Ctrl+number while the tree has focus no longer makes the pane reload its folder and lose its Forward history. | 3 files changed, 57 insertions(+), 3 deletions(-) |

## 릴리즈 R10 · 2026-10-21 14:10 KST · 업데이트 3개

| # | 구현 시각(KST) | 분류 | 업데이트 | 사용자에게 달라지는 것 | 변경 |
|---|---|---|---|---|---|
| #59 | 2026-10-21 14:04 | UX | 폴더 트리 숨기기/보이기 토글 (Ctrl+Shift+E, 보기 메뉴 체크 항목) — 숨긴 동안 사이드바 폭을 덮어쓰지 않음 | Ctrl+Shift+E나 보기 메뉴의 '폴더 트리'로 왼쪽 트리를 숨기면 그 자리까지 패널이 넓어진다. 다시 누르면 전에 맞춰 둔 폭 그대로 트리가 돌아온다. 숨긴 동안 창을 줄이거나 경계를 끌어도, 재시작해도 그 폭은 유지된다. | 3 files changed, 63 insertions(+), 12 deletions(-) |
| #53 | 2026-10-21 14:09 | 디자인 | 단축키 목록을 앱 테마에 맞춘 키캡 안내 창으로 (Ctrl+/ 로도 열기, 길게 눌러도 한 번만 토글, 직접 닫을 때만 포커스 복원) | 보기 메뉴의 '단축키'나 새 단축키 Ctrl+/로 여는 창이 흰 메시지 상자에서 Grids와 같은 색의 안내 창으로 바뀝니다. 키는 키캡 모양으로 한 열에 맞춰 놓이고 설명은 '키보드'와 '마우스·버튼'으로 나뉘며, 다크 테마에서는 어둡게 나옵니다. Ctrl+/를 누르고 있어도 한 번만 열리거나 닫히고, Esc나 Ctrl+/로 닫으면 하던 곳으로 돌아가며, 다른 곳을 누르면 거기에 그대로 머뭅니다. | 3 files changed, 359 insertions(+), 2 deletions(-) |
| #44 | 2026-10-21 14:09 | 디자인 | 패널 주소 칸을 누를 수 있는 경로 조각으로 (Windows 탐색기·Finder 경로 막대식, 실제 탐색 경로 기준, 다 들어가면 접지 않음) | 각 패널 주소 칸이 긴 경로 글자 대신 'C: › Users › Public'처럼 폴더 조각으로 보입니다. 마우스를 올리면 그 조각이 강조되고, 누르면 그 폴더로 바로 갑니다. 다 들어가는 경로는 그대로 다 보입니다. 좁을 때만 앞쪽이 '…'로 접히고, '…'를 누르면 접힌 폴더 목록이 뜹니다. 현재 폴더나 빈 곳을 누르거나 Ctrl+L을 누르면 지금처럼 경로를 입력할 수 있습니다. | 3 files changed, 262 insertions(+), 4 deletions(-) |

## 릴리즈 R11 · 2026-10-21 14:24 KST · 업데이트 2개

| # | 구현 시각(KST) | 분류 | 업데이트 | 사용자에게 달라지는 것 | 변경 |
|---|---|---|---|---|---|
| #55 | 2026-10-21 14:20 | 기능 | 패턴으로 파일 선택·선택 해제·선택 반전 (Num+ / Num− / Num*, Ctrl+Shift+A — Total Commander 'Select group'식, 연 키·실행 Enter는 놓을 때까지 반복 차단, 1만 항목 상한·다시 그리기 묶음) | 파일 목록에서 Num+ 또는 Ctrl+Shift+A를 누르면 목록 위에 작은 입력 칸이 뜹니다. *.jpg처럼 입력하고 Enter를 누르면 맞는 파일이 모두 선택되고, 상태줄에 '*.jpg 12개 선택'이 잠깐 표시됩니다. Num−는 맞는 파일만 선택에서 빼고, Num*는 선택을 반전합니다. 키를 오래 누르고 있어도 입력 칸 내용이 망가지거나 파일이 열리지 않고, 골라 놓은 파일은 ⇄ 버튼으로 바로 옆 패널에 보낼 수 있습니다. | 3 files changed, 310 insertions(+), 9 deletions(-) |
| #51 | 2026-10-21 14:23 | 기능 | 없어진 로컬 폴더는 가장 가까운 상위 폴더로 열고, 그 폴더가 다 열린 뒤 상태줄에 알림 (Total Commander·XYplorer식, '없음'이 확인된 경우만 · 기록은 탐색 전에 · 콜백은 대상 경로로 확인) | 세션·즐겨찾기·최근 목록·명령줄에 있는 로컬 폴더가 지워졌거나 이름이 바뀌었으면, 그 패널은 '내 PC'나 경고음 대신 남아 있는 가장 가까운 상위 폴더를 엽니다. 다 열린 뒤에는 상태줄 맨 앞에 어떤 폴더가 없어서 어디를 열었는지 8초 동안 표시되고, 없어진 폴더는 최근 목록에서 빠집니다. | 3 files changed, 174 insertions(+), 15 deletions(-) |

## 릴리즈 R12 · 2026-10-21 14:37 KST · 업데이트 3개

| # | 구현 시각(KST) | 분류 | 업데이트 | 사용자에게 달라지는 것 | 변경 |
|---|---|---|---|---|---|
| #54 | 2026-10-21 14:33 | 디자인 | 다크 테마 '내 PC'의 남색 묶음 제목을 밝게 직접 그리기 (기본 그리기와 같은 요소만 그리고 나머지는 기본 그리기로) + 변경 전후 글자 대비를 재서 공개를 막는 러너 시험 | 다크 테마에서 '내 PC'를 열면 드라이브 목록 위의 '장치 및 드라이브', '네트워크 위치' 같은 묶음 제목이 짙은 남색 대신 밝은 글자로 또렷하게 보이고, 제목 오른쪽에 가는 구분선이 이어집니다. 다른 폴더에서 묶어 보기를 켰을 때의 제목도 마찬가지로 밝아집니다. | 1 file changed, 103 insertions(+), 1 deletion(-) |
| #71 | 2026-10-21 14:35 | 기능 | F3 빠른 미리보기: 포커스한 파일을 큰 팝업으로 보고 방향키로 다음·이전 (작업자 1개·최신 요청 1개, 픽셀 복사본만 주고받음) | 파일을 고르고 F3을 누르면 창 가운데에 그 파일의 큰 미리보기와 이름·크기가 뜨고, 방향키로 다음·이전 파일로 넘겨 볼 수 있다. Esc나 F3으로 닫으면 보던 목록으로 돌아가고, 미리볼 수 없는 파일은 '미리 볼 수 없습니다'라고 알려 준다. | 3 files changed, 316 insertions(+), 10 deletions(-) |
| #73 | 2026-10-21 14:36 | 디자인 | 패널 색 라벨: 패널 머리줄 오른쪽 클릭으로 패널마다 색 지정 (Windows Terminal 탭 색·Finder 태그·Chrome 탭 그룹식, 세션에 저장, 띠는 버튼 줄 위 여백 안에만) | 패널 윗줄의 빈 곳이나 번호를 오른쪽 클릭하면 색 메뉴가 뜨고, 고른 색이 그 패널 맨 위의 얇은 띠와 번호 칸에 칠해집니다. 색은 다시 켜거나 세션을 불러와도 그대로이고, ⇄ 메뉴에도 '패널 3 (세이지)'처럼 색 이름이 함께 나옵니다. | 3 files changed, 108 insertions(+), 10 deletions(-) |

## 릴리즈 R13 · 2026-10-21 14:45 KST · 업데이트 1개

| # | 구현 시각(KST) | 분류 | 업데이트 | 사용자에게 달라지는 것 | 변경 |
|---|---|---|---|---|---|
| #72 | 2026-10-21 14:42 | UX | 주소창에 폴더 이름 일부를 치면 최근·즐겨찾기 폴더로 이동 (zoxide·브라우저 주소창식) — 탐색 상태를 따로 두지 않고 최근 메뉴·즐겨찾기 팝업과 같은 방식으로 이동 | 패널 주소창(Ctrl+L)에 'down'이나 '보고서'처럼 다녀온 폴더나 즐겨찾기 이름의 일부만 치고 Enter를 누르면 가장 잘 맞는 폴더가 그 패널에 열립니다. 맞는 폴더가 없거나 그 폴더를 열 수 없으면 경고음이 납니다. | 3 files changed, 61 insertions(+), 4 deletions(-) |

## 릴리즈 R14 · 2026-10-21 15:27 KST · 업데이트 1개

| # | 구현 시각(KST) | 분류 | 업데이트 | 사용자에게 달라지는 것 | 변경 |
|---|---|---|---|---|---|
| #78 | 2026-10-21 15:20 | 기능 | 켤 때 창이 화면보다 크거나 화면 밖에 걸치면 작업 영역 안으로 맞추기 (배율이 다른 모니터는 숨긴 채 먼저 옮긴 뒤 최종 배치) | 작은 노트북 화면에서 처음 켜거나, 큰 모니터나 배율이 다른 모니터에서 쓰던 Grids를 다른 화면에서 켜도 창 전체가 화면 안에 들어옵니다. 아래쪽 상태줄과 오른쪽 위 버튼도 가려지지 않습니다. | 2 files changed, 71 insertions(+), 2 deletions(-) |

## 릴리즈 R15 · 2026-10-21 15:41 KST · 업데이트 3개

| # | 구현 시각(KST) | 분류 | 업데이트 | 사용자에게 달라지는 것 | 변경 |
|---|---|---|---|---|---|
| #81 | 2026-10-21 15:33 | 기능 | 보기 메뉴에 '파일 확장명 표시' 추가 (탐색기 보기 > 표시 > 파일 확장명과 같은 Windows 설정) | 보기 메뉴의 '숨김 파일 표시' 아래에 '파일 확장명 표시'가 생깁니다. 누르면 모든 패널에서 report.pdf, setup.exe처럼 확장명이 바로 붙어 보이고, 다시 누르면 숨겨집니다. | 5 files changed, 32 insertions(+), 9 deletions(-) |
| #68 | 2026-10-21 15:34 | 기능 | 설정·폴더를 바뀔 때마다 자동 저장 v2: Windows 재시작·로그오프·강제 종료 뒤에도 마지막 저장 상태로 복원 (원자적 교체·실패 시 5초 뒤 재시도, 기존 ini를 못 읽으면 기본값으로 덮지 않고 다시 시도/취소, 최소화 중에도 최대화 기억 — 브라우저 세션 복원·VS Code hot exit식) | PC를 다시 시작하거나 로그오프해도, Grids가 갑자기 꺼져도, 다음에 열면 마지막으로 자동 저장된 폴더·레이아웃·즐겨찾기·테마와 최대화 상태로 돌아옵니다. 설정 파일이 잠깐 잠겨 있으면 설정을 기본값으로 덮어쓰지 않고 다시 시도할지 묻습니다. | 7 files changed, 127 insertions(+), 16 deletions(-) |
| #70 | 2026-10-21 15:40 | 디자인 | 패널 주소 칸 왼쪽에 현재 폴더 셸 아이콘 표시 (메시지를 처리하며 대기하는 STA 작업자 · 탐색 시작 때 이전 아이콘 지움 · 편집 칸 위치 고정, Windows 탐색기 주소 표시줄식) | 각 패널 주소 칸 맨 앞에 지금 폴더의 아이콘(드라이브, 바탕 화면, 다운로드, 내 PC, 네트워크 등)이 탐색기처럼 붙습니다. 칸이 많아 경로가 잘려도 어느 패널이 어디를 보는지 한눈에 구분되고, 폴더를 옮기면 옛 아이콘은 바로 사라집니다. 느린 네트워크 폴더에서도 화면이 멈추거나 글자가 들썩이지 않습니다. | 9 files changed, 208 insertions(+), 14 deletions(-) |

## 릴리즈 R16 · 2026-10-21 16:03 KST · 업데이트 2개

| # | 구현 시각(KST) | 분류 | 업데이트 | 사용자에게 달라지는 것 | 변경 |
|---|---|---|---|---|---|
| #93 | 2026-10-21 15:52 | UX | 필터 칸 아이콘 클릭·Alt+↓로 파일 종류 필터 메뉴 v2 (사진·동영상·음악·문서·압축·프로그램, 파일 하나를 골랐을 때만 '선택한 파일과 같은 형식', 필터 지우기) | 필터 칸의 돋보기(필터 중에는 강조색 칩)를 누르거나 필터 칸에서 Alt+↓를 누르면 메뉴가 뜹니다. 사진·동영상·음악·문서·압축·프로그램 중 하나를 고르면 그 종류의 파일만(폴더는 그대로) 바로 보입니다. 파일을 하나만 골라 두었다면 '선택한 파일과 같은 형식'도 고를 수 있고, 필터는 같은 메뉴의 '필터 지우기'로 끕니다. | 10 files changed, 107 insertions(+), 7 deletions(-) |
| #89 | 2026-10-21 15:54 | UX | 상단 도구줄 드라이브 버튼 줄 v3 (C: D: E: … 클릭 = 지금 패널에서 그 드라이브 열기, 지금 패널의 드라이브에 선택 색 밑줄, 드라이브가 생기고 없어지면 자동 갱신 · 볼륨 이름 조회 없이 종류만 · 장치 알림은 메시지만 예약하고 끌기·창 크기 조절 중엔 끝날 때 반영 · 시작 때 배치를 중복하지 않음) | 상단 도구줄 가운데에 C:  D:  E: 같은 드라이브 버튼이 생기고, 하나를 누르면 지금 쓰는 패널이 바로 그 드라이브 맨 위로 간다. 지금 패널이 있는 드라이브에는 선택 색 밑줄이 붙어 패널을 바꾸거나 폴더를 옮기면 따라가며, USB를 꽂거나 빼면 버튼이 저절로 생기고 사라진다. | 10 files changed, 155 insertions(+), 2 deletions(-) |

## 릴리즈 R17 · 2026-10-21 16:16 KST · 업데이트 3개

| # | 구현 시각(KST) | 분류 | 업데이트 | 사용자에게 달라지는 것 | 변경 |
|---|---|---|---|---|---|
| #96 | 2026-10-21 16:11 | UX | Global hotkey Win+Alt+E brings Grids to the front from any app v2: press again to minimize, View menu check item; if a dialog is open, that dialog comes forward; the menu shows whether the key is actually registered, so one click retries after a failed start (Windows Terminal quake mode / Everything / Directory Opus style) | From any app, Win+Alt+E brings Grids to the front with the active pane's file list ready for the keyboard; pressing it again minimizes Grids. If a Grids dialog is open, that dialog comes forward, and the key can be turned off or retried from the View menu. | 7 files changed, 100 insertions(+), 2 deletions(-) |
| #91 | 2026-10-21 16:13 | 기능 | 열려 있는 탐색기 창·탭의 폴더를 Grids 패널로 가져오기 v3 (Ctrl+Shift+O · 보기 메뉴) — 이미 열린 폴더의 패널은 그대로 두고, 남은 패널에만 활성 패널부터 새 폴더 배정 | 보기 메뉴의 '탐색기 창의 폴더 가져오기'나 Ctrl+Shift+O를 누르면, 지금 열린 파일 탐색기 창과 탭의 폴더 중 아직 패널에 없는 것만 활성 패널부터 빈 자리에 열린다. 이미 열린 폴더의 패널은 그대로 둔다. 상태줄에 '탐색기 폴더 2개 열기 요청 → 패널 2, 3'처럼 실제 패널 번호가 3초 동안 나오고, 실패하거나 패널이 모자라 못 연 개수도 함께 나온다. | 7 files changed, 122 insertions(+) |
| #97 | 2026-10-21 16:16 | UX | 주소 칸의 › 를 누르면 그 폴더의 하위 폴더 메뉴 v3 (Windows 탐색기 주소 표시줄·VS Code 경로 표시줄식) — 결과는 지금 요청 번호일 때만 게시·수령, 메뉴 위치는 띄우기 직전 현재 배치에서 다시 계산, 창 크기 조절·DPI 변경 시 취소 | 주소 칸 'Users › Public' 사이의 › 에 마우스를 올리면 강조되고, 누르면 Users 안의 폴더 목록이 지금 있는 Public을 굵게 표시한 채 뜹니다. 다른 폴더를 고르면 위로 갔다 내려올 필요 없이 바로 이동합니다. 현재 폴더 뒤의 › 로는 하위 폴더로 바로 들어가고, 느린 네트워크 폴더에서도 창이 멈추지 않습니다. | 10 files changed, 282 insertions(+), 18 deletions(-) |

## 릴리즈 R18 · 2026-10-21 16:28 KST · 업데이트 3개

| # | 구현 시각(KST) | 분류 | 업데이트 | 사용자에게 달라지는 것 | 변경 |
|---|---|---|---|---|---|
| #82 | 2026-10-21 16:19 | UX | 세션 카드에서 바로 삭제·이름 바꾸기 v3: 마우스를 올리면 ✕, 오른쪽 클릭 메뉴, 카드 위에서 이름 고치기 (Chrome 새 탭 바로가기 타일·VS Code 최근 항목 ✕·Directory Opus 레이아웃 관리식) — 덮어쓰지 않는 이름 변경, 실제로 지워졌을 때만 성공 처리, 삭제 확인 중 다른 세션이 열려도 그 세션의 저장 연결은 건드리지 않음 | 세션 불러오기 창에서 카드에 마우스를 올리면 오른쪽 위에 ✕가 나오고, 누르면 그 세션이 지워지며 상태줄에 잠깐 알림이 뜹니다. 카드를 오른쪽 클릭해 '이름 바꾸기'를 고르면 카드 위에서 바로 이름을 고칠 수 있고, 같은 이름이 있으면 덮어쓰지 않고 경고음이 납니다. 지운 자리에는 그동안 가려져 있던 오래된 세션이 나타납니다. | 8 files changed, 337 insertions(+), 28 deletions(-) |
| #166 | 2026-10-21 16:26 | 품질 | 마우스 판정 단위 시험: 경계선 끌기·즐겨찾기 ✕·세션 카드·레이아웃 칸·머리줄 드롭 효과 | 앱 화면과 동작은 그대로다. 대신 앞으로의 변경이 경계선 끌기, 즐겨찾기 ✕, 세션 카드, 레이아웃 칸, 머리줄 드롭 판정을 흐트러뜨리면 공개 전에 시험이 막는다. | 1 file changed, 132 insertions(+) |
| #111 | 2026-10-21 16:28 | 디자인 | 패널 경계선: 마우스를 올리면 잡을 곳을 선택 색 선으로 표시 | 패널 사이나 폴더 트리 옆 경계에 마우스를 올리면 그 자리에 선택 색의 얇은 선이 나타나 어디를 잡아 끌면 되는지 보인다. 끌기 시작하면 같은 색 안내선으로 자연스럽게 이어진다. | 7 files changed, 88 insertions(+), 11 deletions(-) |

## 릴리즈 R19 · 2026-10-21 16:33 KST · 업데이트 3개

| # | 구현 시각(KST) | 분류 | 업데이트 | 사용자에게 달라지는 것 | 변경 |
|---|---|---|---|---|---|
| #98 | 2026-10-21 16:28 | 기능 | 다른 앱에 갔다 돌아오면 키보드 포커스를 떠날 때 있던 곳으로 되돌리기 v3 — 키보드로 돌아오면 바로 되돌리고, 클릭으로 돌아오면 그 클릭이 다 처리된 뒤 포커스가 아무 데도 없을 때만 되돌림 (탐색기·대화 상자 관리자식, 차단형 러너 시험에 '경로 조각 클릭' 포함) | 다른 프로그램에 갔다가 Alt+Tab이나 작업 표시줄로 돌아오면, 목록을 다시 누르지 않아도 방향키·Del·F2·Ctrl+C가 바로 됩니다. 주소창이나 필터에 입력하다 나갔다면 쓰던 글자와 커서 그대로 그 칸으로 돌아오고, 경로 조각을 눌러 돌아오면 그 클릭은 그대로 폴더 이동이 됩니다. | 6 files changed, 93 insertions(+) |
| #99 | 2026-10-21 16:30 | UX | Reorder favourites by dragging or with Alt+↑/↓ in the favourites list (Ctrl+B / ★) v3: dropping outside the list cancels, the drop slot comes from the release point, and only plain Alt (no Ctrl/Shift) reorders | In the favourites list (Ctrl+B or ★), drag a folder to a new spot or press Alt+↑/↓ to move it, and the order is kept after a restart. Letting go outside the list cancels the drag, and Alt+↑ in the list no longer sends the pane behind it up a folder. | 7 files changed, 182 insertions(+), 12 deletions(-) |
| #94 | 2026-10-21 16:33 | UX | Swap the active pane with another pane (Ctrl+Alt+Shift+arrows, or the ⇄ menu's 'Swap places with') — Explorer views are kept, and the pane order is kept in grids.ini so the unsaved-changes dot stays correct after a restart | From the ⇄ menu ('Swap places with ▸ Pane N') or with Ctrl+Alt+Shift+arrow keys, the active pane trades places with another pane. Its folder, back/forward history, selection, scroll position, filter and columns stay exactly as they were. The save button shows unsaved changes until you save or move the panes back, even after restarting Grids. | 10 files changed, 267 insertions(+), 25 deletions(-) |

## 릴리즈 R20 · 2026-10-21 16:47 KST · 업데이트 3개

| # | 구현 시각(KST) | 분류 | 업데이트 | 사용자에게 달라지는 것 | 변경 |
|---|---|---|---|---|---|
| #110 | 2026-10-21 16:34 | 품질 | 필터를 걸어도 ZIP·7z·RAR·CAB 압축 파일이 항상 보이는 버그 수정 v2 (속성 조회가 실패하면 항목 유지) | 필터 칸에 'pdf'처럼 이름을 치면 이제 ZIP·7z·RAR 같은 압축 파일도 이름이 맞을 때만 남고, 진짜 폴더는 전처럼 그대로 보입니다. 필터와 Num+ 패턴 선택이 압축 파일을 똑같이 다룹니다. | 4 files changed, 18 insertions(+), 4 deletions(-) |
| #108 | 2026-10-21 16:40 | 품질 | 테마 색 대비(버튼에 마우스를 올렸을 때·누를 때 포함)·선택 색 이름 6쌍·세션 카드 '몇 분 전' 문구 단위 시험 (동작 변경 없음) | 화면에서 달라지는 것은 없습니다. 앞으로 색이나 테마를 바꾸는 업데이트가 패널 머리줄 버튼(마우스를 올렸을 때·누를 때 포함), 색 라벨 번호, 필터 배지 글자를 지금보다 덜 읽히게 만들거나, 색 이름이 메뉴에서 뒤섞이거나, 세션 카드의 '몇 분 전' 표시가 바뀌면 공개 전에 걸러집니다. | 3 files changed, 134 insertions(+), 8 deletions(-) |
| #107 | 2026-10-21 16:46 | 기능 | 클립보드 그림을 패널에서 Ctrl+V로 붙여 PNG 파일로 저장 v3 — 저장만 하고 패널의 선택·포커스·탐색은 건드리지 않음 (Directory Opus·XYplorer 'Paste image as file'식) | 스크린샷(Win+Shift+S, PrtSc)이나 웹에서 복사한 그림을 패널에서 Ctrl+V 하면 그 폴더에 '클립보드 이미지 날짜 시각.png' 파일이 생긴다. 상태줄에는 저장한 파일 이름이 몇 초 동안 나온다. | 7 files changed, 216 insertions(+), 4 deletions(-) |

## 릴리즈 R21 · 2026-10-21 16:52 KST · 업데이트 3개

| # | 구현 시각(KST) | 분류 | 업데이트 | 사용자에게 달라지는 것 | 변경 |
|---|---|---|---|---|---|
| #168 | 2026-10-21 16:49 | 기능 | 레이아웃을 바꿔도 패널이 화면의 제자리에 남기 v3 (사라지는 칸 기록 → 공통·복귀 칸 예약 → 새 칸 채우기 3단계 배정 · 대체 패널이 아직 안 열렸으면 열린 뒤 한 번만 포커스, Q-Dir·Directory Opus·tmux/VS Code 분할식) | 레이아웃 버튼으로 어느 열에 칸을 더하거나 빼도, 이미 있던 패널은 폴더·이동 기록·필터·선택을 지닌 채 화면의 같은 자리에 남습니다. 여러 번 바꾼 뒤 사라졌던 칸이 다시 생기면 그 칸의 폴더가 원래 자리로 돌아옵니다. 보고 있던 패널의 칸이 없어지면 가장 가까운 패널이 활성이 되고, 그 폴더가 열리는 대로 폴더 트리와 키보드 포커스도 그 패널로 옮겨 갑니다. | 9 files changed, 200 insertions(+), 4 deletions(-) |
| #170 | 2026-10-21 16:50 | UX | 빈 필터 칸에 '이름 필터 · Ctrl+F' 안내 글자 + 화면 낭독기가 주소·필터 칸을 '패널 N 주소'·'패널 N 이름 필터'로 읽음 | 비어 있는 필터 칸에 흐린 글씨로 '이름 필터  Ctrl+F'가 보여서 이 칸이 무엇이고 어떻게 여는지 바로 알 수 있습니다. 화면 낭독기를 쓰면 칸에 들어갈 때 '패널 2 주소', '패널 2 이름 필터'처럼 어느 패널의 어떤 칸인지 읽어 줍니다. | 4 files changed, 55 insertions(+), 4 deletions(-) |
| #169 | 2026-10-21 16:52 | 디자인 | Windows 라이트/다크 설정 따르기 v3: 패널을 다시 만들거나 다시 열지 않고 지금 목록 창에 제자리로 색을 입혔다 걷어냄(이동 기록·필터·선택·스크롤·입력 중 주소 그대로), 보기 메뉴 체크 항목, 새로 설치하면 기본으로 켜짐, 테마 버튼을 누르면 꺼짐 | Windows를 다크 모드로 쓰면 Grids도 처음부터 다크로 열리고, Windows에서 라이트/다크를 바꾸면 Grids도 바로 따라 바뀝니다. 테마가 바뀌어도(테마 버튼을 눌러도) 각 패널의 뒤로/앞으로 기록·필터·선택·스크롤 위치·입력 중인 주소가 그대로 남습니다. 보기 메뉴의 'Windows 라이트·다크 따르기'로 켜고 끌 수 있습니다. | 8 files changed, 129 insertions(+), 11 deletions(-) |

## 릴리즈 R22 · 2026-11-18 09:16 KST · 업데이트 3개

| # | 구현 시각(KST) | 분류 | 업데이트 | 사용자에게 달라지는 것 | 변경 |
|---|---|---|---|---|---|
| #180 | 2026-11-18 08:10 | 품질 | AltGr(Ctrl+Alt)로 치는 글자가 단축키로 먹히지 않게: '글자 입력' 판정 함수 + 빠진 단위 시험 | 폴란드어·US-International·체코·헝가리 등 AltGr을 쓰는 자판에서 파일 이름을 바꾸거나 주소·필터 칸에 ł, ś, ß, ø, [ 같은 글자를 치면 그대로 입력된다. 지금처럼 세션이 저장되거나 포커스가 주소 칸으로 튀거나 숨김 파일 설정이 바뀌지 않는다. 한국어·영어 자판 사용자는 달라지는 것이 없다. | 2 files changed, 25 insertions(+), 1 deletion(-) |
| #181 | 2026-11-18 08:34 | 기능 | 키보드로 정렬 바꾸기: Ctrl+F3 이름 · Ctrl+F5 수정한 날짜 · Ctrl+F6 크기 (다시 누르면 반대 순서, Total Commander식 · Ctrl+F4는 주소창 그대로) | 파일 목록에서 Ctrl+F5를 누르면 그 패널이 수정한 날짜순(최신부터)으로 바뀌고, 다시 누르면 반대 순서가 된다. Ctrl+F3은 이름, Ctrl+F6은 크기이며, 상태줄에 '수정한 날짜순 · 최신부터'처럼 잠깐 표시되고 그 패널은 다른 폴더로 가도 그 정렬을 유지한다. | 7 files changed, 92 insertions(+), 2 deletions(-) |
| #182 | 2026-11-18 08:48 | 품질 | 겹겹이 중첩된 메일(.eml) 미리보기에서 Grids가 꺼지는 문제 수정 + 미리보기 작업자를 요청 단위로 보호 (범위 축소판) | 이상하게 만들어진 메일 파일(.eml)을 F3/Space 미리보기로 지나가도 Grids가 꺼지지 않는다. 어떤 파일의 미리보기가 내부 오류로 실패해도 '이 파일은 미리 볼 수 없습니다' 카드가 나오고, 다음 파일부터는 미리보기가 계속 된다. | 3 files changed, 172 insertions(+), 19 deletions(-) |

## 릴리즈 R23 · 2026-11-18 10:12 KST · 업데이트 3개

| # | 구현 시각(KST) | 분류 | 업데이트 | 사용자에게 달라지는 것 | 변경 |
|---|---|---|---|---|---|
| #189 | 2026-11-18 09:16 | 품질 | 설정 파일 호환 지킴 시험 v2: 모든 설정 왕복·저장 형식 고정·옛 파일/새 버전 파일 읽기 + 확인된 파서 결함 2개 수정(맨 앞 BOM, accent_dark의 '#') | grids.ini를 메모장이나 PowerShell에서 'BOM 있는 UTF-8'로 저장해도 다음 실행 때 창 위치·크기가 그대로입니다. 다크 테마 선택 색을 '#4C8DFF'처럼 '#'을 붙여 적어도 적용됩니다. 그 밖의 화면과 동작은 같습니다. | 1 file changed, 261 insertions(+), 1 deletion(-) |
| #211 | 2026-11-18 09:29 | 품질 | 폴더 트리를 펼친 뒤 다른 패널을 누르면 그 패널이 같은 폴더를 한 번 더 여는 버그 수정 | 폴더 트리에서 ▸로 폴더를 펼치거나 이미 선택된 폴더를 누른 뒤 다른 패널을 클릭해도, 그 패널이 같은 폴더를 다시 열지 않습니다. 하위 폴더로 들어갈 때도 폴더가 두 번 열리지 않습니다. 목록 위치와 방금 누른 파일 선택이 그대로 남습니다. | 3 files changed, 22 insertions(+), 5 deletions(-) |
| #232 | 2026-11-18 09:44 | UX | Insert 키로 항목 하나씩 선택하며 다음 줄로 (Total Commander식 표시) | 파일 목록에서 Insert를 누를 때마다 지금 항목이 선택되거나 선택이 풀리고 커서가 다음 줄로 내려갑니다. Ctrl을 누르고 있지 않아도 ↓와 Insert만으로 떨어진 파일 여러 개를 골라 바로 복사·이동할 수 있고, 상태줄의 선택 개수·크기도 함께 바뀝니다. | 6 files changed, 62 insertions(+) |

## 릴리즈 R24 · 2026-11-18 13:39 KST · 업데이트 1개

| # | 구현 시각(KST) | 분류 | 업데이트 | 사용자에게 달라지는 것 | 변경 |
|---|---|---|---|---|---|
| #230 | 2026-11-18 10:12 | 품질 | 탭 줄 클릭 판정을 그려진 ×와 맞추고 단위 시험으로 묶기 (다른 탭 끝을 누르면 그 탭이 닫히는 버그 · 더블클릭 두 번째 클릭이 탭을 닫는 경우 포함) | 탭을 여러 개 열었을 때 다른 탭은 어디를 누르든, 더블클릭해도 그 탭으로 바뀌기만 한다. 탭은 눈에 보이는 ×(지금 탭), 가운데 클릭, 오른쪽 클릭 메뉴로만 닫힌다. | 3 files changed, 148 insertions(+), 30 deletions(-) |

## 릴리즈 R25 · 2026-11-26 08:33 KST · 업데이트 3개

| # | 구현 시각(KST) | 분류 | 업데이트 | 사용자에게 달라지는 것 | 변경 |
|---|---|---|---|---|---|
| #266 | 2026-11-26 07:45 | 품질 | 탭 이름·세션 카드에 휴지통·네트워크·제어판·라이브러리가 '::{GUID}' 글자로 보이는 결함 수정 + 경로 짧은 이름 단위 시험 | 휴지통·네트워크·제어판·라이브러리·홈·검색 결과를 연 패널의 탭과 세션 카드에 '::{645FF040-…}' 같은 글자 대신 '휴지통', '네트워크', '문서' 같은 이름이 보입니다. 창 제목에 보이는 이름과도 같아집니다. | 4 files changed, 116 insertions(+), 2 deletions(-) |
| #270 | 2026-11-26 07:50 | UX | 단축키 창(Ctrl+/)을 화면 낭독기로 읽고 ↑/↓로 한 줄씩 짚기 | Ctrl+/로 단축키 창을 열면 맨 위 줄이 선택된 채로 열리고, ↑/↓로 한 줄씩 옮겨 가며 지금 보는 줄이 강조됩니다. 화면 낭독기를 쓰면 '단축키 목록, 키보드. Ctrl+Tab, Ctrl+Shift+Tab: 다음 / 이전 패널, 1/52'처럼 줄마다 키와 설명을 소리로 들을 수 있습니다. | 7 files changed, 229 insertions(+), 86 deletions(-) |
| #269 | 2026-11-26 08:06 | 품질 | 디스크 없는 드라이브(빈 카드 리더·DVD·뽑힌 USB 글자)를 건드릴 때 Windows '디스크 없음' 시스템 창 때문에 Grids가 멈추지 않게: 시작할 때 프로세스 오류 모드에 SEM_FAILCRITICALERRORS 설정 | 빈 카드 리더 슬롯이나 디스크 없는 DVD 드라이브를 누를 때, 또는 보던 SD 카드를 뺄 때 'Windows - 디스크 없음' 창이 떠서 Grids 전체가 굳는 일이 없어집니다. 그 드라이브만 열리지 않고(경고음) 다른 패널은 계속 쓸 수 있습니다. | 1 file changed, 37 insertions(+), 1 deletion(-) |

## 릴리즈 R26 · 2026-11-26 09:37 KST · 업데이트 3개

| # | 구현 시각(KST) | 분류 | 업데이트 | 사용자에게 달라지는 것 | 변경 |
|---|---|---|---|---|---|
| #271 | 2026-11-26 08:07 | 기능 | ⇄ 메뉴 '선택한 파일 체크섬(SHA-256) 확인': 클립보드의 해시와 자동 대조, sha256sum 형식으로 복사 | 파일을 고르고 ⇄ 메뉴에서 '선택한 파일의 체크섬(SHA-256)'을 누르면 상태줄에 진행률이 보이고, 끝나면 해시가 상자에 나옵니다. 다운로드 페이지의 SHA-256(또는 MD5·SHA-1)을 먼저 복사해 두면 '✓ 같습니다' 또는 '✗ 맞는 파일이 없습니다'로 바로 알려 주고, '복사'로 결과를 붙여 넣을 수 있습니다. | 7 files changed, 586 insertions(+), 5 deletions(-) |
| #272 | 2026-11-26 09:07 | 기능 | 다른 곳에서 패널 폴더 이름이 바뀌거나 지워지면 패널이 따라가기 (탐색기 창·Total Commander식) | 한 패널이나 탐색기에서 폴더 이름을 바꾸면, 그 폴더나 그 안을 보던 다른 패널의 주소·탭·창 제목이 바로 새 이름으로 바뀝니다. 폴더를 지우면 그 안을 보던 패널은 빈 화면에 멈추지 않고 가장 가까운 상위 폴더로 옮겨지며, 상태줄에 그 사실이 표시됩니다. | 7 files changed, 284 insertions(+), 9 deletions(-) |
| #278 | 2026-11-26 09:09 | UX | Windows 고대비 테마를 켜면 Grids가 직접 그리는 부분도 그 색을 따르기 | Windows에서 고대비 테마(예: 사막, 수평선, 야간 하늘)를 켜면 Grids의 도구줄·패널 머리줄·탭·상태줄·팝업·메뉴·제목 표시줄이 Windows가 고른 색과 선택 색으로 바뀌고, 실행 중에 켜거나 꺼도 바로 반영됩니다. 고대비를 끄면 저장해 둔 라이트/다크 테마와 선택 색으로 그대로 돌아갑니다. | 9 files changed, 232 insertions(+), 44 deletions(-) |
