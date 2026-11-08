@echo off
rem JanScope.exe (launcher) build with Visual Studio (MSVC). Keep this file ASCII only.
rem   build.bat [version]   e.g. build.bat 0.2.0
setlocal
set VER=%~1
if "%VER%"=="" set VER=dev
where cl >nul 2>nul
if not errorlevel 1 goto build
set VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe
for /f "usebackq tokens=*" %%i in (`"%VSWHERE%" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do set VS=%%i
if "%VS%"=="" (echo Visual Studio C++ tools not found & exit /b 1)
call "%VS%\VC\Auxiliary\Build\vcvars64.bat" >nul || exit /b 1
:build
cd /d "%~dp0"
rc /nologo /c65001 /fo JanScope.res JanScope.rc || exit /b 1
cl /nologo /O2 /EHsc /utf-8 /std:c++17 /W3 /DJANSCOPE_VERSION="\"%VER%\"" JanScope.cpp JanScope.res /Fe:JanScope.exe /link /SUBSYSTEM:WINDOWS user32.lib gdi32.lib comctl32.lib shell32.lib shlwapi.lib comdlg32.lib || exit /b 1
del /q JanScope.obj JanScope.res 2>nul
echo built: %~dp0JanScope.exe
