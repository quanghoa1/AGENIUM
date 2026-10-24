@echo off
rem rc.exe stand-in for builds without the MSVC SDK (GNU toolchain). mxc-sdk's build script calls
rem   rc /nologo /fo<out.res> <in.rc>
rem to embed version info into its own helper binaries, which Agenium never builds or ships.
rem Uses MinGW windres when available, otherwise writes an empty resource file.
set "OUT=%~2"
set "OUT=%OUT:~3%"
where windres.exe >nul 2>nul
if %ERRORLEVEL%==0 (
  windres.exe -i "%~3" -O res -o "%OUT%"
  exit /b %ERRORLEVEL%
)
type nul > "%OUT%"
