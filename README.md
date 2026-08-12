# WinSlimCenter (Tauri)

Catálogo de aplicaciones para Windows. Migrado desde CustomTkinter a **Tauri 2 + HTML/CSS/JS**.

## Desarrollo

```bash
cd center-app
npm install
npm run dev
```

## Build

Desde la raíz del repo:

```powershell
.\build-tauri.ps1
```

O:

```bash
cd center-app
npm run build
```

Salida: `src-tauri/target/release/Center.exe` y instaladores en `bundle/msi` y `bundle/nsis`.

## Datos

- Catálogo: `apps.json` (junto al EXE o en recursos)
- Diagnóstico: escribe `/logs` y pulsa Enter en el buscador para abrir el registro de la sesión. Los últimos 20 registros se conservan en `%LOCALAPPDATA%\CenterApps\logs` para poder investigar instalaciones, desinstalaciones y aperturas después de cerrar la tienda.
- Instalaciones: `%LOCALAPPDATA%\CenterApps\`
- Ajustes: `%LOCALAPPDATA%\CenterApps\settings.json`
