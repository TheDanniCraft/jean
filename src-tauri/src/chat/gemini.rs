use std::io::{BufRead, BufReader, Write};
use std::path::Path;

use crate::http_server::EmitExt;

use super::types::{ContentBlock, ToolCall, UsageData};

pub struct GeminiResponse {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub content_blocks: Vec<ContentBlock>,
    pub usage: Option<UsageData>,
    pub session_id: Option<String>,
}

fn extract_usage(v: &serde_json::Value) -> Option<UsageData> {
    // In stream-json, usage is in the "stats" field of the "result" event
    let stats = if v.get("type") == Some(&serde_json::json!("result")) {
        v.get("stats")
    } else {
        v.get("usage")
    };

    stats.and_then(|u| {
        let input_tokens = u.get("input_tokens")
            .or_else(|| u.get("inputTokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let output_tokens = u.get("output_tokens")
            .or_else(|| u.get("outputTokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        
        if input_tokens == 0 && output_tokens == 0 {
            return None;
        }

        Some(UsageData {
            input_tokens,
            output_tokens,
            ..Default::default()
        })
    })
}

#[derive(serde::Serialize, Clone)]
struct ChunkEvent {
    session_id: String,
    worktree_id: String,
    content: String,
}

#[derive(serde::Serialize, Clone)]
struct ToolUseEvent {
    session_id: String,
    worktree_id: String,
    id: String,
    name: String,
    input: serde_json::Value,
}

#[derive(serde::Serialize, Clone)]
struct ToolResultEvent {
    session_id: String,
    worktree_id: String,
    tool_use_id: String,
    output: String,
}

#[derive(serde::Serialize, Clone)]
struct ToolBlockEvent {
    session_id: String,
    worktree_id: String,
    tool_call_id: String,
}

#[derive(serde::Serialize, Clone)]
struct DoneEvent {
    session_id: String,
    worktree_id: String,
    waiting_for_plan: bool,
}

#[derive(serde::Serialize, Clone)]
struct GeminiPlanModeChangedEvent {
    active: bool,
}

#[derive(serde::Serialize, Clone)]
struct GeminiPlanUpdatedEvent {
    content: String,
}

pub fn execute_gemini(
    app: &tauri::AppHandle,
    session_id: &str,
    worktree_id: &str,
    working_dir: &Path,
    prompt: &str,
    output_file: &Path,
    resume_id: Option<&str>,
    execution_mode: Option<&str>,
    model: Option<&str>,
) -> Result<GeminiResponse, String> {
    let mut child = crate::gemini_cli::spawn_gemini_process(app, prompt, working_dir, resume_id, execution_mode, model)?;
    
    let stdout = child.stdout.take().ok_or("Failed to open Gemini stdout")?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(output_file)
        .map_err(|e| format!("Failed opening Gemini run log: {e}"))?;

    let mut content = String::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();
    let mut content_blocks: Vec<ContentBlock> = Vec::new();
    let mut usage = Some(UsageData::default());
    let mut gemini_session_id = None;

    let reader = BufReader::new(stdout);
    for line in reader.lines() {
        let raw_line = line.map_err(|e| format!("Failed reading Gemini output: {e}"))?;
        if raw_line.trim().is_empty() {
            continue;
        }
        
        // Write to log file immediately
        let _ = writeln!(file, "{raw_line}");
        let _ = file.flush();

        // Skip leading non-JSON noise
        let json_start = match raw_line.find('{') {
            Some(idx) => idx,
            None => {
                continue;
            }
        };

        let parsed: serde_json::Value = match serde_json::from_str(&raw_line[json_start..]) {
            Ok(v) => v,
            Err(_) => {
                continue;
            }
        };

        let event_type = parsed.get("type").and_then(|v| v.as_str()).unwrap_or("");

        match event_type {
            "init" => {
                if let Some(id) = parsed.get("session_id").or_else(|| parsed.get("sessionId")).and_then(|v| v.as_str()) {
                    gemini_session_id = Some(id.to_string());
                }
            }
            "message" => {
                let role = parsed.get("role").and_then(|v| v.as_str()).unwrap_or("");
                if role == "assistant" {
                    if let Some(text) = parsed.get("content").and_then(|v| v.as_str()) {
                        content.push_str(text);
                        
                        // Like Codex, we only push a Text content block if the last block isn't text
                        match content_blocks.last_mut() {
                            Some(ContentBlock::Text { text: existing_text }) => {
                                existing_text.push_str(text);
                            }
                            _ => {
                                content_blocks.push(ContentBlock::Text { text: text.to_string() });
                            }
                        }

                        let _ = app.emit_all(
                            "chat:chunk",
                            &ChunkEvent {
                                session_id: session_id.to_string(),
                                worktree_id: worktree_id.to_string(),
                                content: text.to_string(),
                            },
                        );
                    }
                }
            }
            "tool_use" => {
                let name = parsed.get("name")
                    .or_else(|| parsed.get("tool"))
                    .or_else(|| parsed.get("tool_name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                
                let id = parsed.get("id")
                    .or_else(|| parsed.get("tool_id"))
                    .or_else(|| parsed.get("toolId"))
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| "unknown");

                let input = parsed.get("input")
                    .or_else(|| parsed.get("args"))
                    .or_else(|| parsed.get("arguments"))
                    .cloned()
                    .unwrap_or(serde_json::json!({}));

                tool_calls.push(ToolCall {
                    id: id.to_string(),
                    name: name.to_string(),
                    input: input.clone(),
                    output: None,
                    parent_tool_use_id: None,
                });
                content_blocks.push(ContentBlock::ToolUse {
                    tool_call_id: id.to_string(),
                });

                // Emit tool block event (placeholder)
                let _ = app.emit_all(
                    "chat:tool_block",
                    &ToolBlockEvent {
                        session_id: session_id.to_string(),
                        worktree_id: worktree_id.to_string(),
                        tool_call_id: id.to_string(),
                    },
                );

                // Emit tool use event to frontend
                let _ = app.emit_all(
                    "chat:tool_use",
                    &ToolUseEvent {
                        session_id: session_id.to_string(),
                        worktree_id: worktree_id.to_string(),
                        id: id.to_string(),
                        name: name.to_string(),
                        input: input.clone(),
                    },
                );

                if name == "enter-plan-mode" {
                    let _ = app.emit_all(
                        "gemini:plan_mode_changed",
                        &GeminiPlanModeChangedEvent { active: true },
                    );
                } else if name == "exit-plan-mode" {
                    let _ = app.emit_all(
                        "gemini:plan_mode_changed",
                        &GeminiPlanModeChangedEvent { active: false },
                    );
                }
            }
            "tool_result" => {
                let id = parsed.get("id")
                    .or_else(|| parsed.get("tool_id"))
                    .or_else(|| parsed.get("toolId"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                
                let output_text = parsed.get("output")
                    .or_else(|| parsed.get("content"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                if let Some(tc) = tool_calls.iter_mut().find(|t| t.id == id) {
                    tc.output = Some(output_text.to_string());
                }

                // Emit tool result event to frontend
                let _ = app.emit_all(
                    "chat:tool_result",
                    &ToolResultEvent {
                        session_id: session_id.to_string(),
                        worktree_id: worktree_id.to_string(),
                        tool_use_id: id.to_string(),
                        output: output_text.to_string(),
                    },
                );
            }
            "result" => {
                if let Some(new_usage) = extract_usage(&parsed) {
                    usage = Some(new_usage);
                }
                if let Some(resp) = parsed.get("response").and_then(|v| v.as_str()) {
                    if content.is_empty() {
                        content = resp.to_string();
                        content_blocks.push(ContentBlock::Text { text: resp.to_string() });
                    }
                }
            }
            _ => {
                if let Some(role) = parsed.get("role").and_then(|v| v.as_str()) {
                    if role == "assistant" {
                        if let Some(text) = parsed.get("content").or_else(|| parsed.get("text")).and_then(|v| v.as_str()) {
                            content.push_str(text);
                            match content_blocks.last_mut() {
                                Some(ContentBlock::Text { text: existing_text }) => {
                                    existing_text.push_str(text);
                                }
                                _ => {
                                    content_blocks.push(ContentBlock::Text { text: text.to_string() });
                                }
                            }
                        }
                    }
                }
                
                if let Some(new_usage) = extract_usage(&parsed) {
                    usage = Some(new_usage);
                }
            }
        }

        if let Some(plan) = parsed.get("plan").or_else(|| parsed.get("plan_content")) {
            if let Some(plan_text) = plan.as_str() {
                let _ = app.emit_all("gemini:plan_updated", &GeminiPlanUpdatedEvent { content: plan_text.to_string() });
            } else if let Some(plan_text) = plan.get("content").and_then(|v| v.as_str()) {
                let _ = app.emit_all("gemini:plan_updated", &GeminiPlanUpdatedEvent { content: plan_text.to_string() });
            }
        }
    }

    let status = child.wait().map_err(|e| format!("Gemini CLI process error: {e}"))?;
    if !status.success() {
        return Err(format!("Gemini CLI failed with status {status}"));
    }

    let _ = app.emit_all(
        "chat:done",
        &DoneEvent {
            session_id: session_id.to_string(),
            worktree_id: worktree_id.to_string(),
            waiting_for_plan: false,
        },
    );

    Ok(GeminiResponse {
        content,
        tool_calls,
        content_blocks,
        usage,
        session_id: gemini_session_id,
    })
}
