fn main() {
    // Only the tray app is a Tauri app; t3boat-server builds without it.
    #[cfg(feature = "desktop")]
    tauri_build::build()
}
