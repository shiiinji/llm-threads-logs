use ai_log_exporter::{git_project_name, safe_id, safe_name, with_lock_file};
use anyhow::{Context, Result};
use serde_json::Value;
use std::{
    env, fs,
    io::{self, Read},
    path::PathBuf,
    process::Command,
};

fn main() -> Result<()> {
    // SessionEnd hook payload arrives on stdin as JSON
    let mut stdin = String::new();
    io::stdin()
        .read_to_string(&mut stdin)
        .context("failed to read stdin")?;

    let stdin = stdin.trim();
    if stdin.is_empty() {
        return Ok(());
    }

    let payload: Value =
        serde_json::from_str(stdin).context("failed to parse hook JSON from stdin")?;

    let session_id = payload
        .get("session_id")
        .and_then(|v| v.as_str())
        .context("missing session_id in hook payload")?;
    let session_id_safe = safe_id(session_id, "unknown-session");

    let cwd = payload.get("cwd").and_then(|v| v.as_str()).unwrap_or(".");

    let vault = env::var("OBSIDIAN_VAULT").context("Missing OBSIDIAN_VAULT env var")?;
    let ai_root = env::var("OBSIDIAN_AI_ROOT").context("Missing OBSIDIAN_AI_ROOT env var")?;

    let project = safe_name(&git_project_name(cwd));

    // Find the MD file for this session
    let vault_path = PathBuf::from(&vault);
    let md_dir = vault_path
        .join(&ai_root)
        .join("Claude Code")
        .join(&project)
        .join("Threads");

    let lock_path = md_dir.join(format!(".lock_{session_id_safe}"));
    let md_content = with_lock_file(&lock_path, || {
        let md_path = find_md_by_session_id(&md_dir, &session_id_safe);
        let md_path = match md_path {
            Some(p) => p,
            None => {
                eprintln!("MD file not found for session: {}", session_id);
                return Ok(None);
            }
        };
        let md_content = fs::read_to_string(&md_path).context("failed to read MD file")?;
        Ok(Some((md_path, md_content)))
    })?;
    let (md_path, md_content) = match md_content {
        Some(v) => v,
        None => return Ok(()),
    };

    // Extract user messages from MD content
    let user_messages = extract_user_messages(&md_content);
    if user_messages.is_empty() {
        return Ok(());
    }

    // Review with LLM and get skill proposals
    let proposals = match review_with_llm(&user_messages, &project)? {
        Some(p) => p,
        None => {
            // No skill proposals - don't create file
            return Ok(());
        }
    };

    // Save proposals to file
    let proposals_dir = vault_path.join(&ai_root).join("skill_proposals");
    fs::create_dir_all(&proposals_dir).context("failed to create proposals dir")?;

    let proposal_file = proposals_dir.join(format!("{session_id_safe}.md"));
    let proposal_content = format!(
        r#"---
session_id: "{}"
project: "{}"
reviewed_file: "{}"
---

# Skill 提案

{}
"#,
        session_id,
        project,
        md_path.display(),
        proposals
    );

    fs::write(&proposal_file, proposal_content).context("failed to write proposal file")?;

    eprintln!("Skill proposals saved to: {}", proposal_file.display());
    Ok(())
}

fn find_md_by_session_id(md_dir: &PathBuf, session_id: &str) -> Option<PathBuf> {
    if !md_dir.exists() {
        return None;
    }

    let entries = fs::read_dir(md_dir).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        if let Some(name_str) = name.to_str() {
            if name_str.contains(session_id) && name_str.ends_with(".md") {
                return Some(entry.path());
            }
        }
    }
    None
}

pub fn extract_user_messages(md_content: &str) -> Vec<String> {
    let mut messages = Vec::new();
    let mut current_message = String::new();
    let mut in_user_block = false;

    for line in md_content.lines() {
        if line.starts_with("### ") && line.contains(" User") {
            // Start of a user message block
            if !current_message.trim().is_empty() {
                messages.push(current_message.trim().to_string());
            }
            current_message = String::new();
            in_user_block = true;
        } else if line.starts_with("### ") && line.contains(" Assistant") {
            // End of user block, start of assistant block
            if in_user_block && !current_message.trim().is_empty() {
                messages.push(current_message.trim().to_string());
            }
            current_message = String::new();
            in_user_block = false;
        } else if in_user_block {
            current_message.push_str(line);
            current_message.push('\n');
        }
    }

    // Don't forget the last message if we ended in a user block
    if in_user_block && !current_message.trim().is_empty() {
        messages.push(current_message.trim().to_string());
    }

    messages
}

fn review_with_llm(user_messages: &[String], project: &str) -> Result<Option<String>> {
    let messages_text = user_messages.join("\n\n---\n\n");

    let prompt = format!(
        r#"プロジェクト「{}」のコーディングセッションでのユーザー指示をレビューしています。

以下のユーザーメッセージを分析し、再利用可能な Skill（AIアシスタント向けのカスタム指示/ワークフロー）として自動化できるパターンを特定してください。

ユーザーメッセージ:
{}

このセッションの内容から、今後のセッションで役立つ Skill を提案してください。各 Skill について:
1. 名前: 短い説明的な名前
2. 目的: 何を自動化・簡略化するか
3. 使用条件: いつ使うべきか
4. 実装ヒント: 主要なステップやパターン

明確なパターンがある場合のみ提案してください。セッションが単純すぎる、または一回限りの作業の場合は「NONE」とだけ出力してください。

出力は日本語の Markdown 形式で。"#,
        project, messages_text
    );

    let tmp_dir = std::env::temp_dir();
    let tmp_file = tmp_dir.join(format!("review_{}.txt", std::process::id()));

    let tmp_file_str = match tmp_file.to_str() {
        Some(s) => s,
        None => return Ok(None),
    };

    let status = match Command::new("codex")
        .args(["exec", "-c", "notify=[]", "-o", tmp_file_str, &prompt])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
    {
        Ok(s) => s,
        Err(_) => {
            let _ = fs::remove_file(&tmp_file);
            return Ok(None);
        }
    };

    if !status.success() {
        let _ = fs::remove_file(&tmp_file);
        return Ok(None);
    }

    let result = fs::read_to_string(&tmp_file).unwrap_or_default();
    let _ = fs::remove_file(&tmp_file);

    // Check if LLM returned "NONE" (no proposals)
    let trimmed = result.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
        return Ok(None);
    }

    Ok(Some(result))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
