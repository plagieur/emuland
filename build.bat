@echo off
rem Builds emuland from source and puts emuland.exe next to this file.
rem Needs Rust (https://rustup.rs). Run it from anywhere: double-click or "build.bat".
setlocal
cd /d "%~dp0"
set "PATH=%PATH%;%USERPROFILE%\.cargo\bin"

where cargo >nul 2>nul
if errorlevel 1 (
    echo Rust was not found. Install it from https://rustup.rs then run this file again.
    pause
    exit /b 1
)

if /i "%~1"=="test" (
    echo Running the tests...
    cargo test --release || goto :fail
)

echo Building emuland in release mode...
cargo build --release -p emuland || goto :fail

copy /y "target\release\emuland.exe" "emuland.exe" >nul || goto :fail
echo.
echo Done: %~dp0emuland.exe
if /i not "%~1"=="nopause" pause
exit /b 0

:fail
echo.
echo The build failed.
pause
exit /b 1
