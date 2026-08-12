@echo off
setlocal
cd /d "%~dp0"

rem Keep Cargo's incremental cache outside the repository. This leaves the
rem source tree clean while preserving fast subsequent builds.
set "WINSLIM_TARGET_DIR=%LOCALAPPDATA%\WinSlimCenter\cargo-target"
set "CARGO_TARGET_DIR=%WINSLIM_TARGET_DIR%"

if not exist "node_modules\@tauri-apps\" (
  echo [INFO] Instalando dependencias del proyecto...
  call npm install
  if errorlevel 1 goto :err_npm
)

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
echo [INFO] Iniciando compilacion de WinSlimCenter...
call npm run build
if errorlevel 1 goto :err_build

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

:err_npm
echo.
echo [ERROR] Fallo al instalar las dependencias con npm install.
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
