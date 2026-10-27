//! Lo que una actualización deja en el escritorio.
//!
//! Muchos instaladores vuelven a poner su icono en el escritorio cada vez que se
//! ejecutan, también cuando solo actualizan: Inno Setup repite las tareas que se
//! eligieron en la primera instalación y bastantes NSIS lo escriben sin
//! preguntar. Quien había borrado ese icono lo veía volver con cada
//! actualización. La tienda mira el escritorio justo antes de actualizar y, al
//! terminar, retira lo que apareció entretanto apuntando a la aplicación
//! actualizada. Lo que el usuario ya tenía no se toca.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Los escritorios en los que un instalador deja accesos directos: el del
/// usuario —allí donde Windows lo tenga, también redirigido a OneDrive— y el
/// público, que es el que usan los instaladores para todos los usuarios.
fn roots() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    let candidates = [
        dirs::desktop_dir(),
        std::env::var_os("USERPROFILE").map(|base| PathBuf::from(base).join("Desktop")),
        std::env::var_os("PUBLIC").map(|base| PathBuf::from(base).join("Desktop")),
    ];
    for root in candidates.into_iter().flatten() {
        if root.is_dir() && !roots.iter().any(|known| key(known) == key(&root)) {
            roots.push(root);
        }
    }
    roots
}

/// Windows no distingue mayúsculas en las rutas: un instalador que escribe
/// `app.lnk` donde había `App.lnk` deja el mismo acceso.
fn key(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

fn shortcuts_in(roots: &[PathBuf]) -> Vec<PathBuf> {
    roots
        .iter()
        .filter_map(|root| std::fs::read_dir(root).ok())
        .flat_map(|entries| entries.filter_map(Result::ok))
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("lnk"))
                && path.is_file()
        })
        .collect()
}

/// Los accesos directos que había en el escritorio antes de actualizar.
pub struct Snapshot {
    roots: Vec<PathBuf>,
    links: HashSet<String>,
}

impl Snapshot {
    /// Solo lista carpetas: no resuelve ningún acceso ni lanza procesos.
    pub fn take() -> Self {
        Self::of(roots())
    }

    fn of(roots: Vec<PathBuf>) -> Self {
        let links = shortcuts_in(&roots).iter().map(|link| key(link)).collect();
        Self { roots, links }
    }

    /// Si `link` ya estaba en el escritorio cuando se tomó la foto.
    pub fn had(&self, link: &Path) -> bool {
        self.links.contains(&key(link))
    }

    /// Retira los accesos que aparecieron desde la foto y apuntan dentro de
    /// alguna de `folders`, las carpetas de la aplicación antes y después de
    /// actualizarla. Devuelve los que retiró.
    ///
    /// Se mira el destino y no el nombre: un acceso nuevo de otro programa —una
    /// instalación de la cola que terminó mientras tanto, o uno que creó el
    /// propio usuario— no apunta a esta aplicación y se queda.
    pub fn remove_added_for(&self, folders: &[PathBuf]) -> Vec<PathBuf> {
        let now = shortcuts_in(&self.roots);
        let present: HashSet<String> = now.iter().map(|link| key(link)).collect();
        let added: Vec<PathBuf> = now
            .into_iter()
            .filter(|link| !self.links.contains(&key(link)))
            .collect();
        if added.is_empty() {
            return Vec::new();
        }
        // Un acceso que desaparece mientras aparece otro es un instalador que lo
        // renombró («App 2025» por «App 2026»): el nuevo sustituye al que el
        // usuario tenía, y quitarlo lo dejaría sin ninguno.
        if self.links.iter().any(|link| !present.contains(link)) {
            crate::logger::info(
                "desktop-shortcut",
                "La actualización sustituyó un acceso del escritorio; se deja como está.",
            );
            return Vec::new();
        }
        let owned = owned_folders(folders, &self.roots);
        if owned.is_empty() {
            return Vec::new();
        }
        let mut removed = Vec::new();
        for (link, target) in resolve_targets(&added) {
            if !inside_any(&target, &owned) {
                continue;
            }
            match std::fs::remove_file(&link) {
                Ok(()) => removed.push(link),
                Err(error) => crate::logger::warn(
                    "desktop-shortcut",
                    format!("No se pudo retirar {}: {error}", link.display()),
                ),
            }
        }
        removed
    }
}

/// Las carpetas que identifican a la aplicación, listas para comparar.
///
/// Una ruta a un ejecutable cuenta por su carpeta. Las raíces que comparten
/// muchos programas —`Program Files`, `%LOCALAPPDATA%`, el perfil— y el propio
/// escritorio no identifican a ninguno: con ellas, cualquier acceso nuevo
/// parecería de esta aplicación.
fn owned_folders(folders: &[PathBuf], desktops: &[PathBuf]) -> Vec<String> {
    let mut owned: Vec<String> = Vec::new();
    for path in folders {
        if !crate::installer::is_filesystem_target(path) {
            continue;
        }
        let is_program = path.is_file()
            || path.extension().is_some_and(|extension| {
                ["exe", "com", "bat", "cmd"]
                    .iter()
                    .any(|known| extension.eq_ignore_ascii_case(known))
            });
        let folder = if is_program {
            path.parent()
        } else {
            Some(path.as_path())
        };
        let Some(folder) = folder else {
            continue;
        };
        if crate::installer::is_protected_installation_root(folder) {
            continue;
        }
        let folder = key(folder);
        if desktops.iter().any(|desktop| key(desktop) == folder) || owned.contains(&folder) {
            continue;
        }
        owned.push(folder);
    }
    owned
}

fn inside_any(target: &Path, folders: &[String]) -> bool {
    let target = key(target);
    folders
        .iter()
        .any(|folder| target == *folder || target.starts_with(&format!("{folder}\\")))
}

/// El destino de cada acceso, resuelto como lo resuelve el propio Windows.
#[cfg(windows)]
fn resolve_targets(links: &[PathBuf]) -> Vec<(PathBuf, PathBuf)> {
    let quote = |value: &str| value.replace('\'', "''");
    let literal = links
        .iter()
        .map(|link| format!("'{}'", quote(&link.to_string_lossy())))
        .collect::<Vec<_>>()
        .join(",");
    // Cada respuesta lleva la posición del acceso y no su ruta: lo que se borra
    // es siempre el archivo por el que se preguntó, nunca una ruta reconstruida
    // a partir de lo que escribió la consola.
    let script = format!(
        r#"$ErrorActionPreference='SilentlyContinue';
[Console]::OutputEncoding=[Text.Encoding]::UTF8;
$shell=New-Object -ComObject WScript.Shell;
$links=@({literal});
for($i=0; $i -lt $links.Count; $i++){{
  try {{
    $raw=[Environment]::ExpandEnvironmentVariables([string]$shell.CreateShortcut($links[$i]).TargetPath);
    if(-not [string]::IsNullOrWhiteSpace($raw)){{ [Console]::Out.WriteLine([string]$i + '|' + [IO.Path]::GetFullPath($raw)) }}
  }} catch {{}}
}};"#
    );
    let Ok(output) = crate::process::hidden_output(
        "powershell.exe",
        &[
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-Command",
            script.as_str(),
        ],
    ) else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let (index, target) = line.trim_start_matches('\u{feff}').split_once('|')?;
            let link = links.get(index.trim().parse::<usize>().ok()?)?;
            let target = target.trim();
            (!target.is_empty()).then(|| (link.clone(), PathBuf::from(target)))
        })
        .collect()
}

#[cfg(not(windows))]
fn resolve_targets(_links: &[PathBuf]) -> Vec<(PathBuf, PathBuf)> {
    Vec::new()
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::start_menu::{write_shortcut, Extras};

    /// Un escritorio de pega con dos programas, para no tocar el del usuario.
    fn sandbox(label: &str) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "winslimcenter-desktop-{label}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let desktop = root.join("Desktop");
        let app = root.join("Aplicacion");
        let other = root.join("Otra");
        for folder in [&desktop, &app, &other] {
            std::fs::create_dir_all(folder).unwrap();
        }
        std::fs::copy(r"C:\Windows\System32\cmd.exe", app.join("App.exe")).unwrap();
        std::fs::copy(r"C:\Windows\System32\cmd.exe", other.join("Otra.exe")).unwrap();
        (root, desktop, app, other)
    }

    fn link(desktop: &Path, name: &str, target: &Path) -> PathBuf {
        let shortcut = desktop.join(format!("{name}.lnk"));
        write_shortcut(&shortcut, target, name, &Extras::default()).unwrap();
        shortcut
    }

    #[test]
    fn an_update_takes_back_only_the_shortcut_it_added_for_its_own_app() {
        let (root, desktop, app, other) = sandbox("added");
        let kept = link(&desktop, "Mi App", &app.join("App.exe"));
        let before = Snapshot::of(vec![desktop.clone()]);
        assert!(before.had(&kept));

        let recreated = link(&desktop, "App", &app.join("App.exe"));
        let unrelated = link(&desktop, "Otra", &other.join("Otra.exe"));
        let removed = before.remove_added_for(&[app.join("App.exe")]);

        assert_eq!(removed, vec![recreated.clone()]);
        assert!(!recreated.exists());
        // Lo que el usuario ya tenía y lo que no es de esta aplicación se quedan.
        assert!(kept.is_file());
        assert!(unrelated.is_file());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_shortcut_renamed_by_the_update_stays() {
        let (root, desktop, app, _) = sandbox("renamed");
        let old = link(&desktop, "App 2025", &app.join("App.exe"));
        let before = Snapshot::of(vec![desktop.clone()]);

        std::fs::remove_file(&old).unwrap();
        let renamed = link(&desktop, "App 2026", &app.join("App.exe"));

        assert!(before.remove_added_for(&[app.clone()]).is_empty());
        assert!(renamed.is_file());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn shared_roots_never_stand_for_an_application() {
        let (root, desktop, app, _) = sandbox("roots");
        let program_files = PathBuf::from(std::env::var("ProgramFiles").unwrap());
        assert!(owned_folders(&[program_files.clone()], &[]).is_empty());
        // Un ejecutable suelto en `Program Files` tampoco señala a nadie.
        assert!(owned_folders(&[program_files.join("Suelto.exe")], &[]).is_empty());
        assert!(owned_folders(&[desktop.join("Portable.exe")], &[desktop.clone()]).is_empty());
        assert!(owned_folders(&[PathBuf::from(r"shell:AppsFolder\App")], &[]).is_empty());
        assert_eq!(owned_folders(&[app.join("App.exe")], &[]), vec![key(&app)]);
        assert!(inside_any(&app.join("bin").join("App.exe"), &[key(&app)]));
        // `Aplicacion2` empieza igual que `Aplicacion`, pero es otra carpeta.
        assert!(!inside_any(
            &root.join("Aplicacion2").join("App.exe"),
            &[key(&app)]
        ));
        let _ = std::fs::remove_dir_all(&root);
    }
}
