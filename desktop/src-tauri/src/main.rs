mod commands;

use tracing_subscriber::EnvFilter;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(commands::AppState::new())
        .invoke_handler(tauri::generate_handler![
            commands::greet,
            commands::run_agent,
            commands::get_models,
            commands::get_config,
            commands::get_agents,
            commands::chat,
        ])
        .run(tauri::generate_context!())
        .expect("error while running ForgeCode Desktop");
}
