@echo off
chcp 65001 >nul
cd /d "%~dp0"
echo 계장 개념설계 IO 산출 - 필요한 패키지를 확인합니다...
python -m pip install -q -r requirements.txt
if errorlevel 1 (
  echo [오류] 파이썬 또는 pip를 찾을 수 없습니다. Python 3.10 이상을 설치해 주세요.
  pause
  exit /b 1
)
echo 브라우저가 열립니다. 이 창을 닫으면 프로그램이 종료됩니다.
python -m pid_io.app
pause
