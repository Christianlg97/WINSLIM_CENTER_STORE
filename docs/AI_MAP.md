# Mapa de WinSlimCenter para IA

Verificado: 2026-10-20. `R` = `src-tauri/src/`, `F` = `src/`. Los simbolos son anclas para `rg -n`, no numeros de linea. Lee la fila de tu tarea y su contrato; amplia solo si aparecen dependencias nuevas.

## Enrutador: donde empezar

| Tarea / sintoma | Archivo y simbolos | Consumidor / comprobacion |
|---|---|---|
| Anadir o corregir una app del catalogo | `src-tauri/apps.json` + icono en F`assets/catalog/` | Seccion "Fichas del catalogo"; tests de R`store.rs`; validacion de R`lib.rs`: `save_catalog` |
| Arranque, estado global, comandos IPC | R`lib.rs`: `run`, `AppState`, `generate_handler!`, `get_bootstrap`, `schedule_startup_cleanup` | F`main.js`: `invoke(...)`, `listen(...)`, `refreshStore`, `finishStartupInBackground` |
| Carga, variantes, ajustes y editor del catalogo | R`store.rs`: `load_catalog`, `apply_variant`, `Settings`, `load_repo_entries`; R`paths.rs`: `resolve_apps_json`; R`lib.rs`: `save_catalog`, `reload_catalog`, `VISIBLE_CATALOG_SECTIONS` | F`main.js`: `validateCatalog`, `openCatalogEditor`, `replaceCatalog` |
| Instalar: tuberia por origen | R`installer.rs`: `do_install`, `InstallOutcome`, `acquire_install_stage`; R`lib.rs`: `install_app`, `acquire_download_slot`, `confirm_installed` | Evento `install-finished`; F`main.js`: `installApp`, `showPackageOperationModal` |
| Ejecutar setups: argumentos silenciosos, ventana, codigos de salida, UAC | R`installer.rs`: `run_installer_over`, `run_installer_in_background`, `app_installer_args`, `installer_runs_hidden`, `detect_installer_family`, `interpret_installer_exit`, `run_elevated_installer_and_wait`, `exit_code_requires_elevation` | Tests de R`installer.rs` (`custom_installer_args_override_defaults`, `velopack_setups_run_silently_...`) |
| ZIP/7z, instalador dentro de archivo, portables | R`installer.rs`: `extract_archive_async`, `inspect_extracted_payload_async`, `wrapped_installer`, `swap_into_install_path_async`, `protect_portable_before_setup`, `is_portable_payload`; R`portable.rs`: `backup`, `prepare_update` | R`lib.rs`: `portable_data_info`; F`main.js`: `portableDataChoices`, `portableDataMessage` |
| Descargas, cola, pausa, GitHub Releases, enlaces de pagina | R`download.rs`: `DownloadManager`, `download_url`, `download_with_curl`, `github_latest_release_asset`, `select_github_asset_url`, `RELEASE_CACHE`, `link_on_page`; R`lib.rs`: `DOWNLOAD_SLOTS`, `pause_download`, `cancel_all` | Evento `downloads-changed`; F`main.js`: `renderDlPanel`, `syncDownloadTasks` |
| WinGet: instalar, buscar, upgrades, desinstalar | R`installer.rs`: `install_with_winget`, `winget_installer_url`, `search_winget`, `parse_winget_search`, `winget_available_updates`, `parse_winget_upgrades`, `uninstall_with_winget`, `uninstall_with_winget_as_user` | R`detect.rs`: `scan_winget_packages`; R`lib.rs`: `winget_search` |
| Detectar instaladas, version, ruta de lanzamiento | R`detect.rs`: `build_statuses`, `velopack_install`, `velopack_status`, `scan_installed_programs`, `match_system_app`, `names_match`, `scan_start_apps`, `StartAppIndex`, `system_launch_candidates`, `AppStatus`; R`lib.rs`: `rebuild_statuses_with_progress`, `probe_app_status`, `refresh_statuses` | F`main.js`: `replaceStatuses`, `appStatus`, `actionButtons` |
| Buscar actualizaciones / Actualizar todo | R`lib.rs`: `check_updates`, `check_updates_once`, `settle_pending_restart`; R`detect.rs`: `is_newer`, `catalog_update_available` | F`main.js`: `scanForUpdates`, `updateEverything`, `startBulkUpdate`, `finishBulkItem` |
| Seleccionar fichas e instalar en cola | F`main.js`: `state.selection`, `isQueueable`, `toggleSelection`, `syncCardSelection`, `renderQueueDock`, `installSelection`, `queueReviewHtml`; `startBulkUpdate(items, "install")`, `bulkStateLabel`; F`styles.css`: `.card-select`, `.queue-dock`, `.queue-review` | `scratch/test_review_regressions.cjs` (`queuedInstall`, `queueEligibility`); arnes `?headless` |
| Desinstalar y limpiar restos | R`lib.rs`: `uninstall_app`, `windows_forgets_the_app`, `finish_uninstall`, `confirm_uninstalled`, `run_uninstall_cleanup`; R`installer.rs`: `uninstall_system_app`, `split_registered_command`, `uninstall_from_install_path`, `remove_managed_install`, `cleanup_shortcuts_for_install_target`, `cleanup_declared_residual_paths`, `validate_removable_install_dir`; R`residue.rs`: `AppIdentity`, `purge_stale_index_entries` | F`main.js`: `uninstallApp`; R`detect.rs`: `is_a_different_operation` |
| Abrir apps, cerrar las que bloquean, procesos | R`lib.rs`: `launch_app_internal`, `running_blocker`, `close_running_app`; R`installer.rs`: `resolve_launchable_path`, `launch_app`; R`process.rs`: `stop_application_at`, `close_application_at`, `run_hidden_and_wait`, `run_elevated_and_wait`, `launch_as_interactive_user` | F`main.js`: `launchApp`, `showRunningAppModal`, `blockingRunningApp`, `showElevationFallbackModal` |
| Microsoft Store (buscar, instalar, actualizar, quitar) | R`msstore.rs`: `search`, `details`, `packages`, `install_package`, `install_official_store_product`, `installed_apps`, `scan_updates`, `uninstall`, `verify_digest`; R`msstore_soap.rs`; R`msix.rs`; R`lib.rs`: `msstore_*` | Evento `msstore-install-finished`; F`main.js`: funciones `msStore*`, `openMsStoreModal`, `installMsStoreProduct` |
| Chocolatey y busqueda en repositorios | R`choco.rs`: `search`, `ensure_available`, `install`; R`lib.rs`: `choco_search`, `choco_status`, `AppState::detection_catalog` | F`main.js`: `REPOS`, `repoCardHtml`, `installRepoPackage`, `runInlineRepoSearch`; `scratch/test_repo_detection.cjs` |
| Apps web, menu Inicio, escritorio, PATH | R`webapp.rs`: `install`, `uninstall`, `shortcut_of`, `launch`; R`start_menu.rs`: `publish`, `withdraw`, `publish_if_missing`; R`env_path.rs`: `requested_entries`, `add_entries`; R`lib.rs`: `published_shortcut_name`, `desktop_shortcut_name` | Campos `start_menu_shortcut`, `desktop_shortcut`, `path_entries` |
| Descargas de TechPowerUp | R`techpowerup.rs`: `plan`, `mirror_url`, `parse_mirrors`; R`installer.rs`: `download_from_techpowerup`, `verify_techpowerup_digest` | `techpowerup_slug` |
| UI: render, tarjetas, modales, barra de estado, tema | F`main.js`: `state`, `derived`, `renderContentNow`, `renderSidebarNow`, `cardHtml`, `actionButtons`, `openAppModal`, `openModal`, `showConfirmModal`, `setStatus`, `bindShellDelegation`, `applyTheme`, `openThemePicker`; F`styles.css`; F`index.html` | `npm run test:ui`; `scratch/ui_harness.html` (`?headless`) |
| Iconos del catalogo | F`main.js`: `resolveIconUrl`, `iconCandidates`, `renderAvatar`; `scratch/fetch_catalog_icons.mjs`, `optimize_catalog_icons.mjs`, `resize_icons.ps1` | Campos `icon_url`, `icon_background`, `icon_padding`, `icon_fit`, `icon_position` |
| Autoactualizacion de la tienda, version, notas | R`lib.rs`: `check_store_update`, `update_center_app`, `prepare_and_launch_self_update`, `GITHUB_REPO`, `get_release_notes`; R`download.rs`: `github_release_notes` | Evento `self-update-progress`; F`main.js`: `checkStoreUpdate`, `startStoreUpdate`, `showStoreUpdateModal`; `UPversion.cmd` |
| Logs y comandos de la barra (`/logs`, `/woa`) | R`logger.rs`: `init`, `log`, `safe_url`; R`lib.rs`: `open_logs`, `write_log`, `run_woa` | F`main.js`: `clientLog` |
| Compilacion y empaquetado | `build.cmd`, `scratch/build_frontend.mjs`, `scratch/install_toolchain.ps1`, `src-tauri/tauri.conf.json`, `tauri.prod.conf.json`, `windows-app-manifest.xml`, `Cargo.toml` | `npm run frontend:check`, `npm run test:rust` |

## Dependencias

```text
main.js --invoke--> lib.rs (comandos) --> store/paths (catalogo, ajustes, installed.json)
lib.rs::install_app -> installer::do_install -> download (GitHub/URL/curl/TechPowerUp) -> extraer | setup (exe/msi) | portable -> confirm_installed -> detect
                                             -> install_with_winget | choco::install | webapp::install
lib.rs::uninstall_app -> winget -> uninstall_system_app (registro) -> uninstall_from_install_path -> residue -> finish_uninstall -> detect
lib.rs::check_updates -> download::github_latest_release_asset + installer::winget_available_updates -> detect::is_newer
detect -> registro Uninstall (HKLM/HKCU/WOW6432Node) + menu Inicio (scan_start_apps) + winget list + installed.json
msstore -> apps.microsoft.com + msstore_soap (entrega de Windows Update) -> msix/Add-AppxPackage | setup clasico
process -> procesos ocultos, UAC (ShellExecuteExW), Explorer como usuario interactivo
lib.rs --emit--> downloads-changed, install-finished, background-progress, msstore-install-finished, self-update-progress --> main.js
```

## Contratos que preservar

**Catalogo.** `resolve_apps_json`: `apps.json` junto al exe o en `resources` > `src-tauri/apps.json` via `CARGO_MANIFEST_DIR` (solo existe en el equipo que compilo) > copia embebida (`include_str!` en R`store.rs`). Una ficha con `variants` es una sola app: `apply_variant` aplica `overrides` de la opcion elegida (recordada en `Settings::variants`) y elimina el bloque antes de que nada mas la lea. Toda instalacion `winget`/`choco` recuerda su ficha en `repository-apps.json`; `detection_catalog` suma al catalogo las que no estan en el (resultados de busqueda), nunca dentro de `apps.json`. `save_catalog` valida: id unico, `id`/`name`/`source_type`/`section` presentes, seccion visible, campo propio del origen, `residual_paths` y `winget_dependencies` bien formados.

**Tuberia de instalacion.** `install_app` decide `current_version`/`installed_at` (carpeta registrada) y encola en `DownloadManager`; `do_install` resuelve el origen: `webapp`, `choco` y `winget` terminan dentro de su rama (WinGet con respaldo a la URL del manifiesto); `wget`/`direct`, `github_release` (respaldo `download_url` y luego WinGet), `github_repo` y `techpowerup` descargan a `Downloads/WinSlimCenter/<id>/`. Maximo 4 descargas (`DOWNLOAD_SLOTS`) y una sola fase de instalacion a la vez (`acquire_install_stage`). Si en una actualizacion GitHub no tiene version mas nueva, devuelve `InstallOutcome::unchanged`. La copia anterior no se toca hasta tener la nueva descargada y extraida.

**Setups del sistema.** `.msi` -> `msiexec /i ... /qn /norestart` + log verboso en `CenterApps/logs/msi-*.log`. `.exe` sin `installer_args` = asistente visible (no se adivinan interruptores); con argumentos corre oculto. `run_installer_over` detiene lo que corre en `installed_at` antes y rearranca servicios despues. Codigos: 0/1641/3010 exito; cancelacion segun `InstallerFamily`; 740 o codigos de elevacion -> reintento UAC. Un setup devuelve `system_managed`: no se registra en `installed.json`, Windows es la verdad y `confirm_installed` espera hasta 30 s a que la deteccion lo vea. Los setups por usuario corren con el token elevado del mismo usuario (HKCU y `%LOCALAPPDATA%` correctos).

**Deteccion.** Orden en `build_statuses`: `component` (ruta conocida) -> `webapp` (acceso directo) -> `installed.json` (gestionadas por la tienda) -> Velopack (`velopack_id`: version de `%LOCALAPPDATA%\<id>\current\sq.version`, desinstalador `Update.exe --uninstall --silent`; no depende de la entrada del registro, que su setup puede no escribir) -> registro `Uninstall` por `name`+`detect_names` (difuso, `match_quality`; `detect_exclude_names` aparta hermanos como "Antigravity IDE") -> menu Inicio (`start_app_names`) -> `winget list`. El registro aporta version, `InstallLocation`, `QuietUninstallString` (preferido) y `UninstallString` solo si es otra operacion. Ruta de lanzamiento: `known_launch_paths` -> candidatos del registro con `launch_executable` -> ejecutables indexados -> carpeta con su nombre -> menu Inicio; nunca `*update*`, `*setup*`, `*unins*`.

**Actualizaciones.** `github_release` instalada: la ultima release (API `releases/latest`, si no casa ninguna, las 20 ultimas) manda frente a WinGet; cache en memoria por los 60 req/h anonimos y reutiliza la ultima conocida si GitHub falla. El resto: `winget upgrade` por `winget_id`; `version` fija en la ficha solo cuenta si es numerica. `is_newer` compara numeros (ignora `v`); "latest"/"nightly" = desconocido. `github_release_is_update`: con version instalada desconocida ("latest" o vacia) nunca hay actualizacion; sin numeros en ambos, compara el texto. Actualizar = `install_app` con `force_update`.

**Desinstalacion.** `origin=system`: WinGet si hay `winget_id` (si falla por ambito de usuario elevado, se repite como usuario interactivo via Explorer) -> comando registrado (`msiexec` pasa a `/x ... /qn`) -> comando completo -> carpeta (`uninstall_from_install_path`, solo rutas validadas por `validate_removable_install_dir`/`is_protected_installation_root`) -> limpieza de marcas. Un exito solo cuenta si Windows deja de listarla (`windows_forgets_the_app`, 6 s). Despues: accesos directos, `residual_paths` (solo declaradas) y `confirm_uninstalled`. `origin=center`: `remove_managed_install` con politica de datos del portable. Nunca borres datos adivinados por nombre.

**Portables.** Viven en `%LOCALAPPDATA%\CenterApps\<id>`. Antes de actualizar o desinstalar, el usuario elige conservar (copia completa en `%LOCALAPPDATA%\WinSlimCenter-Backups`; si la copia falla, se aborta) o eliminar. Las actualizaciones recuperan archivos personales anadidos/modificados sin sustituir ejecutables/DLL nuevos.

**UI.** Sin framework: `state` + `derived`, render agrupado por `requestAnimationFrame` (`renderContent`/`renderContentNow`), eventos delegados en `bindShellDelegation`. Busqueda: catalogo -> Microsoft Store -> WinGet -> Chocolatey, cada una en su subseccion sin esperar a las demas; los paquetes ya cubiertos por el catalogo se descuentan. Los nombres de eventos y comandos son contrato entre `lib.rs` y `main.js`.

**Cola de instalacion.** `state.selection` (Set, orden de eleccion = orden de la cola, solo en memoria) admite lo que `isQueueable` acepta: ficha del catalogo, ni `web` ni `component`, sin instalar, sin tarea y fuera de la tanda en marcha; `pruneSelection` saca lo demas en cada render. Se marca con el circulo `.card-select` (visible al pasar o con algo elegido), Ctrl+clic en la tarjeta o el boton de la ficha; Escape sin dialogo vacia. La barra `#queue-dock` se crea por JS dentro de `.main`. `installSelection` repasa (edicion recordada o por defecto, datos de portables) y llama a `startBulkUpdate(items, "install")`: misma tanda y dialogo que "Actualizar todo", con `forceUpdate`/`closeRunning` a false y textos via `bulk.mode`/`bulkStateLabel`. Hay una sola tanda a la vez (`state.bulkUpdate`). Para probar la UI con el panel del navegador oculto, usa el arnes con `?headless` y dispara clics por JS: las capturas pueden salir desfasadas.

## Fichas del catalogo

Obligatorios siempre: `id` (snake_case unico), `name`, `description` (espanol), `version` ("latest" salvo version fija), `author`, `category`, `section`, `featured`, `source_type`, `icon_url`. Secciones: Juegos, Emuladores, Navegadores, Desarrollo, IA, Utilidades, Multimedia, Productividad, Social y Comunicacion; la categoria debe estar en la tabla de `catalog_categories_are_assigned_to_their_expected_sections`.

| `source_type` | Campos propios | Notas |
|---|---|---|
| `winget` (~155) | `winget_id`, `winget_source` (`winget`/`msstore`), `winget_dependencies` | El preferido si el paquete existe en WinGet |
| `github_release` (~50) | `github_repo`, `asset_pattern` (wildmatch sin mayusculas, `*`), `download_url` de reserva (mejor `.../releases/latest/download/<asset>`) | Sin patron: setup x64 > archivo. `winget_id` opcional = otro respaldo y otra via de desinstalar |
| `wget` / `direct` | `download_url` o `download_page` + `download_link_pattern` | Para URLs que cambian, la pagina se lee al descargar |
| `webapp` | `web_url` | Acceso directo que abre el navegador; sin descarga |
| `web` | `web_url` | Boton "Obtener": solo abre la web oficial |
| `component` | `component_of`, `known_launch_paths` | Programa incluido en una suite (Word en Microsoft 365) |
| `techpowerup` | `techpowerup_slug` | Espejos con verificacion SHA-256 |
| `github_repo` / `choco` | `github_repo`+`branch` / `choco_id` | `choco` solo llega desde la busqueda |

Instalacion: `download_filename`; `installer_args` (lista o texto; Inno `/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /SP-`, NSIS `/S`, Velopack `--silent`, MSI por defecto); `installer_in_archive` (`true` o nombre del setup dentro del zip); `installer_cancel_exit_codes`; `portable` (+ `start_menu_shortcut`, `desktop_shortcut`); `path_entries`; `variants` (`default`, `options[{id, overrides}]`). Deteccion/lanzamiento: `velopack_id` (packId de Velopack; carpeta bajo `%LOCALAPPDATA%`), `detect_names`, `detect_exclude_names`, `start_app_names`, `launch_executable`, `known_launch_paths`, `launchable: false` (suites). Restos: `residual_paths` (`%VAR%` y `{install_dir}`, solo bajo LOCALAPPDATA/APPDATA/PROGRAMDATA/Program Files; se omiten si el usuario conserva datos). UI: `accent_color`, `icon_background`, `icon_padding`, `icon_fit` (`contain`), `icon_position` (`left`), `console_tags` (filtro de Emuladores), `web_url`.

Receta setup de GitHub (p. ej. `scskiller`, Velopack): `github_release` + `asset_pattern` exacto + `download_url` `latest/download` + `installer_args` silenciosos + `velopack_id` si es Velopack (el `<id>` de su `.nuspec`) + `launch_executable` + `detect_names` con el `DisplayName` real + `residual_paths` para la carpeta de ajustes que su desinstalador deja (`%LOCALAPPDATA%\SCSKiller`; con desinstalador registrado no hay dialogo de conservar datos, asi que siempre se borra). Comprueba antes el `DisplayName`, el `QuietUninstallString` y los argumentos del setup leyendo su release/manifiesto, sin ejecutarlo. Icono local de 256 px como maximo.

## Archivos y datos de ejecucion

| Ruta | Uso |
|---|---|
| `src-tauri/Cargo.toml` | Version unica de la app (`UPversion.cmd`), dependencias |
| `src-tauri/tauri.conf.json`, `tauri.prod.conf.json` | Ventana 1200x780 (min 1040x680), `frontendDist` `../src` en desarrollo y `../dist` en produccion |
| `src-tauri/windows-app-manifest.xml`, `build.rs` | Manifiesto propio (el de Tauri + `requireAdministrator`) incrustado por `build.rs` |
| `src-tauri/certs/` | Raiz de Microsoft Update para el canal SOAP de la Store |
| `scratch/` | Herramientas del proyecto (build del frontend, tests UI, auditorias de catalogo/iconos, toolchain), no codigo de la app |
| `%LOCALAPPDATA%\CenterApps\` | `installed.json`, `settings.json`, `repository-apps.json`, portables `<id>\`, `logs\` (20 sesiones + `msi-*.log`), carpetas `.*.borrar` pendientes |
| `%USERPROFILE%\Downloads\WinSlimCenter\<id>\` | Paquetes descargados; se vacia al arrancar |
| `%LOCALAPPDATA%\WinSlimCenter-Backups\` | Copias de datos de portables |
| `Build\WinSlimCenter.exe`, `dist\` | Salida de `build.cmd` y del empaquetado del frontend (ignorados por git) |

## Compilar y verificar

```powershell
.\build.cmd
npm run test:rust
npm run test:ui
npm run frontend:check
npm run audit:categories
```

Cobertura: Rust unitario por modulo (parsers de WinGet/registro, seleccion de assets, variantes, catalogo, rutas protegidas) y funciones de `main.js` en `vm`. Sin cobertura: instalaciones/desinstalaciones reales, WinGet/Chocolatey/Store reales, UAC y la GUI completa. Una compilacion correcta no verifica estetica ni comportamiento en Windows.
