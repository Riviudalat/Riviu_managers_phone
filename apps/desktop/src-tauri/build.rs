fn main() {
    println!("cargo:rerun-if-env-changed=RIVIU_DEFAULT_AGENT_MODE");
    if let Ok(mode) = std::env::var("RIVIU_DEFAULT_AGENT_MODE") {
        println!("cargo:rustc-env=RIVIU_DEFAULT_AGENT_MODE={mode}");
    }
    tauri_build::build()
}
