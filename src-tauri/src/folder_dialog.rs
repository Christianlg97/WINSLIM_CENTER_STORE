//! The Explorer folder picker, for the settings that move the store's folders.

use std::path::{Path, PathBuf};

/// Asks for a folder with the same dialog the Explorer uses, owned by the
/// store's window so it stays in front and blocks it like any other dialog.
/// `Ok(None)` is the user cancelling.
///
/// It runs on a thread of its own: the dialog needs a single-threaded COM
/// apartment and pumps its own messages until it closes, which neither the
/// window's thread nor the async runtime can be left doing.
#[cfg(windows)]
pub async fn pick_folder(
    owner: isize,
    title: String,
    initial: Option<PathBuf>,
) -> Result<Option<PathBuf>, String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let _ = sender.send(show(owner, &title, initial.as_deref()));
    });
    receiver
        .await
        .map_err(|_| "El selector de carpetas se cerró de forma inesperada.".to_string())?
}

#[cfg(windows)]
fn show(owner: isize, title: &str, initial: Option<&Path>) -> Result<Option<PathBuf>, String> {
    use windows::core::{HRESULT, HSTRING};
    use windows::Win32::Foundation::{ERROR_CANCELLED, HWND};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
    };
    use windows::Win32::UI::Shell::{
        FileOpenDialog, IFileOpenDialog, IShellItem, SHCreateItemFromParsingName,
        FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST, FOS_PICKFOLDERS, SIGDN_FILESYSPATH,
    };

    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            // SAFETY: only built after CoInitializeEx succeeded on this thread.
            unsafe { CoUninitialize() };
        }
    }

    let failed = |error: windows::core::Error| format!("No se pudo abrir el selector de carpetas: {error}");
    // SAFETY: every COM call happens on this thread, inside the apartment opened
    // here and kept by `_apartment` until the interfaces above it are released
    // (they are declared after it, so they drop first). The string Windows
    // allocates for the result is freed with CoTaskMemFree once copied.
    unsafe {
        let _apartment = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE)
            .is_ok()
            .then_some(Apartment);
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).map_err(failed)?;
        let options = dialog.GetOptions().map_err(failed)?;
        dialog
            .SetOptions(options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST)
            .map_err(failed)?;
        let _ = dialog.SetTitle(&HSTRING::from(title));
        if let Some(folder) = initial.filter(|folder| folder.is_dir()) {
            let item: Result<IShellItem, _> =
                SHCreateItemFromParsingName(&HSTRING::from(folder.as_os_str()), None);
            if let Ok(item) = item {
                let _ = dialog.SetFolder(&item);
            }
        }
        match dialog.Show(Some(HWND(owner as _))) {
            Ok(()) => {}
            Err(error) if error.code() == HRESULT::from_win32(ERROR_CANCELLED.0) => return Ok(None),
            Err(error) => return Err(failed(error)),
        }
        let item = dialog.GetResult().map_err(failed)?;
        let name = item.GetDisplayName(SIGDN_FILESYSPATH).map_err(failed)?;
        let path = name.to_string();
        CoTaskMemFree(Some(name.0 as _));
        path.map(|path| Some(PathBuf::from(path)))
            .map_err(|error| format!("Ruta no válida: {error}"))
    }
}

#[cfg(not(windows))]
pub async fn pick_folder(
    _owner: isize,
    _title: String,
    _initial: Option<PathBuf>,
) -> Result<Option<PathBuf>, String> {
    Err("El selector de carpetas solo existe en Windows.".into())
}
