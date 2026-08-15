//! Publishing an application in the Start Menu.
//!
//! Open-Shell builds its program list — and therefore its search results — from
//! the shortcuts under `shell:programs`. A program that never leaves one there
//! cannot be found by name no matter how it was installed, so the store
//! publishes itself and the terminal it ships, each under a folder of its own,
//! exactly the way an ordinary installer would.

use std::path::{Path, PathBuf};

/// `%APPDATA%\Microsoft\Windows\Start Menu\Programs`, the per-user
/// `shell:programs`. The per-user location needs no administrator rights and is
/// indexed just like the machine-wide one.
pub fn programs_dir() -> Option<PathBuf> {
    std::env::var("APPDATA")
        .ok()
        .map(|base| PathBuf::from(base).join(r"Microsoft\Windows\Start Menu\Programs"))
        .filter(|path| path.is_dir())
}

/// Where the shortcut of an application published by the store lives.
pub fn shortcut_path(folder: &str, name: &str) -> Option<PathBuf> {
    Some(programs_dir()?.join(folder).join(format!("{name}.lnk")))
}

/// Everything a shortcut carries beyond where it points.
///
/// A web application needs the other two: the browser is the target for every
/// one of them, so what tells them apart is the address in `arguments` and the
/// site's own icon.
#[derive(Default)]
pub struct Extras<'a> {
    pub arguments: Option<&'a str>,
    pub icon: Option<&'a Path>,
}

/// Creates `<Programs>\<folder>\<name>.lnk` pointing at `target`, replacing any
/// shortcut already there.
#[cfg(windows)]
pub fn publish(folder: &str, name: &str, target: &Path) -> Result<PathBuf, String> {
    let programs =
        programs_dir().ok_or("Windows no indica dónde está el menú Inicio del usuario")?;
    publish_into(&programs, folder, name, target)
}

/// [`publish`] for a shortcut that also carries arguments or an icon.
#[cfg(windows)]
pub fn publish_with(
    folder: &str,
    name: &str,
    target: &Path,
    extras: &Extras<'_>,
) -> Result<PathBuf, String> {
    let programs =
        programs_dir().ok_or("Windows no indica dónde está el menú Inicio del usuario")?;
    let directory = programs.join(folder);
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("No se pudo crear {}: {error}", directory.display()))?;
    let shortcut = directory.join(format!("{name}.lnk"));
    write_shortcut(&shortcut, target, name, extras)?;
    Ok(shortcut)
}

/// Writes a `.lnk` wherever it is asked to, which is what the desktop copy and
/// the Start Menu one have in common.
#[cfg(windows)]
pub fn write_shortcut(
    shortcut: &Path,
    target: &Path,
    description: &str,
    extras: &Extras<'_>,
) -> Result<(), String> {
    if !target.is_file() {
        return Err(format!(
            "No existe el ejecutable que se quería publicar: {}",
            target.display()
        ));
    }
    let quote = |value: &str| value.replace('\'', "''");
    let working = target
        .parent()
        .map(|parent| parent.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut script = format!(
        "$ErrorActionPreference='Stop'; \
         $shell = New-Object -ComObject WScript.Shell; \
         $link = $shell.CreateShortcut('{}'); \
         $link.TargetPath = '{}'; \
         $link.WorkingDirectory = '{}'; \
         $link.Description = '{}'; ",
        quote(&shortcut.to_string_lossy()),
        quote(&target.to_string_lossy()),
        quote(&working),
        quote(description),
    );
    if let Some(arguments) = extras.arguments {
        script.push_str(&format!("$link.Arguments = '{}'; ", quote(arguments)));
    }
    if let Some(icon) = extras.icon {
        script.push_str(&format!(
            "$link.IconLocation = '{}'; ",
            quote(&icon.to_string_lossy())
        ));
    }
    script.push_str("$link.Save()");

    let output = crate::process::hidden_output(
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
    )
    .map_err(|error| format!("Windows no pudo crear el acceso directo: {error}"))?;
    if !output.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if detail.is_empty() {
            format!("Windows no pudo guardar {}", shortcut.display())
        } else {
            format!("Windows no pudo guardar {}: {detail}", shortcut.display())
        });
    }
    Ok(())
}

/// The body of [`publish`], with the Start Menu handed in so it can be exercised
/// without writing into the one the user is looking at.
#[cfg(windows)]
fn publish_into(
    programs: &Path,
    folder: &str,
    name: &str,
    target: &Path,
) -> Result<PathBuf, String> {
    if !target.is_file() {
        return Err(format!(
            "No existe el ejecutable que se quería publicar: {}",
            target.display()
        ));
    }
    let directory = programs.join(folder);
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("No se pudo crear {}: {error}", directory.display()))?;
    let shortcut = directory.join(format!("{name}.lnk"));

    let quote = |value: &str| value.replace('\'', "''");
    let working = target
        .parent()
        .map(|parent| parent.to_string_lossy().to_string())
        .unwrap_or_default();
    let script = format!(
        "$ErrorActionPreference='Stop'; \
         $shell = New-Object -ComObject WScript.Shell; \
         $link = $shell.CreateShortcut('{}'); \
         $link.TargetPath = '{}'; \
         $link.WorkingDirectory = '{}'; \
         $link.Description = '{}'; \
         $link.Save()",
        quote(&shortcut.to_string_lossy()),
        quote(&target.to_string_lossy()),
        quote(&working),
        quote(name),
    );
    let output = crate::process::hidden_output(
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
    )
    .map_err(|error| format!("Windows no pudo crear el acceso directo: {error}"))?;

    if !output.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if detail.is_empty() {
            format!("Windows no pudo guardar {}", shortcut.display())
        } else {
            format!("Windows no pudo guardar {}: {detail}", shortcut.display())
        });
    }
    crate::logger::info(
        "start-menu",
        format!(
            "Publicado en el menú Inicio: {} -> {}",
            shortcut.display(),
            target.display()
        ),
    );
    Ok(shortcut)
}

#[cfg(not(windows))]
pub fn publish(_folder: &str, _name: &str, _target: &Path) -> Result<PathBuf, String> {
    Err("La publicación en el menú Inicio solo está disponible en Windows.".into())
}

/// Publishes only when the shortcut is not already there.
///
/// The check is a plain look at the filesystem, so the ordinary start-up — where
/// the entry already exists — costs nothing and never launches a process.
pub fn publish_if_missing(folder: &str, name: &str, target: &Path) {
    if shortcut_path(folder, name).is_some_and(|path| path.is_file()) {
        return;
    }
    announce(publish(folder, name, target));
}

/// Publishes, replacing whatever was there. Used right after an installation,
/// when the executable may well sit somewhere new.
pub fn republish(folder: &str, name: &str, target: &Path) {
    announce(publish(folder, name, target));
}

/// A missing Start Menu entry is a cosmetic loss: it must never turn a start-up
/// or a finished installation into a failure the user has to deal with.
fn announce(outcome: Result<PathBuf, String>) {
    if let Err(error) = outcome {
        crate::logger::warn(
            "start-menu",
            format!("No se pudo publicar en el menú Inicio: {error}"),
        );
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    /// Reads the shortcut back the same way Windows — and Open-Shell — do.
    fn resolved_target(shortcut: &Path) -> String {
        let script = format!(
            "(New-Object -ComObject WScript.Shell).CreateShortcut('{}').TargetPath",
            shortcut.to_string_lossy().replace('\'', "''")
        );
        let output = crate::process::hidden_output(
            "powershell.exe",
            &[
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                script.as_str(),
            ],
        )
        .unwrap();
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    #[test]
    fn the_published_shortcut_lives_in_its_own_folder_and_resolves() {
        let root =
            std::env::temp_dir().join(format!("winslimcenter-startmenu-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let programs = root.join("Programs");
        std::fs::create_dir_all(&programs).unwrap();
        let target = root.join("WinSlimCenter.exe");
        std::fs::copy(r"C:\Windows\System32\cmd.exe", &target).unwrap();

        let shortcut = publish_into(&programs, "WinSlimCenter", "WinSlimCenter", &target).unwrap();

        // The shape Open-Shell lists: a folder carrying the program's name with
        // the shortcut inside it, not a loose .lnk in the root.
        assert_eq!(
            shortcut,
            programs.join("WinSlimCenter").join("WinSlimCenter.lnk")
        );
        assert_eq!(resolved_target(&shortcut), target.to_string_lossy());

        // Publishing again must overwrite rather than pile up copies.
        publish_into(&programs, "WinSlimCenter", "WinSlimCenter", &target).unwrap();
        let entries = std::fs::read_dir(programs.join("WinSlimCenter"))
            .unwrap()
            .count();
        assert_eq!(entries, 1);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn publishing_something_that_is_not_there_fails_instead_of_leaving_a_dead_entry() {
        let root = std::env::temp_dir()
            .join(format!("winslimcenter-startmenu-dead-{}", std::process::id()));
        let programs = root.join("Programs");
        std::fs::create_dir_all(&programs).unwrap();

        let outcome = publish_into(
            &programs,
            "WinSlimTerminal",
            "WinSlimTerminal",
            &root.join("no-existe.exe"),
        );

        assert!(outcome.is_err());
        assert!(!programs.join("WinSlimTerminal").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
