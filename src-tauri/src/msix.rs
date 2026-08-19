//! Applications Windows installs as packages, and the one thing that makes them
//! different from every other install the store handles.
//!
//! A packaged application is never replaced in place. While any of its processes
//! is running, Windows keeps the version currently registered for the user and
//! leaves the new one staged on disk, applying it only once the last process
//! exits. WinGet reports that upgrade as successful — it was — and everything
//! that asks Windows afterwards is told the old version, because that is still
//! the registered one.
//!
//! Nothing here knew the difference between "not updated" and "updated, waiting
//! for a restart". So the store announced "Claude actualizado correctamente",
//! asked WinGet a second later, got `1.26832.0.0` back and put the update badge
//! straight back on the card — for nine minutes and two full re-downloads of a
//! package that had installed correctly the first time. The uninstall side had
//! the same blind spot from the other direction: WinGet removed the package,
//! Windows deferred the removal because the app was open, and the store reported
//! a successful uninstall as "Windows sigue informando de que está instalada".
//!
//! The registered version is read straight from the package repository in the
//! registry. That is the value Windows itself answers with, it costs no process
//! to read, and it is the only source here that distinguishes the two states —
//! the Start Menu, which is what the store used to trust, keeps listing a
//! package long after it is gone.

use std::path::{Path, PathBuf};

/// Where Windows records the packages registered for the current user. The name
/// of each subkey is the package full name, `Name_Version_Arch__PublisherHash`.
#[cfg(windows)]
const PACKAGE_REPOSITORY: &str = r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages";

/// One registered package, as Windows currently has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    /// `Claude_1.30096.1.0_x64__pzs8sxrjxfjjc`
    pub full_name: String,
    /// `1.30096.1.0`
    pub version: String,
}

impl Package {
    /// Where the package's own executables live, which is what identifies its
    /// running processes.
    pub fn folder(&self) -> PathBuf {
        let program_files = std::env::var("ProgramFiles")
            .unwrap_or_else(|_| r"C:\Program Files".to_string());
        PathBuf::from(program_files)
            .join("WindowsApps")
            .join(&self.full_name)
    }
}

/// The package family an identifier belongs to, when it names a packaged
/// application at all.
///
/// The store carries these as the Start Menu reports them —
/// `shell:AppsFolder\Claude_pzs8sxrjxfjjc!Claude` — where the part before the
/// `!` is the family: the package name and the publisher hash, without the
/// version that changes under it on every update.
pub fn family_from_identifier(identifier: &str) -> Option<String> {
    let trimmed = identifier.trim();
    let without_prefix = trimmed
        .strip_prefix(crate::residue::START_MENU_PREFIX)
        .unwrap_or(trimmed);
    let (family, _application) = without_prefix.split_once('!')?;
    let family = family.trim();
    // A family is `Name_PublisherHash`. Anything without that separator is an
    // ordinary AppID that merely happens to carry a `!`.
    if family.is_empty() || !family.contains('_') {
        return None;
    }
    Some(family.to_string())
}

/// Splits a family into the two halves a package full name is built from.
fn family_parts(family: &str) -> Option<(&str, &str)> {
    let (name, hash) = family.rsplit_once('_')?;
    if name.is_empty() || hash.is_empty() {
        return None;
    }
    Some((name, hash))
}

/// `true` when `full_name` is a version of this family.
///
/// Matched on both halves at once. The name alone would let `Claude` claim
/// `ClaudeCode`, and the publisher hash alone is shared by everything the same
/// publisher ships.
fn full_name_belongs_to(full_name: &str, name: &str, hash: &str) -> bool {
    full_name.len() > name.len() + hash.len() + 3
        && full_name[..name.len()].eq_ignore_ascii_case(name)
        && full_name.as_bytes().get(name.len()) == Some(&b'_')
        && full_name
            .to_ascii_lowercase()
            .ends_with(&format!("__{}", hash.to_ascii_lowercase()))
}

/// The version field of a package full name, `Name_Version_Arch__Hash`.
fn version_in_full_name(full_name: &str, name: &str) -> Option<String> {
    let rest = full_name.get(name.len() + 1..)?;
    let version = rest.split('_').next()?;
    if version.is_empty() {
        return None;
    }
    Some(version.to_string())
}

/// The package Windows currently has registered for this family, if any.
///
/// `None` is the honest answer for "not installed for this user": a package
/// staged on disk but not registered is not something the user can run.
#[cfg(windows)]
pub fn registered_package(family: &str) -> Option<Package> {
    use winreg::enums::*;
    use winreg::RegKey;

    let (name, hash) = family_parts(family)?;
    let packages = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(PACKAGE_REPOSITORY)
        .ok()?;
    packages
        .enum_keys()
        .flatten()
        .find(|full_name| full_name_belongs_to(full_name, name, hash))
        .and_then(|full_name| {
            let version = version_in_full_name(&full_name, name)?;
            Some(Package { full_name, version })
        })
}

#[cfg(not(windows))]
pub fn registered_package(_family: &str) -> Option<Package> {
    None
}

/// Whether Windows still has any version of this family registered.
pub fn is_registered(family: &str) -> bool {
    registered_package(family).is_some()
}

/// The folder whose processes belong to this family, when it is registered.
pub fn running_folder(identifier: &str) -> Option<PathBuf> {
    let family = family_from_identifier(identifier)?;
    registered_package(&family).map(|package| package.folder())
}

/// Whether the version Windows has registered is already the one the store was
/// upgrading to.
///
/// Answers `false` while the swap is still pending, which is exactly the state
/// the store had no way of naming: the new package is on disk, the old one is
/// still what runs, and the only thing missing is for the application to be
/// closed.
pub fn registration_caught_up(family: &str, target: &str) -> bool {
    let Some(package) = registered_package(family) else {
        return false;
    };
    if package.version.eq_ignore_ascii_case(target.trim()) {
        return true;
    }
    // A target the store read from WinGet is not always spelled like the package
    // version — `1.30096.1` against `1.30096.1.0` — so "not older" settles it.
    !crate::detect::is_newer(target, &package.version).unwrap_or(false)
}

/// What a Windows-deferred operation looks like in WinGet's own words.
///
/// WinGet exits successfully and says so in prose, which is the only signal
/// there is: the exit code is the same `0` a completed install returns.
pub fn output_defers_to_restart(output: &str) -> bool {
    let lowered = output.to_lowercase();
    [
        // Spanish and English, as WinGet localises this line.
        "reinicie la aplicación para completar",
        "reinicie la aplicacion para completar",
        "restart the application to complete",
        "reinicia la aplicación para completar",
    ]
    .iter()
    .any(|marker| lowered.contains(marker))
}

/// `true` when this identifier names a packaged application.
pub fn is_packaged(identifier: &str) -> bool {
    family_from_identifier(identifier).is_some()
}

/// Where a packaged application's processes run from, for the caller that needs
/// to close them before Windows will apply anything.
pub fn folder_of(identifier: &str) -> Option<PathBuf> {
    running_folder(identifier)
}

/// `true` when the folder is one of ours to act on: under WindowsApps and
/// naming a package, never the WindowsApps root itself.
pub fn is_package_folder(folder: &Path) -> bool {
    folder
        .parent()
        .and_then(|parent| parent.file_name())
        .is_some_and(|name| name.eq_ignore_ascii_case("WindowsApps"))
        && folder.file_name().is_some_and(|name| {
            name.to_string_lossy().contains("__")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_start_menu_identifier_gives_up_its_package_family() {
        assert_eq!(
            family_from_identifier(r"shell:AppsFolder\Claude_pzs8sxrjxfjjc!Claude").as_deref(),
            Some("Claude_pzs8sxrjxfjjc")
        );
        // The bare AppUserModelID, without the shell prefix.
        assert_eq!(
            family_from_identifier("Claude_pzs8sxrjxfjjc!Claude").as_deref(),
            Some("Claude_pzs8sxrjxfjjc")
        );
        // An ordinary program: a path, no `!`, no family.
        assert!(family_from_identifier(r"C:\Program Files\Ejemplo\app.exe").is_none());
        // A legacy Start Menu identifier that carries no publisher hash.
        assert!(family_from_identifier("Anysphere.Cursor").is_none());
    }

    #[test]
    fn a_package_is_matched_on_its_name_and_its_publisher_together() {
        // The real one.
        assert!(full_name_belongs_to(
            "Claude_1.30096.1.0_x64__pzs8sxrjxfjjc",
            "Claude",
            "pzs8sxrjxfjjc"
        ));
        // Same publisher, different product: `Claude` must not claim it.
        assert!(!full_name_belongs_to(
            "ClaudeCode_1.0.0.0_x64__pzs8sxrjxfjjc",
            "Claude",
            "pzs8sxrjxfjjc"
        ));
        // Same product name from somebody else entirely.
        assert!(!full_name_belongs_to(
            "Claude_1.30096.1.0_x64__otherpublisher",
            "Claude",
            "pzs8sxrjxfjjc"
        ));
    }

    #[test]
    fn the_version_is_read_out_of_the_package_full_name() {
        assert_eq!(
            version_in_full_name("Claude_1.30096.1.0_x64__pzs8sxrjxfjjc", "Claude").as_deref(),
            Some("1.30096.1.0")
        );
        assert_eq!(
            version_in_full_name("Claude_1.26832.0.0_x64__pzs8sxrjxfjjc", "Claude").as_deref(),
            Some("1.26832.0.0")
        );
    }

    #[test]
    fn winget_saying_it_needs_a_restart_is_recognised_in_either_language() {
        // The exact line WinGet printed while Claude was open.
        assert!(output_defers_to_restart(
            "Se instaló correctamente. Reinicie la aplicación para completar la actualización."
        ));
        assert!(output_defers_to_restart(
            "Successfully installed. Restart the application to complete the upgrade."
        ));
        // A plain success says nothing about restarting.
        assert!(!output_defers_to_restart("Se instaló correctamente"));
    }

    #[test]
    fn only_a_versioned_package_directory_is_treated_as_one() {
        assert!(is_package_folder(Path::new(
            r"C:\Program Files\WindowsApps\Claude_1.30096.1.0_x64__pzs8sxrjxfjjc"
        )));
        // The root itself must never be handed to anything that stops processes.
        assert!(!is_package_folder(Path::new(
            r"C:\Program Files\WindowsApps"
        )));
        assert!(!is_package_folder(Path::new(
            r"C:\Program Files\Ejemplo"
        )));
    }
}
