// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // WebView2 debe nacer con el mismo fondo que la ventana y el HTML. Fijarlo
    // solo en el controlador permite un destello durante su inicialización.
    // Es una variable de este proceso, antes de crear WebView2 o sus hilos.
    #[cfg(windows)]
    std::env::set_var("WEBVIEW2_DEFAULT_BACKGROUND_COLOR", "FF181818");
    center_app_lib::run()
}
