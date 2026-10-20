# Entrada para agentes

WinSlimCenter: tienda de software para Windows x64. Tauri 2 + Rust (`src-tauri/`, crate `center_app_lib`) + frontend HTML/CSS/JS sin framework (`src/`). Manifiesto `requireAdministrator`: la app siempre corre elevada. Usuario, UI y logs en espanol; comentarios del codigo explican el porque (ingles o espanol).

## Presupuesto de contexto

- Antes de explorar, abre [docs/AI_MAP.md](docs/AI_MAP.md) y lee solo la fila del enrutador de tu tarea y su contrato. No leas el mapa entero ni todo el codigo. El codigo es la autoridad: verifica cada simbolo con `rg -n` y lee el bloque afectado y sus llamadores.
- No leas completos los archivos grandes: `src/main.js` (~5900 lineas), `installer.rs` (~5400), `lib.rs` (~4100), `styles.css` (~3400), `msstore.rs` (~3200), `detect.rs` (~2000), `download.rs` (~1700). Busca el simbolo y lee 40-80 lineas alrededor.
- `src-tauri/apps.json` (~280 fichas, 170 KB): nunca entero. Busca por `"id": "<id>"` o filtra con `ConvertFrom-Json` / `JSON.parse`. Para copiar el estilo de una ficha, lee una parecida del mismo `source_type`.
- `README.md` es la portada humana (producto, comandos, estructura); solo si la tarea es de producto. Excluye `node_modules/`, `dist/`, `Build/`, `src-tauri/target/`, `src-tauri/gen/`, `scratch/.frontend-maps/` y `src/assets/` (binarios).

## Reglas

- Conserva funcionalidad, estetica (tokens de `styles.css`, fuentes Ubuntu, medidas) y compatibilidad salvo peticion expresa. Textos visibles y logs en espanol.
- Una ficha nueva del catalogo no necesita codigo: elige `source_type` y campos segun la seccion "Fichas del catalogo" del mapa y reutiliza la maquinaria existente.
- Secciones (9) y categorias estan cerradas: una categoria nueva exige actualizar el test `catalog_categories_are_assigned_to_their_expected_sections` (`store.rs`) y `scratch/audit_catalog_categories.cjs`.
- No instales ni desinstales software real ni cambies PATH/registro para validar: la tienda corre elevada y tus pruebas no. Comprueba instaladores de terceros leyendo su release, manifiesto o cadenas, nunca ejecutandolos.
- `scratch/test_*.cjs` cargan funciones de `main.js` por nombre: si renombras una, actualiza el test.
- Archivos con CRLF en el arbol de trabajo (`core.autocrlf=true`); conserva el formato. Scripts `.ps1` corren en PowerShell 5.1: sin caracteres no ASCII salvo que el archivo tenga BOM.

## Compilar y probar

- Compilar: `build.cmd` (instala Build Tools/Rust/Node si faltan, empaqueta el frontend con esbuild en `dist/` y deja `Build/WinSlimCenter.exe`; target de Cargo en `%LOCALAPPDATA%\WinSlimCenter\cargo-target`). Version: solo `src-tauri/Cargo.toml` (`UPversion.cmd`); la de `package.json` no se usa.
- Pruebas: `npm run test:rust` (cargo test `--offline` + manifiesto Common Controls con `mt.exe` del Windows SDK), `npm run test:ui` (node + vm), `npm run frontend:check`, `npm run audit:categories`. Si en la sesion no hay `cargo`/`node` en PATH, valida `apps.json` con PowerShell y di que las pruebas no se ejecutaron.
- Un cambio en `apps.json` o en `src/` solo llega a otros equipos recompilando: el catalogo y los iconos van embebidos en el exe.

## Mantenimiento del mapa

Al cambiar responsabilidades, flujos, campos del catalogo o validaciones, actualiza la fila o el contrato del mapa en una o dos lineas. El mapa no es un changelog: nada de historial ni "revisado/pendiente" salvo que cambie una decision. No dupliques este indice en otros documentos.
