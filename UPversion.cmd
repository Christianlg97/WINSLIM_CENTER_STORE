@echo off
setlocal EnableExtensions
cd /d "%~dp0"

rem Cambia la version del proyecto en un solo sitio.
rem
rem src-tauri\Cargo.toml es la unica fuente de la version: el compilador la
rem incrusta en el binario, el backend se la pasa a la interfaz y de ahi sale
rem tanto la insignia de la barra inferior como la comparacion con la release
rem publicada en GitHub. Cambiarla aqui y compilar es todo lo que hace falta.

set "MANIFEST=src-tauri\Cargo.toml"

echo.
echo  ===========================================================
echo    WinSlimCenter  -  Cambiar version del proyecto
echo  ===========================================================
echo.

if not exist "%MANIFEST%" (
  echo  [ERROR] No se encuentra %MANIFEST%
  echo  Ejecuta este archivo desde la raiz del proyecto.
  echo.
  pause
  exit /b 1
)

rem --- Version actual ---------------------------------------------------------
set "CURRENT="
for /f "usebackq delims=" %%V in (`powershell -NoProfile -ExecutionPolicy Bypass -Command "$q=[char]34; $t=[System.IO.File]::ReadAllText('src-tauri\Cargo.toml'); $m=[regex]::Match($t, '(?m)^\s*version\s*=\s*'+$q+'(.+?)'+$q); if ($m.Success) { $m.Groups[1].Value }"`) do set "CURRENT=%%V"

if not defined CURRENT (
  echo  [ERROR] No se pudo leer la version actual de %MANIFEST%
  echo.
  pause
  exit /b 1
)

echo    Version actual:  %CURRENT%
echo.
echo    Escribe la nueva version y pulsa Enter.
echo    Formato: numeros y puntos, por ejemplo  1.8.0
echo.

set "NEW="
set /p "NEW=   Nueva version: "

if not defined NEW goto :cancelado

rem --- Validacion -------------------------------------------------------------
set "VALID="
for /f "usebackq delims=" %%R in (`powershell -NoProfile -ExecutionPolicy Bypass -Command "if ($env:NEW -match '^[0-9]+\.[0-9]+(\.[0-9]+)?$') { 'OK' } else { 'BAD' }"`) do set "VALID=%%R"

if not "%VALID%"=="OK" (
  echo.
  echo  [ERROR] "%NEW%" no es una version valida.
  echo  Se esperaba algo como 1.8.0 o 2.0
  echo.
  pause
  exit /b 1
)

if "%NEW%"=="%CURRENT%" (
  echo.
  echo  [AVISO] El proyecto ya esta en la version %CURRENT%. No se cambia nada.
  echo.
  pause
  exit /b 0
)

echo.
echo    %CURRENT%   -^>   %NEW%
echo.
set "CONFIRM="
set /p "CONFIRM=   Confirmas el cambio? (S/N): "
if /i not "%CONFIRM%"=="S" goto :cancelado

rem --- Escritura --------------------------------------------------------------
rem Solo la primera linea que empieza por `version`, que es la del bloque
rem [package]. Las de las dependencias van dentro de llaves y nunca al principio
rem de la linea, asi que no pueden coincidir.
rem
rem Se sustituye sobre el texto completo, no linea a linea: asi el resto del
rem archivo sale byte a byte como entro, incluidos sus finales de linea. Volver
rem a escribirlo entero convertia todo el manifiesto a CRLF y ensuciaba el
rem control de versiones con un cambio que nadie habia pedido.
set "RESULT="
for /f "usebackq delims=" %%W in (`powershell -NoProfile -ExecutionPolicy Bypass -Command "$q=[char]34; $p='src-tauri\Cargo.toml'; $t=[System.IO.File]::ReadAllText($p); $rx=[regex]::new('(?m)^(\s*version\s*=\s*)'+$q+'.+?'+$q); if($rx.IsMatch($t)){ $n=$rx.Replace($t, '${1}'+$q+$env:NEW+$q, 1); [System.IO.File]::WriteAllText($p,$n,(New-Object System.Text.UTF8Encoding($false))); 'CHANGED' } else { 'NOTFOUND' }"`) do set "RESULT=%%W"

if not "%RESULT%"=="CHANGED" (
  echo.
  echo  [ERROR] No se encontro la linea de version en %MANIFEST%
  echo  No se ha modificado nada.
  echo.
  pause
  exit /b 1
)

rem --- Comprobacion -----------------------------------------------------------
set "WROTE="
for /f "usebackq delims=" %%V in (`powershell -NoProfile -ExecutionPolicy Bypass -Command "$q=[char]34; $t=[System.IO.File]::ReadAllText('src-tauri\Cargo.toml'); $m=[regex]::Match($t, '(?m)^\s*version\s*=\s*'+$q+'(.+?)'+$q); if ($m.Success) { $m.Groups[1].Value }"`) do set "WROTE=%%V"

if not "%WROTE%"=="%NEW%" (
  echo.
  echo  [ERROR] La comprobacion fallo: el archivo dice "%WROTE%".
  echo.
  pause
  exit /b 1
)

echo.
echo  ===========================================================
echo    Hecho.  Version del proyecto:  %WROTE%
echo  ===========================================================
echo.
echo    Siguiente paso:  compila con build.cmd
echo    La tienda mostrara v%WROTE% en la barra inferior.
echo.
echo    Al publicar la release en GitHub, escribe el numero en
echo    las notas:   ### WinSlimCenter %WROTE% ###
echo    De ahi lo lee la tienda para avisar de actualizaciones.
echo.
pause
exit /b 0

:cancelado
echo.
echo  Cancelado. La version sigue siendo %CURRENT%.
echo.
pause
exit /b 0
