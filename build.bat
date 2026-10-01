@echo off
pushd "%~dp0" || exit /b 1
cargo build --release
set "build_exit_code=%errorlevel%"
popd
pause
exit /b %build_exit_code%
