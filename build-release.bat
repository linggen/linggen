@echo off
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

REM Create distribution directory
echo.
echo Creating distribution package...
if not exist dist\linggen mkdir dist\linggen
copy backend\target\release\api.exe dist\linggen\linggen.exe
xcopy /E /I frontend\dist dist\linggen\frontend
if not exist dist\linggen\data mkdir dist\linggen\data

REM Create README for users
echo Linggen RAG - Local Semantic Search > dist\linggen\README.txt
echo. >> dist\linggen\README.txt
echo To run: >> dist\linggen\README.txt
echo   linggen.exe >> dist\linggen\README.txt
echo. >> dist\linggen\README.txt
echo The application will start on http://localhost:3000 >> dist\linggen\README.txt
echo Open your browser and navigate to that URL. >> dist\linggen\README.txt
echo. >> dist\linggen\README.txt
echo Data will be stored in the .\data directory. >> dist\linggen\README.txt

REM Create run script
echo @echo off > dist\linggen\run.bat
echo echo Starting Linggen... >> dist\linggen\run.bat
echo echo Open your browser to: http://localhost:3000 >> dist\linggen\run.bat
echo start http://localhost:3000 >> dist\linggen\run.bat
echo linggen.exe >> dist\linggen\run.bat

echo.
echo Build complete!
echo Distribution package: dist\linggen\
echo.
echo To test:
echo   cd dist\linggen
echo   run.bat

