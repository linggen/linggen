@echo off
REM Navigate to project root (parent of deploy folder)
cd /d "%~dp0\.."

echo Building Linggen for distribution...

REM Build frontend
echo.
echo Building frontend...
cd frontend
call npm ci
call npm run build
cd ..

REM Build backend
echo.
echo Building backend (release mode)...
cd backend
cargo build --release --package api
cd ..

REM Create distribution directory (standalone server for Windows)
set DIST_DIR=dist\standalone-windows-x64
echo.
echo Creating distribution package in %DIST_DIR%...
if exist %DIST_DIR% rmdir /S /Q %DIST_DIR%
mkdir %DIST_DIR%
copy backend\target\release\api.exe %DIST_DIR%\linggen.exe
xcopy /E /I frontend\dist %DIST_DIR%\frontend
mkdir %DIST_DIR%\data

REM Create README for users
echo Linggen RAG - Local Semantic Search (Standalone Server) > %DIST_DIR%\README.txt
echo. >> %DIST_DIR%\README.txt
echo To run: >> %DIST_DIR%\README.txt
echo   linggen.exe >> %DIST_DIR%\README.txt
echo. >> %DIST_DIR%\README.txt
echo The application will start on http://localhost:7000 >> %DIST_DIR%\README.txt
echo Open your browser and navigate to that URL. >> %DIST_DIR%\README.txt
echo. >> %DIST_DIR%\README.txt
echo Data will be stored in the .\data directory. >> %DIST_DIR%\README.txt

REM Create run script
echo @echo off > %DIST_DIR%\run.bat
echo echo Starting Linggen... >> %DIST_DIR%\run.bat
echo echo Open your browser to: http://localhost:7000 >> %DIST_DIR%\run.bat
echo start http://localhost:7000 >> %DIST_DIR%\run.bat
echo linggen.exe >> %DIST_DIR%\run.bat

echo.
echo Build complete!
echo Distribution package: %DIST_DIR%\
echo.
echo To test:
echo   cd %DIST_DIR%
echo   run.bat

