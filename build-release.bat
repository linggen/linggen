@echo off
echo Building RememberMe for distribution...

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
if not exist dist\rememberme mkdir dist\rememberme
copy backend\target\release\api.exe dist\rememberme\rememberme.exe
xcopy /E /I frontend\dist dist\rememberme\frontend
if not exist dist\rememberme\data mkdir dist\rememberme\data

REM Create README for users
echo RememberMe RAG - Local Semantic Search > dist\rememberme\README.txt
echo. >> dist\rememberme\README.txt
echo To run: >> dist\rememberme\README.txt
echo   rememberme.exe >> dist\rememberme\README.txt
echo. >> dist\rememberme\README.txt
echo The application will start on http://localhost:3000 >> dist\rememberme\README.txt
echo Open your browser and navigate to that URL. >> dist\rememberme\README.txt
echo. >> dist\rememberme\README.txt
echo Data will be stored in the .\data directory. >> dist\rememberme\README.txt

REM Create run script
echo @echo off > dist\rememberme\run.bat
echo echo Starting RememberMe... >> dist\rememberme\run.bat
echo echo Open your browser to: http://localhost:3000 >> dist\rememberme\run.bat
echo start http://localhost:3000 >> dist\rememberme\run.bat
echo rememberme.exe >> dist\rememberme\run.bat

echo.
echo Build complete!
echo Distribution package: dist\rememberme\
echo.
echo To test:
echo   cd dist\rememberme
echo   run.bat

