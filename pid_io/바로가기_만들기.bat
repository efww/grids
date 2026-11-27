@echo off
chcp 65001 >nul
cd /d "%~dp0"
rem 바탕화면에 '계장 IO 산출' 바로가기를 만듭니다 (아이콘 포함, 창 최소화로 실행)
powershell -NoProfile -ExecutionPolicy Bypass -Command "$s=(New-Object -ComObject WScript.Shell).CreateShortcut([Environment]::GetFolderPath('Desktop')+'\계장 IO 산출.lnk'); $s.TargetPath='%~dp0실행.bat'; $s.WorkingDirectory='%~dp0'; $s.IconLocation='%~dp0pid_io\app\static\pid_io.ico'; $s.WindowStyle=7; $s.Save()"
echo 바탕화면에 '계장 IO 산출' 바로가기를 만들었습니다.
pause
