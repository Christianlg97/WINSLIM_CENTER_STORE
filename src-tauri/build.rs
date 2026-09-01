fn main() {
    // El manifiesto propio sustituye al que Tauri incrusta por defecto: es el
    // mismo más `requireAdministrator`, que es lo que necesita la tienda para
    // registrar paquetes de la Microsoft Store con servicio dentro. Ver
    // `windows-app-manifest.xml`.
    println!("cargo:rerun-if-changed=windows-app-manifest.xml");
    let manifest = include_str!("windows-app-manifest.xml");
    let windows = tauri_build::WindowsAttributes::new().app_manifest(manifest);
    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
        .expect("no se pudo preparar la compilación de Tauri");
}
