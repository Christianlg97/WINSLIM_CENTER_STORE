@echo off
setlocal
cd /d "%~dp0"

rem Keep Cargo's incremental cache outside the repository. This leaves the
rem source tree clean while preserving fast subsequent builds.
set "WINSLIM_TARGET_DIR=%LOCALAPPDATA%\WinSlimCenter\cargo-target"
set "CARGO_TARGET_DIR=%WINSLIM_TARGET_DIR%"

rem The CLI is resolved after the C++ environment is ready: the last fallback
rem builds tauri-cli from source and needs the linker itself.

where link.exe >nul 2>&1
if not errorlevel 1 goto :do_build

echo [INFO] Inicializando entorno de compilacion de Visual Studio C++...
if exist "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" (
  call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat"
  goto :do_build
)
if exist "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat" (
  call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"
  goto :do_build
)
if exist "C:\Program Files\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" (
  call "C:\Program Files\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat"
  goto :do_build
)

where link.exe >nul 2>&1
if errorlevel 1 goto :install_vctools

:do_build
rem ---------------------------------------------------------------------------
rem Resolve a working Tauri CLI.
rem
rem `npx tauri` alone is not enough: an interrupted or partially extracted
rem install leaves node_modules\@tauri-apps\cli in place but without its
rem tauri.js entry point and with an empty node_modules\.bin, and npx then fails
rem with "could not determine executable to run". Checking that the folder
rem exists therefore proves nothing; what matters is the entry point itself.
rem Each step below repairs one more layer, ending with a CLI built by Cargo,
rem which needs no Node at all because the frontend is static.
rem ---------------------------------------------------------------------------
set "TAURI_JS=%~dp0node_modules\@tauri-apps\cli\tauri.js"
set "TAURI_MODE="

if exist "%TAURI_JS%" goto :cli_node

echo [INFO] Instalando dependencias del proyecto...
call npm install
if exist "%TAURI_JS%" goto :cli_node

echo [ADVERTENCIA] La CLI de Tauri no quedo utilizable; reinstalandola...
call npm install --no-save "@tauri-apps/cli@^2"
if exist "%TAURI_JS%" goto :cli_node

echo [ADVERTENCIA] npm no pudo proporcionar la CLI. Probando con Cargo...
where cargo-tauri >nul 2>&1
if not errorlevel 1 goto :cli_cargo

where cargo >nul 2>&1
if errorlevel 1 goto :err_cli

echo [INFO] Instalando tauri-cli con Cargo (puede tardar varios minutos)...
call cargo install tauri-cli --version "^2.0.0" --locked
where cargo-tauri >nul 2>&1
if not errorlevel 1 goto :cli_cargo
goto :err_cli

:cli_node
set "TAURI_MODE=node"
echo [INFO] CLI de Tauri: npm (@tauri-apps/cli)
goto :cli_ready

:cli_cargo
set "TAURI_MODE=cargo"
echo [INFO] CLI de Tauri: cargo-tauri
goto :cli_ready

:cli_ready
echo [INFO] Iniciando compilacion de WinSlimCenter...
if "%TAURI_MODE%"=="cargo" goto :build_with_cargo

call node "%TAURI_JS%" build
if errorlevel 1 goto :err_build
goto :build_done

:build_with_cargo
call cargo tauri build
if errorlevel 1 goto :err_build

:build_done
echo.
echo [INFO] Limpiando y copiando únicamente WinSlimCenter.exe a la carpeta Build...
if exist "%~dp0Build\" rmdir /s /q "%~dp0Build"
mkdir "%~dp0Build"

if exist "%WINSLIM_TARGET_DIR%\release\WinSlimCenter.exe" (
  copy /y "%WINSLIM_TARGET_DIR%\release\WinSlimCenter.exe" "%~dp0Build\" >nul
)

echo.
echo [EXITO] Compilacion completada correctamente.
echo Los archivos resultantes se han guardado en: "%~dp0Build"
echo.
dir /b "%~dp0Build"
echo.
pause
exit /b 0

:install_vctools
echo.
echo [ADVERTENCIA] No se detecto el enlazador C++ (link.exe missing).
echo [INFO] Intentando instalar Visual Studio C++ Build Tools con winget...
echo.
winget install --id Microsoft.VisualStudio.2022.BuildTools --override "--passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended" --accept-package-agreements --accept-source-agreements
if errorlevel 1 goto :err_winget

echo.
echo [EXITO] C++ Build Tools instalado. Vuelve a ejecutar este script para compilar.
echo.
pause
exit /b 0

:err_cli
echo.
echo [ERROR] No se pudo obtener una CLI de Tauri utilizable.
echo Se intento, en este orden:
echo   1. usar node_modules\@tauri-apps\cli\tauri.js
echo   2. npm install
echo   3. npm install --no-save "@tauri-apps/cli@^2"
echo   4. cargo-tauri ya instalado
echo   5. cargo install tauri-cli --version "^2.0.0"
echo.
echo Comprueba que Node.js y/o Rust estan instalados y accesibles en el PATH,
echo y que hay conexion a internet. Para reparar a mano:
echo   rmdir /s /q node_modules ^&^& npm install
echo.
pause
exit /b 1

:err_winget
echo.
echo [ERROR] No se pudo completar la instalacion automatica de C++ Build Tools.
echo Abriendo la pagina oficial de descarga en tu navegador...
start https://visualstudio.microsoft.com/visual-cpp-build-tools/
echo.
pause
exit /b 1

:err_build
echo.
echo [ERROR] La compilacion ha fallado. Revisa los mensajes superiores.
echo.
pause
exit /b 1
