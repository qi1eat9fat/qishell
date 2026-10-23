@echo off
REM 把 release 产物整理成一个可直接拷走的便携文件夹。
REM 用法：scripts\package_portable.bat [目标目录]
REM 默认输出到 dist\QiShell-portable。
setlocal enabledelayedexpansion

set "ROOT=%~dp0.."
for %%I in ("%ROOT%") do set "ROOT=%%~fI"
if "%~1"=="" (set "OUT=%ROOT%\dist\QiShell-portable") else (set "OUT=%~1")

if not defined CARGO_TARGET_DIR (set "TARGET=%ROOT%\target\release") else (set "TARGET=%CARGO_TARGET_DIR%\release")
set "SRC=%TARGET%\qishell.exe"

if not exist "%SRC%" (
    echo 找不到 %SRC%，先跑 cargo build --release 1>&2
    exit /b 1
)

if exist "%OUT%" rmdir /s /q "%OUT%"
mkdir "%OUT%"
copy /y "%SRC%" "%OUT%\qishell.exe" >nul
copy /y "%ROOT%\LICENSE" "%OUT%\LICENSE.txt" >nul

REM data\ 不放任何东西：应用第一次启动自己建，这样「拷走整个文件夹」不会
REM 把开发机的数据库、密码、known_hosts 一起带出去。
echo 已生成 %OUT%
dir /b "%OUT%"
