@echo off
chcp 65001 >nul
cd /d "%~dp0"
title 계장 개념설계 IO 산출 - 준비 중
echo 계장 개념설계 IO 산출 - 필요한 패키지를 확인합니다...
python -m pip install -q -r requirements.txt
if errorlevel 1 (
  echo [오류] 파이썬 또는 pip를 찾을 수 없습니다. Python 3.10 이상을 설치해 주세요.
  pause
  exit /b 1
)
rem v13.4: 콘솔 없이 앱 창으로 실행 (창을 닫으면 프로그램도 종료). 문제가 있으면 projects\_app.log 확인
rem 콘솔을 보면서 실행하려면:  실행.bat debug
if /i "%~1"=="debug" goto debug
where pythonw >nul 2>nul
if errorlevel 1 goto debug
start "" pythonw -m pid_io.app
exit /b 0
:debug
python -m pid_io.app
pause
