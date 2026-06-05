@echo off
setlocal EnableExtensions
cd /d "%~dp0\.."

set "LOG_FILE=%CD%\target\mahjuro-startup-profile.log"
set "RUST_LOG=mahjuro=debug"
set "MAHJURO_STARTUP_PROFILE=1"
set "MAHJURO_GPU_MEM_PROFILE=1"
if not defined MAHJURO_VALIDATE_OFFLINE_BAKES set "MAHJURO_VALIDATE_OFFLINE_BAKES=0"
set "MAHJURO_LOG_FILE=%LOG_FILE%"

echo Building release binary...
cargo build --release --bin mahjuro
if errorlevel 1 (
    echo Build failed.
    exit /b 1
)

if not exist "target\release\mahjuro.exe" (
    echo Error: target\release\mahjuro.exe not found after build.
    exit /b 1
)

if exist "%LOG_FILE%" del /f /q "%LOG_FILE%"

echo.
echo Release startup profile run
echo   Binary:  target\release\mahjuro.exe --no-steam
echo   Log:     %LOG_FILE%
echo   RUST_LOG=%RUST_LOG%
echo   MAHJURO_STARTUP_PROFILE=1
echo   MAHJURO_GPU_MEM_PROFILE=1
echo   MAHJURO_VALIDATE_OFFLINE_BAKES=%MAHJURO_VALIDATE_OFFLINE_BAKES%
echo.
echo Release builds have no console on Windows; logs go to the file above.
echo A second window will tail the log live. Reach the main menu (or gameplay)
echo before quitting so async GPU upload timings are recorded.
echo.

start "Mahjuro startup log" powershell -NoProfile -Command ^
    "Write-Host 'Tailing %LOG_FILE%' -ForegroundColor Cyan; Get-Content -LiteralPath '%LOG_FILE%' -Wait"

echo Launching...
"target\release\mahjuro.exe" --no-steam %*
set "EXIT_CODE=%ERRORLEVEL%"

echo.
echo Game exited with code %EXIT_CODE%.
echo.
echo Full log:
echo ========================================================================
type "%LOG_FILE%" 2>nul
echo ========================================================================
echo Log saved at: %LOG_FILE%

exit /b %EXIT_CODE%
