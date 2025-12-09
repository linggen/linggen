@echo off
echo Building Linggen CLI...

cd "%~dp0backend"
cargo build --release --bin linggen

echo.
echo Build complete!
echo.
echo Binary location: %CD%\target\release\linggen.exe
echo.
echo To use the CLI, add the binary location to your PATH or copy it to a directory already in your PATH.
echo.
echo Test the installation with:
echo   linggen --help
