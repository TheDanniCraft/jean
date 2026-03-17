//! Configuration and path management for the Gemini CLI

use std::path::PathBuf;
use tauri::{AppHandle, Manager};

pub const CLI_DIR_NAME: &str = "gemini-cli";

#[cfg(windows)]
pub const CLI_BINARY_NAME: &str = "gemini.cmd";
#[cfg(not(windows))]
pub const CLI_BINARY_NAME: &str = "gemini";

pub fn get_cli_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data directory: {e}"))?;
    Ok(app_data_dir.join(CLI_DIR_NAME))
}

pub fn get_cli_binary_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(get_cli_dir(app)?
        .join("node_modules")
        .join(".bin")
        .join(CLI_BINARY_NAME))
}

pub fn resolve_cli_binary(app: &AppHandle) -> PathBuf {
    get_cli_binary_path(app).unwrap_or_else(|_| {
        PathBuf::from(CLI_DIR_NAME)
            .join("node_modules")
            .join(".bin")
            .join(CLI_BINARY_NAME)
    })
}

pub fn ensure_cli_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let cli_dir = get_cli_dir(app)?;
    std::fs::create_dir_all(&cli_dir)
        .map_err(|e| format!("Failed to create CLI directory: {e}"))?;
    Ok(cli_dir)
}

