@echo off
setlocal
set "ARGS=%*"
if defined ARGS (
    set "ARGS=%ARGS:--target=x86_64-pc-windows-gnullvm=--target=x86_64-w64-windows-gnu%"
)
clang.exe %ARGS%
exit /b %ERRORLEVEL%
