//! Gemini CLI helpers and Tauri commands.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use tauri::AppHandle;

use super::config::{ensure_cli_dir, resolve_cli_binary};
use crate::http_server::EmitExt;
use crate::platform::silent_command;

const GEMINI_NPM_PACKAGE: &str = "@google/gemini-cli";
const GEMINI_NPM_REGISTRY_URL: &str = "https://registry.npmjs.org/@google/gemini-cli";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiCliStatus {
    pub installed: bool,
    pub version: Option<String>,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiAuthStatus {
    pub authenticated: bool,
    pub error: Option<String>,
    pub email: Option<String>,
    pub project: Option<String>,
    pub tier: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiReleaseInfo {
    pub version: String,
    pub tag_name: String,
    pub published_at: String,
    pub prerelease: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct GeminiInstallProgress {
    pub stage: String,
    pub message: String,
    pub percent: u8,
}

#[derive(Debug, Deserialize)]
struct NpmPackageInfo {
    versions: HashMap<String, serde_json::Value>,
    time: HashMap<String, String>,
    #[serde(rename = "dist-tags")]
    dist_tags: HashMap<String, String>,
}

fn emit_progress(app: &AppHandle, stage: &str, message: &str, percent: u8) {
    let _ = app.emit_all(
        "gemini-cli:install-progress",
        &GeminiInstallProgress {
            stage: stage.to_string(),
            message: message.to_string(),
            percent,
        },
    );
}

fn selected_gemini_model_from_prefs(app: &AppHandle) -> String {
    crate::get_preferences_path(app)
        .ok()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|contents| serde_json::from_str::<crate::AppPreferences>(&contents).ok())
        .map(|prefs| prefs.selected_gemini_model)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "auto-gemini-3".to_string())
}

fn npm_program() -> &'static str {
    #[cfg(windows)]
    {
        "npm.cmd"
    }
    #[cfg(not(windows))]
    {
        "npm"
    }
}

fn parse_version(version: &str) -> Vec<u32> {
    version
        .split('.')
        .filter_map(|s| s.parse::<u32>().ok())
        .collect()
}

async fn fetch_latest_gemini_version() -> Result<String, String> {
    let client = reqwest::Client::new();
    let response = client
        .get(GEMINI_NPM_REGISTRY_URL)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch Gemini versions: {e}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "npm registry returned status: {}",
            response.status()
        ));
    }

    let package_info: NpmPackageInfo = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse npm response: {e}"))?;

    package_info
        .dist_tags
        .get("latest")
        .cloned()
        .ok_or_else(|| "No latest tag found for Gemini CLI".to_string())
}

/// Ensure Gemini settings exist and security.folderTrust.enabled is false
fn ensure_gemini_settings() -> Result<(), String> {
    let home = dirs::home_dir().ok_or_else(|| "No home directory found".to_string())?;
    let gemini_config_dir = home.join(".gemini");
    if !gemini_config_dir.exists() {
        if let Err(e) = std::fs::create_dir_all(&gemini_config_dir) {
            log::warn!("Failed to create ~/.gemini directory: {e}");
            return Ok(());
        }
    }

    let settings_path = gemini_config_dir.join("settings.json");
    let mut settings: serde_json::Value = if settings_path.exists() {
        let content = std::fs::read_to_string(&settings_path).unwrap_or_default();
        serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    // Ensure security.folderTrust.enabled is false to skip interactive prompts
    if !settings.is_object() {
        settings = serde_json::json!({});
    }

    let security = settings
        .as_object_mut()
        .unwrap()
        .entry("security")
        .or_insert(serde_json::json!({}));
    
    if !security.is_object() {
        *security = serde_json::json!({});
    }

    let folder_trust = security
        .as_object_mut()
        .unwrap()
        .entry("folderTrust")
        .or_insert(serde_json::json!({}));

    if !folder_trust.is_object() {
        *folder_trust = serde_json::json!({});
    }

    folder_trust
        .as_object_mut()
        .unwrap()
        .insert("enabled".to_string(), serde_json::json!(false));

    if let Ok(content) = serde_json::to_string_pretty(&settings) {
        let _ = std::fs::write(settings_path, content);
    }

    Ok(())
}

/// Spawn a Gemini CLI process for a chat prompt.
///
/// Uses `gemini --model <model>` and writes the prompt to stdin.
pub fn spawn_gemini_process(
    app: &AppHandle,
    prompt: &str,
    working_dir: &Path,
    resume_id: Option<&str>,
    execution_mode: Option<&str>,
    model: Option<&str>,
) -> Result<std::process::Child, String> {
    let _ = ensure_gemini_settings();
    let model_string = match model {
        Some(m) if !m.is_empty() => m.to_string(),
        _ => selected_gemini_model_from_prefs(app),
    };
    let binary_path = resolve_cli_binary(app);

    let mut cmd = std::process::Command::new(&binary_path);
    cmd.arg("--model")
        .arg(&model_string)
        .arg("--output-format")
        .arg("stream-json")
        .env("CI", "true")
        .current_dir(working_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // Map execution modes to Gemini's --approval-mode
    // Jean: plan, build, yolo
    // Gemini: plan, default, auto_edit, yolo
    let approval_mode = match execution_mode {
        Some("plan") => "plan",
        Some("yolo") => "yolo",
        _ => "default", // "build" mode maps to default (prompt for approval)
    };
    cmd.arg("--approval-mode").arg(approval_mode);

    if let Some(id) = resume_id {
        if !id.is_empty() {
            cmd.arg("--resume").arg(id);
        }
    }

    let mut child = cmd.spawn()
        .map_err(|e| format!("Failed to spawn Gemini CLI: {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin
            .write_all(prompt.as_bytes())
            .map_err(|e| format!("Failed to write Gemini prompt to stdin: {e}"))?;
    }

    Ok(child)
}

#[tauri::command]
pub async fn check_gemini_cli_installed(app: AppHandle) -> Result<GeminiCliStatus, String> {
    let _ = ensure_gemini_settings();
    let binary_path = resolve_cli_binary(&app);
    if !binary_path.exists() {
        return Ok(GeminiCliStatus {
            installed: false,
            version: None,
            path: None,
        });
    }

    let version = match silent_command(&binary_path).arg("--version").output() {
        Ok(output) if output.status.success() => {
            let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if raw.is_empty() {
                None
            } else {
                Some(
                    raw.split_whitespace()
                        .last()
                        .unwrap_or(raw.as_str())
                        .to_string(),
                )
            }
        }
        _ => None,
    };

    Ok(GeminiCliStatus {
        installed: true,
        version,
        path: Some(binary_path.to_string_lossy().to_string()),
    })
}

#[tauri::command]
pub async fn check_gemini_cli_auth(app: AppHandle) -> Result<GeminiAuthStatus, String> {
    let _ = ensure_gemini_settings();
    let binary_path = resolve_cli_binary(&app);
    if !binary_path.exists() {
        return Ok(GeminiAuthStatus {
            authenticated: false,
            error: Some("Gemini CLI not installed".to_string()),
            email: None,
            project: None,
            tier: None,
        });
    }

    // --- Fast Path: Check for credentials on disk or in env ---
    let home = dirs::home_dir();
    let has_cached_google = home.as_ref()
        .map(|h| h.join(".gemini").join("google_accounts.json").exists())
        .unwrap_or(false);
    
    let has_api_key_env = std::env::var("GEMINI_API_KEY").is_ok() 
        || std::env::var("GOOGLE_API_KEY").is_ok();

    // Run auth status check with a timeout and null stdin to prevent UI hangs.
    let output_result = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let path = binary_path.clone();
        tokio::task::spawn_blocking(move || {
            silent_command(&path)
                .args(["--prompt", "/about", "--output-format", "json"])
                .stdin(std::process::Stdio::null())
                .env("CI", "true")
                .output()
        })
        .await
    })
    .await;

    let output = match output_result {
        Ok(Ok(Ok(o))) => o,
        Ok(Ok(Err(e))) => return Err(format!("Failed to execute Gemini CLI: {e}")),
        Ok(Err(e)) => return Err(format!("Task join error: {e}")),
        Err(_) => {
            if has_cached_google || has_api_key_env {
                return Ok(GeminiAuthStatus {
                    authenticated: true,
                    error: None,
                    email: None,
                    project: None,
                    tier: None,
                });
            }
            return Ok(GeminiAuthStatus {
                authenticated: false,
                error: Some("Gemini CLI auth check timed out".to_string()),
                email: None,
                project: None,
                tier: None,
            });
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let combined_raw = format!("{}\n{}", stdout, stderr);
    let combined_lower = combined_raw.to_lowercase();
    
    // Parse the JSON response - find the first '{' to skip leading noise/logs
    let json_value: Option<serde_json::Value> = if let Some(start_idx) = stdout.find('{') {
        serde_json::from_str(&stdout[start_idx..]).ok()
    } else {
        None
    };
    
    // Check for authentication markers
    let log_mentions_auth = combined_lower.contains("signed in") 
        || combined_lower.contains("authenticated") 
        || combined_lower.contains("logged in")
        || combined_lower.contains("cached account")
        || combined_lower.contains("cached credentials")
        || combined_lower.contains("retrieved cached")
        || combined_lower.contains("api key")
        || combined_lower.contains("vertex ai")
        || combined_lower.contains("gcp project");

    let mut email = None;
    let mut project = None;
    let mut tier = None;

    let authenticated = if let Some(ref json) = json_value {
        let auth_type = json.get("selectedAuthType")
            .or_else(|| json.get("authType"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        
        let is_authed_by_type = matches!(auth_type, "oauth-personal" | "gemini-api-key" | "vertex-ai");

        email = json.get("userEmail")
            .or_else(|| json.get("email"))
            .or_else(|| json.get("cachedAccount"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        
        project = json.get("gcpProject")
            .or_else(|| json.get("projectId"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        
        tier = json.get("tier").and_then(|v| v.as_str()).map(|s| s.to_string());

        let has_identity = email.is_some() || project.is_some();

        let text_mentions_auth = json.get("text")
            .and_then(|v| v.as_str())
            .map(|s| s.to_lowercase())
            .map(|s| {
                s.contains("signed in") || s.contains("authenticated") || s.contains("logged in") || 
                s.contains("cached account") || s.contains("cached credentials") || s.contains("retrieved cached")
            })
            .unwrap_or(false);

        is_authed_by_type || has_identity || text_mentions_auth || log_mentions_auth
    } else {
        log_mentions_auth || (has_cached_google && output.status.success())
    };

    if authenticated {
        // --- Identity Refinement: Try to extract email from logs if missing from JSON ---
        if email.is_none() {
            // 1. Try to find the specific cachedAccount log pattern: "cachedAccount: 'user@gmail.com'"
            let re_log = regex::Regex::new(r"cachedAccount:\s*'([^']+)'").unwrap();
            if let Some(caps) = re_log.captures(&combined_raw) {
                email = Some(caps.get(1).unwrap().as_str().to_string());
            }
            
            // 2. Fallback: Generic email extraction from anywhere in the output
            if email.is_none() {
                let re_email = regex::Regex::new(r"[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}").unwrap();
                if let Some(caps) = re_email.captures(&combined_raw) {
                    email = Some(caps.get(0).unwrap().as_str().to_string());
                }
            }
        }

        return Ok(GeminiAuthStatus {
            authenticated: true,
            error: None,
            email,
            project,
            tier,
        });
    }

    Ok(GeminiAuthStatus {
        authenticated: false,
        error: Some(if !stderr.is_empty() { 
            stderr 
        } else if !stdout.is_empty() { 
            stdout 
        } else { 
            "Gemini CLI is not authenticated".to_string() 
        }),
        email: None,
        project: None,
        tier: None,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiUsageBucket {
    pub model_id: String,
    pub remaining_fraction: f64,
    pub usage_percent: f64,
    pub reset_time: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiUsageSnapshot {
    pub quotas: Vec<GeminiUsageBucket>,
    pub fetched_at: u64,
    pub plan_name: Option<String>,
    pub tier: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GeminiQuotaBucketRaw {
    #[serde(rename = "modelId")]
    model_id: String,
    #[serde(rename = "remainingFraction")]
    remaining_fraction: f64,
    #[serde(rename = "resetTime")]
    reset_time: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GeminiQuotaResponseRaw {
    buckets: Option<Vec<GeminiQuotaBucketRaw>>,
}

#[derive(Debug, Deserialize)]
struct GeminiTierResponseRaw {
    tier: Option<String>,
    #[serde(rename = "cloudaicompanionProject")]
    project: Option<String>,
    #[serde(rename = "planName")]
    plan_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OAuthCreds {
    access_token: String,
    expiry_date: Option<u64>, // Unix ms
}

#[tauri::command]
pub async fn get_gemini_usage(app: AppHandle) -> Result<GeminiUsageSnapshot, String> {
    let home = dirs::home_dir().ok_or_else(|| "No home directory found".to_string())?;
    let creds_path = home.join(".gemini").join("oauth_creds.json");
    
    if !creds_path.exists() {
        return Err("Gemini CLI credentials not found. Please log in first.".to_string());
    }

    let creds_json = std::fs::read_to_string(&creds_path)
        .map_err(|e| format!("Failed to read credentials: {e}"))?;
    let creds: OAuthCreds = serde_json::from_str(&creds_json)
        .map_err(|e| format!("Failed to parse credentials: {e}"))?;

    let client = reqwest::Client::new();
    let auth_header = format!("Bearer {}", creds.access_token);

    // 1. Fetch Tier/Project
    let tier_res = client.post("https://cloudcode-pa.googleapis.com/v1internal:loadCodeAssist")
        .header("Authorization", &auth_header)
        .header("Content-Type", "application/json")
        .header("User-Agent", "jean/1.0")
        .json(&serde_json::json!({
            "metadata": {
                "ideType": "IDE_UNSPECIFIED",
                "platform": "PLATFORM_UNSPECIFIED",
                "pluginType": "GEMINI"
            }
        }))
        .send()
        .await
        .map_err(|e| format!("Tier request failed: {e}"))?;

    let tier_data: GeminiTierResponseRaw = tier_res.json()
        .await
        .map_err(|e| format!("Failed to parse tier response: {e}"))?;

    // 2. Fetch Quotas
    let mut quota_body = serde_json::json!({});
    if let Some(project_id) = tier_data.project {
        quota_body = serde_json::json!({ "project": project_id });
    }

    let quota_res = client.post("https://cloudcode-pa.googleapis.com/v1internal:retrieveUserQuota")
        .header("Authorization", &auth_header)
        .header("Content-Type", "application/json")
        .header("User-Agent", "jean/1.0")
        .json(&quota_body)
        .send()
        .await
        .map_err(|e| format!("Quota request failed: {e}"))?;

    let quota_data: GeminiQuotaResponseRaw = quota_res.json()
        .await
        .map_err(|e| format!("Failed to parse quota response: {e}"))?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let buckets = quota_data.buckets.unwrap_or_default()
        .into_iter()
        .map(|b| {
            let reset_time = b.reset_time.and_then(|t| {
                chrono::DateTime::parse_from_rfc3339(&t)
                    .ok()
                    .map(|dt| dt.timestamp() as u64)
            });
            GeminiUsageBucket {
                model_id: b.model_id,
                usage_percent: (1.0 - b.remaining_fraction) * 100.0,
                remaining_fraction: b.remaining_fraction,
                reset_time,
            }
        })
        .collect();

    Ok(GeminiUsageSnapshot {
        quotas: buckets,
        fetched_at: now,
        plan_name: tier_data.plan_name,
        tier: tier_data.tier,
    })
}

#[tauri::command]
pub async fn get_available_gemini_versions() -> Result<Vec<GeminiReleaseInfo>, String> {
    let client = reqwest::Client::new();
    let response = client
        .get(GEMINI_NPM_REGISTRY_URL)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch Gemini versions: {e}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "npm registry returned status: {}",
            response.status()
        ));
    }

    let package_info: NpmPackageInfo = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse npm response: {e}"))?;

    let latest_version = package_info
        .dist_tags
        .get("latest")
        .ok_or_else(|| "No latest tag found for Gemini CLI".to_string())?;
    let latest_parts = parse_version(latest_version);

    let mut versions: Vec<GeminiReleaseInfo> = package_info
        .versions
        .keys()
        .filter(|version| {
            if version.contains('-') {
                return false;
            }
            let parts = parse_version(version);
            parts <= latest_parts
        })
        .map(|version| GeminiReleaseInfo {
            version: version.clone(),
            tag_name: format!("v{version}"),
            published_at: package_info.time.get(version).cloned().unwrap_or_default(),
            prerelease: false,
        })
        .collect();

    versions.sort_by(|a, b| {
        let a_parts = parse_version(&a.version);
        let b_parts = parse_version(&b.version);
        b_parts.cmp(&a_parts)
    });
    versions.truncate(5);
    Ok(versions)
}

#[tauri::command]
pub async fn install_gemini_cli(app: AppHandle, version: Option<String>) -> Result<(), String> {
    emit_progress(&app, "starting", "Preparing installation...", 0);

    let cli_dir = ensure_cli_dir(&app)?;
    let version = match version {
        Some(v) if !v.trim().is_empty() => v,
        _ => fetch_latest_gemini_version().await?,
    };

    emit_progress(&app, "downloading", "Installing Gemini CLI via npm...", 35);
    let package_spec = format!("{GEMINI_NPM_PACKAGE}@{version}");
    let cli_dir_str = cli_dir
        .to_str()
        .ok_or_else(|| "Invalid Gemini CLI directory path".to_string())?;

    let output = silent_command(npm_program())
        .args([
            "install",
            "--prefix",
            cli_dir_str,
            "--no-audit",
            "--no-fund",
            "--loglevel",
            "error",
            package_spec.as_str(),
        ])
        .output()
        .map_err(|e| format!("Failed to run npm install for Gemini CLI: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        return Err(if !stderr.is_empty() {
            format!("Failed to install Gemini CLI: {stderr}")
        } else if !stdout.is_empty() {
            format!("Failed to install Gemini CLI: {stdout}")
        } else {
            "Failed to install Gemini CLI via npm".to_string()
        });
    }

    emit_progress(&app, "verifying", "Verifying installation...", 80);
    let status = check_gemini_cli_installed(app.clone()).await?;
    if !status.installed {
        return Err("Gemini CLI installation finished but binary not found".to_string());
    }

    emit_progress(&app, "complete", "Installation complete!", 100);
    Ok(())
}
