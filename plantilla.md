# Plantilla de aplicación para `src-tauri/apps.json`

Este documento sirve como guía para agregar aplicaciones a mano en el catálogo del proyecto.

## Estado de la plantilla

Sí, la plantilla ya está lista y alineada con el comportamiento actual del backend. Para que la app pueda guardar una entrada correctamente, los campos mínimos que se validan son:

- `id`
- `name`
- `source_type`

El resto de campos son recomendados para que la interfaz y el catálogo se muestren de forma completa.

## Estructura básica

`src-tauri/apps.json` es un array JSON de objetos. Cada objeto describe una aplicación y tiene esta forma:

```json
{
  "id": "mi_app_unica",
  "name": "Mi App",
  "description": "Descripción corta de la app.",
  "version": "latest",
  "author": "Nombre del autor o compañía",
  "category": "Utilidades",
  "section": "Utilidades",
  "featured": false,
  "source_type": "wget",
  "download_url": "https://example.com/miapp-installer.exe",
  "download_filename": "MiAppInstaller.exe",
  "accent_color": "#1a73e8",
  "icon_url": "https://example.com/miapp-icon.png",
  "icon_background": "#000000",
  "icon_padding": 9,
  "detect_names": [
    "MiApp",
    "Mi App"
  ]
}
```

## Campos recomendados

- `id`: identificador único en minúsculas y con `_` (por ejemplo, `notepad_plus_plus`).
- `name`: nombre visible de la app.
- `description`: descripción breve para mostrar en la UI.
- `version`: versión mostrada; puede ser `latest` si no se conoce versión fija.
- `author`: autor o marca de la app.
- `category`: categoría usada para clasificar (por ejemplo, `Desarrollo`, `Juegos`, `Utilidades`).
- `section`: sección en la UI; normalmente coincide con `category` o con secciones como `Destacados`, `Juegos`, `Desarrollo`, `Utilidades`, `Multimedia`, `Productividad`, `Social y Comunicación`.
- `featured`: `true` si la app debe aparecer en destacados.
- `accent_color`: color hexadecimal para la tarjeta de la app.
- `icon_url`: URL de icono específico. Si no existe, el código puede usar el avatar de GitHub o Clearbit.
- `icon_background`: fondo hexadecimal específico del icono (por ejemplo, `#000000` para logotipos diseñados sobre negro).
- `icon_padding`: ajuste interior del icono en porcentaje, entre `0` y `35`; usa `9` como valor normal y redúcelo para agrandar el logotipo.
- `download_filename`: nombre de archivo a usar al descargar.
- `detect_names`: lista de nombres que se usan para detectar si la app ya está instalada.

## Tipos de descarga (`source_type`)

El backend actual acepta estos valores:

- `wget`
- `github_release`
- `github_repo`
- `direct` (equivalente a una descarga directa por URL)

### `wget`
Para descargas directas.

- `download_url`: URL directa al instalador.
- `download_filename`: opcional; nombre del archivo cuando se descarga.

```json
{
  "source_type": "wget",
  "download_url": "https://example.com/miapp.exe",
  "download_filename": "MiAppSetup.exe"
}
```

### `github_release`
Para apps que se extraen desde GitHub Releases.

- `github_repo`: repositorio en formato `owner/repo`.
- `asset_pattern`: patrón glob que identifica el activo de la release.

```json
{
  "source_type": "github_release",
  "github_repo": "usuario/repositorio",
  "asset_pattern": "MiApp-*-win64.exe"
}
```

### `github_repo`
Para descargar el contenido de un repositorio GitHub en una rama concreta.

- `github_repo`: repositorio en formato `owner/repo`.
- `branch`: rama que se descargará; por defecto es `main`.

```json
{
  "source_type": "github_repo",
  "github_repo": "usuario/repositorio",
  "branch": "main"
}
```

## Campos opcionales extra

- `installer_args`: argumentos extra para instalar el ejecutable o MSI. Puede ser una cadena o un array de cadenas.
- `branch`: rama a descargar cuando `source_type` es `github_repo`.

Ejemplo:

```json
{
  "source_type": "wget",
  "download_url": "https://example.com/instalador.exe",
  "installer_args": ["--silent", "--agree"]
}
```

## Cómo agregar una app

1. Abre `src-tauri/apps.json`.
2. Inserta un nuevo objeto dentro del array, separado por comas.
3. Asegúrate de que `id` sea único.
4. Define `source_type` y los campos correspondientes.
5. Guarda el archivo.

## Ejemplo completo

```json
{
  "id": "notepad_plus_plus",
  "name": "Notepad++",
  "description": "Editor de texto ligero con soporte para plugins y múltiples lenguajes.",
  "version": "latest",
  "author": "Notepad++ Team",
  "category": "Desarrollo",
  "section": "Desarrollo",
  "featured": true,
  "source_type": "github_release",
  "github_repo": "notepad-plus-plus/notepad-plus-plus",
  "asset_pattern": "npp.*.Installer.x64.exe",
  "accent_color": "#009688",
  "icon_url": "https://github.com/notepad-plus-plus.png?size=128",
  "detect_names": [
    "Notepad++"
  ]
}
```

## Notas

- Si no colocas `icon_url`, el código intentará usar el avatar de GitHub cuando `github_repo` está disponible.
- Para descargas desde Google Drive u otros enlaces especiales, usa `download_filename` para dar un nombre coherente al archivo.
- Mantén `section` consistente con los filtros del menú de la app.
- Para instaladores `.exe` o `.msi`, puedes añadir `installer_args` para automatizar la instalación silenciosa.
