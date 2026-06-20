// Tauri build script.
//
// Only runs when the `tauri-shell` feature is enabled. Without the feature,
// this build script is a no-op so that the MCP-only build does not require the
// system webview (webkit2gtk-4.1 on Linux) to be installed.

fn main() {
    #[cfg(feature = "tauri-shell")]
    {
        tauri_build::build()
    }
    // No-op when tauri-shell is disabled.
    #[cfg(not(feature = "tauri-shell"))]
    {}
}
