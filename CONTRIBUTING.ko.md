# 기여하기

## 소스에서 빌드

Rust GNU 툴체인(`x86_64-pc-windows-gnu`)과 MSYS2의 MinGW 도구(`C:\msys64\mingw64\bin`의 `windres`, `dlltool`, `as`)가 필요합니다.

```
build.cmd
```

`target\release\maguro.exe`를 빌드해 `build\maguro.exe`로 복사합니다. 탐색기에 보이는 exe 아이콘은 `assets/maguro-exe.ico`(리소스 그룹 1)입니다. 제목 표시줄과 정보 창은 라이트 `app.ico`(그룹 3)와 다크 `assets/maguro-dark-bg.ico`(그룹 2)를 그대로 씁니다.

## 릴리스

GitHub 릴리스 자산 두 개를 올리고, zip으로 묶지 않습니다.

- `maguro.exe`
- `maguro.exe.sha256`

exe와 함께 `maguro.exe.sha256`을 항상 올립니다. 파일은 sha256sum 한 줄, `<64 hex>  maguro.exe`(공백 두 칸)입니다. 파일 안의 이름이 `maguro.exe`가 되도록 exe가 있는 폴더에서 만듭니다.

```
sha256sum maguro.exe > maguro.exe.sha256
```

앱 안 업데이트는 두 파일을 받아 SHA-256이 같을 때만 exe를 바꿉니다. `maguro.exe.sha256`이 없거나 zip만 있는 릴리스는 안내만 하고 설치하지 않습니다.

