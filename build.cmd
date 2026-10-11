@echo off
setlocal
cd /d "%~dp0"

rem Keep Cargo's incremental cache outside the repository. This leaves the
rem source tree clean while preserving fast subsequent builds.
set "WINSLIM_TARGET_DIR=%LOCALAPPDATA%\WinSlimCenter\cargo-target"
set "CARGO_TARGET_DIR=%WINSLIM_TARGET_DIR%"

rem The CLI is resolved after the C++ environment is ready: the last fallback
rem builds tauri-cli from source and needs the linker itself.

rem ---------------------------------------------------------------------------
rem The C++ toolchain: Rust links with Microsoft's linker, so nothing builds
rem without it. Visual Studio is found through vswhere, which knows every
rem edition and version (Build Tools or the full IDE, 2022 or later) instead of
rem three fixed 2022 paths. A missing one is installed and the build carries on:
rem asking for a second run only postponed the same failure.
rem
rem Every install below goes through scratch\install_toolchain.ps1, which tries
rem winget first and the vendor's official installer when winget is missing or
rem fails, so a machine without winget is no longer a dead end.
rem ---------------------------------------------------------------------------
set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
set "INSTALL_TOOLCHAIN=%~dp0scratch\install_toolchain.ps1"

where link.exe >nul 2>&1
if not errorlevel 1 goto :do_build

call :find_vcvars
if defined VCVARS goto :vc_init

echo.
echo [ADVERTENCIA] No se detecto el compilador C++ de Visual Studio (link.exe).
echo [INFO] Instalando Visual Studio C++ Build Tools...
echo.
call :install_component vctools
call :find_vcvars
if not defined VCVARS goto :err_vctools

:vc_init
echo [INFO] Inicializando entorno de compilacion de Visual Studio C++...
rem vcvars64 calls vswhere by name. Build Tools installed by winget leave its
rem folder off the PATH, and the script then printed "vswhere.exe no se
rem reconoce" in the middle of a build that was working fine.
for %%D in ("%VSWHERE%") do set "PATH=%%~dpD;%PATH%"
call "%VCVARS%" >nul
where link.exe >nul 2>&1
if errorlevel 1 goto :err_vctools

:do_build
rem ---------------------------------------------------------------------------
rem Resolve the toolchains the build needs.
rem
rem winget publishes a new PATH to the processes started after it, never to the
rem one that called it, so a toolchain installed by the fallbacks below would
rem still look missing until the script was run a second time. Their default
rem locations go on the PATH by hand, which also picks up an installation that
rem was already on disk but never reached the environment.
rem ---------------------------------------------------------------------------
if exist "%ProgramFiles%\nodejs\node.exe" set "PATH=%ProgramFiles%\nodejs;%PATH%"
if exist "%USERPROFILE%\.cargo\bin\cargo.exe" set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"

rem Tauri compiles the application itself with Cargo, so a missing Rust is
rem fatal. Node only minifies the frontend and runs the npm CLI: without it the
rem build still finishes through cargo-tauri, so it is repaired but not required.
where cargo >nul 2>&1
if errorlevel 1 goto :install_rust

:rust_ready
where node.exe >nul 2>&1
if not errorlevel 1 goto :node_ready

echo.
echo [ADVERTENCIA] No se detecto Node.js.
echo [INFO] Instalando Node.js LTS...
echo.
call :install_component node
if exist "%ProgramFiles%\nodejs\node.exe" set "PATH=%ProgramFiles%\nodejs;%PATH%"
where node.exe >nul 2>&1
if errorlevel 1 goto :node_missing
echo [INFO] Node.js disponible.
goto :node_ready

:node_missing
echo [ADVERTENCIA] Node.js sigue sin estar disponible; se compilara con cargo-tauri
echo               y el frontend ira sin minificar.

:node_ready
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

rem That entry point is JavaScript, so a complete folder proves nothing on a
rem machine without Node: this mode used to be chosen on the strength of the
rem file alone and the build then died with "node no se reconoce".
where node.exe >nul 2>&1
if errorlevel 1 goto :cli_from_cargo

if exist "%TAURI_JS%" goto :cli_node

echo [INFO] Instalando dependencias del proyecto...
call npm install
if exist "%TAURI_JS%" goto :cli_node

echo [ADVERTENCIA] La CLI de Tauri no quedo utilizable; reinstalandola...
call npm install --no-save "@tauri-apps/cli@^2"
if exist "%TAURI_JS%" goto :cli_node

echo [ADVERTENCIA] npm no pudo proporcionar la CLI. Probando con Cargo...

:cli_from_cargo
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
set "TAURI_PROD_CONFIG="
where node.exe >nul 2>&1
if errorlevel 1 goto :frontend_ready

rem El minificador es una dependencia de desarrollo aparte de la CLI: sin ella
rem build_frontend.mjs avisa y copia el JS y el CSS sin minificar, que compila
rem igual pero pesa mas de lo necesario.
if not exist "%~dp0node_modules\esbuild\package.json" (
  echo [INFO] Falta esbuild; instalando dependencias de desarrollo...
  call npm install
)

echo [INFO] Preparando frontend optimizado...
call node "%~dp0scratch\build_frontend.mjs"
if errorlevel 1 goto :err_build
set "TAURI_PROD_CONFIG=%~dp0src-tauri\tauri.prod.conf.json"

:frontend_ready
echo [INFO] Iniciando compilacion de WinSlimCenter...
if "%TAURI_MODE%"=="cargo" goto :build_with_cargo

if defined TAURI_PROD_CONFIG (
  call node "%TAURI_JS%" build --config "%TAURI_PROD_CONFIG%"
) else (
  call node "%TAURI_JS%" build
)
if errorlevel 1 goto :err_build
goto :build_done

:build_with_cargo
if defined TAURI_PROD_CONFIG (
  call cargo tauri build --config "%TAURI_PROD_CONFIG%"
) else (
  echo [ADVERTENCIA] Node.js no esta disponible; se compilara el frontend fuente sin minificar.
  call cargo tauri build
)
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

rem ---------------------------------------------------------------------------
rem Subroutines, reached only through `call`.
rem ---------------------------------------------------------------------------

rem Installs one toolchain component (vctools, rust or node).
:install_component
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%INSTALL_TOOLCHAIN%" -Component %1
exit /b %errorlevel%

rem Sets VCVARS to vcvars64.bat of the newest Visual Studio that has the C++
rem compiler, or leaves it empty. `call` keeps cmd from stripping the quotes
rem around a path that contains "(x86)".
:find_vcvars
set "VCVARS="
set "VS_PATH="
if not exist "%VSWHERE%" exit /b 0
for /f "usebackq delims=" %%P in (`call "%VSWHERE%" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do set "VS_PATH=%%P"
if not defined VS_PATH exit /b 0
if exist "%VS_PATH%\VC\Auxiliary\Build\vcvars64.bat" set "VCVARS=%VS_PATH%\VC\Auxiliary\Build\vcvars64.bat"
exit /b 0

:install_rust
echo.
echo [ADVERTENCIA] No se detecto Cargo (la cadena de herramientas de Rust).
echo [INFO] Instalando Rust...
echo.
call :install_component rust
if exist "%USERPROFILE%\.cargo\bin" set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
where rustup >nul 2>&1
if errorlevel 1 goto :err_rust

rem winget deja rustup instalado, no necesariamente una cadena activa, y la que
rem sirve aqui es la MSVC: es la que usa el enlazador de Visual Studio que este
rem script acaba de preparar.
echo [INFO] Preparando la cadena estable MSVC...
call rustup default stable-x86_64-pc-windows-msvc

where cargo >nul 2>&1
if errorlevel 1 goto :err_rust
echo [INFO] Rust disponible.
goto :rust_ready

:err_rust
echo.
echo [ERROR] No se pudo obtener Cargo, y Tauri no puede compilar sin Rust.
echo Instalalo a mano y vuelve a ejecutar este script:
echo   winget install --id Rustlang.Rustup
echo.
echo Abriendo la pagina oficial en tu navegador...
start https://rustup.rs/
echo.
pause
exit /b 1

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

:err_vctools
echo.
echo [ERROR] No se pudo preparar el compilador C++ de Visual Studio.
echo Se intento con winget y con el instalador oficial de Microsoft.
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
