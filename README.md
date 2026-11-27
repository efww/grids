# pid_io 자료함 (inbox 브랜치)

회사 PC ↔ Claude 작업 사이에 파일을 주고받는 공간입니다. 코드(main / 작업 브랜치)와 따로 둡니다.

| 폴더 | 용도 |
|---|---|
| `받은자료/` | 사용자 → Claude. 회사 가이드 HTML, Task1 패키지, MD, 케이블 데이터시트, 타피컬 서포트 드로잉, 전기 3D JSON 등 |
| `보낸자료/` | Claude → 사용자. 결과 묶음 등 |

## 올리는 법 (웹)
1. https://github.com/khmremotedesktop-bot/grids/tree/inbox/받은자료 열기 (로그인 필요)
2. 오른쪽 위 **Add file → Upload files** → 파일 끌어다 놓기
3. 아래 **Commit changes** 누르기 (메시지는 아무거나)
4. Claude에게 "올렸어"라고 말하기

- 웹 업로드는 **파일당 25 MB**까지. 더 크면 7-Zip으로 20 MB씩 분할(.zip/.z01 …)해서 올리면 됩니다.
- 받은 파일은 Claude가 프로젝트 폴더(`pid_io/projects/…` 또는 `pid_io/protocols/`)로 옮겨 원본 그대로 보관합니다. 여기 파일은 지우지 않습니다.

## 받는 법
https://github.com/khmremotedesktop-bot/grids/tree/inbox/보낸자료 에서 파일을 눌러 **Download** (또는 브랜치 전체: https://github.com/khmremotedesktop-bot/grids/archive/refs/heads/inbox.zip)
