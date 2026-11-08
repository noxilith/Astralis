// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // `--version` prints the version and quits without opening a window. The updater runs a
    // downloaded copy this way to check it starts on this system before installing it.
    if std::env::args().skip(1).any(|a| a == "--version" || a == "-V") {
        println!("Astralis {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    astralis_desktop_lib::run()
}
