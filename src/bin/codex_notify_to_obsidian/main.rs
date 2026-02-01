use ai_log_exporter::{
    find_md_file_containing_id, generate_title, git_project_name, safe_id, safe_name, with_lock_file,
    yaml_quote,
};
use anyhow::{Context, Result};
use chrono::{Local, SecondsFormat};
use serde_json::Value;
use std::{
    env, fs,
    path::{Path, PathBuf},
};

pub const BEGIN: &str = "<!-- BEGIN AUTO TURNS -->";
pub const END: &str = "<!-- END AUTO TURNS -->";

fn main() -> Result<()> {
    let payload_arg = env::args().nth(1);
    let payload_arg = match payload_arg {
        Some(p) if !p.trim().is_empty() => p,
        _ => return Ok(()),
    };

    // Codex has historically passed the notification as a JSON string in argv[1],
    // but support treating argv[1] as a file path (JSON) as well.
    let notification: Value = match serde_json::from_str(&payload_arg) {
        Ok(v) => v,
        Err(e_json) => match fs::read_to_string(&payload_arg) {
            Ok(file_text) => {
                serde_json::from_str(&file_text)
                    .context("failed to parse notify JSON from file path in argv[1]")?
            }
            Err(e_file) => {
                return Err(e_json).with_context(|| {
                    format!(
                        "failed to parse notify JSON from argv[1] and failed to read it as a file path: {}",
                        e_file
                    )
                })
            }
        },
    };

    if !should_process_notification(&notification) {
        return Ok(());
    }

    let thread_id = notification_str(&notification, &["thread-id", "thread_id", "threadId"])
        .unwrap_or("unknown-thread");
    let thread_id_safe = safe_id(thread_id, "unknown-thread");

    let turn_id = notification_str(&notification, &["turn-id", "turn_id", "turnId"]).unwrap_or("");

    let cwd = notification_str(&notification, &["cwd"]).unwrap_or(".");

    let input_messages = notification
        .get("input-messages")
        .or_else(|| notification.get("input_messages"))
        .or_else(|| notification.get("inputMessages"))
        .cloned()
        .unwrap_or(Value::Null);

    let last_assistant =
        notification_str(&notification, &["last-assistant-message", "last_assistant_message", "lastAssistantMessage"])
            .unwrap_or("");

    let vault = env::var("OBSIDIAN_VAULT").context("Missing OBSIDIAN_VAULT env var")?;
    let ai_root = env::var("OBSIDIAN_AI_ROOT").context("Missing OBSIDIAN_AI_ROOT env var")?;

    let project = safe_name(&git_project_name(cwd));

    let vault_path = PathBuf::from(&vault);
    let base_dir = vault_path.join(&ai_root).join("Codex").join(&project);
    let md_dir = base_dir.join("Threads");
    fs::create_dir_all(&md_dir).context("failed to create md_dir")?;

    let lock_path = md_dir.join(format!(".lock_{thread_id_safe}"));
    with_lock_file(&lock_path, || {
        let first_user_msg = extract_first_user_msg(&input_messages);
        let md_path = find_or_create_md_path(&md_dir, &thread_id_safe, first_user_msg.as_deref())
            .context("failed to find or create md path")?;
        let mut text = if md_path.exists() {
            fs::read_to_string(&md_path).context("failed to read existing md")?
        } else {
            build_codex_note_skeleton(&project, thread_id, cwd)
        };

        text = ensure_turns_block(&text);

        if !turn_id.is_empty() {
            let sentinel = format!("<!-- turn-id:{turn_id} -->");
            if text.contains(&sentinel) {
                return Ok(());
            }
            let block = build_turn_block(turn_id, &input_messages, last_assistant, &sentinel);
            text = insert_before_end(&text, &block);
        } else {
            let sentinel = "<!-- turn-id:(missing) -->".to_string();
            let block =
                build_turn_block("(no turn-id)", &input_messages, last_assistant, &sentinel);
            text = insert_before_end(&text, &block);
        }

        fs::write(&md_path, text).context("failed to write md")?;
        Ok(())
    })?;
    Ok(())
}

pub fn should_process_notification(notification: &Value) -> bool {
    let typ = notification.get("type").and_then(|v| v.as_str()).unwrap_or("");
    if typ == "agent-turn-complete" {
        return true;
    }

    // Be tolerant of schema changes across Codex CLI versions.
    // Examples seen/expected: "agent-turn-complete", "assistant-turn-complete", "turn-complete".
    if typ.ends_with("turn-complete") || typ.ends_with("turn_complete") {
        return true;
    }

    false
}

fn notification_str<'a>(notification: &'a Value, keys: &[&str]) -> Option<&'a str> {
    for key in keys {
        if let Some(s) = notification.get(*key).and_then(|v| v.as_str()) {
            return Some(s);
        }
    }
    None
}

pub fn build_codex_note_skeleton(project: &str, thread_id: &str, cwd: &str) -> String {
    let created = Local::now().to_rfc3339_opts(SecondsFormat::Secs, true);

    let project_q = yaml_quote(project);
    let thread_q = yaml_quote(thread_id);
    let cwd_q = yaml_quote(cwd);

    format!(
        r#"---
tool: "Codex CLI"
project: "{project_q}"
thread_id: "{thread_q}"
cwd: "{cwd_q}"
created: "{created}"
tags:
  - ai-log
  - codex
  - {project_q}
---

"#
    )
}

pub fn ensure_turns_block(s: &str) -> String {
    if s.contains(BEGIN) && s.contains(END) {
        return s.to_string();
    }
    format!("{}\n\n{}\n## Turns (auto)\n{}\n", s.trim_end(), BEGIN, END)
}

pub fn build_turn_block(
    _turn_id: &str,
    input_messages: &Value,
    last_assistant: &str,
    sentinel: &str,
) -> String {
    let now = Local::now().format("%Y-%m-%d %H:%M:%S %z").to_string();

    let user_part = match input_messages {
        Value::Array(arr) => {
            let mut lines: Vec<String> = Vec::new();
            for v in arr {
                if let Some(s) = v.as_str() {
                    let s = s.trim();
                    if !s.is_empty() {
                        lines.push(format!("- {s}"));
                    }
                }
            }
            if lines.is_empty() {
                "- (empty)".to_string()
            } else {
                lines.join("\n")
            }
        }
        Value::String(s) => {
            let s = s.trim();
            if s.is_empty() {
                "(empty)".to_string()
            } else {
                s.to_string()
            }
        }
        _ => "- (empty)".to_string(),
    };

    format!(
        r#"{sentinel}

### {now} User
{user_part}

### {now} Assistant
{assistant}

"#,
        assistant = last_assistant.trim_end()
    )
}

pub fn insert_before_end(s: &str, block: &str) -> String {
    if let Some(pos) = s.find(END) {
        let (pre, post) = s.split_at(pos);
        format!(
            "{pre}\n\n{block}\n{post}",
            pre = pre.trim_end(),
            block = block.trim_end()
        )
    } else {
        format!("{}\n\n{}", s.trim_end(), block.trim_end())
    }
}

fn find_or_create_md_path(md_dir: &Path, thread_id: &str, first_user_msg: Option<&str>) -> Result<PathBuf> {
    if let Some(existing) = find_md_file_containing_id(md_dir, thread_id) {
        if let Some(migrated) = maybe_migrate_legacy_md_path(md_dir, &existing) {
            return Ok(migrated);
        }
        return Ok(existing);
    }

    let now = Local::now();
    let day_dir = md_dir
        .join(now.format("%Y").to_string())
        .join(now.format("%m").to_string())
        .join(now.format("%d").to_string());
    fs::create_dir_all(&day_dir).context("failed to create dated Threads dir")?;

    let title = generate_title(first_user_msg);
    let filename = format!("{title}_{thread_id}.md");
    Ok(day_dir.join(filename))
}

fn maybe_migrate_legacy_md_path(md_dir: &Path, existing: &Path) -> Option<PathBuf> {
    if existing.parent()? != md_dir {
        return None;
    }

    let name = existing.file_name()?.to_str()?;
    let (yyyy, mm, dd, rest) = split_legacy_dated_filename(name)?;

    let target_dir = md_dir.join(yyyy).join(mm).join(dd);
    let target_path = target_dir.join(rest);

    if target_path.exists() {
        return None;
    }

    fs::create_dir_all(&target_dir).ok()?;
    fs::rename(existing, &target_path).ok()?;
    Some(target_path)
}

fn split_legacy_dated_filename(name: &str) -> Option<(&str, &str, &str, &str)> {
    // legacy: YYYY-MM-DD_<title>_<id>.md
    if !name.ends_with(".md") || name.len() < 12 {
        return None;
    }

    let bytes = name.as_bytes();
    if bytes.get(4) != Some(&b'-') || bytes.get(7) != Some(&b'-') || bytes.get(10) != Some(&b'_') {
        return None;
    }
    if !bytes.get(0..4)?.iter().all(|b| b.is_ascii_digit())
        || !bytes.get(5..7)?.iter().all(|b| b.is_ascii_digit())
        || !bytes.get(8..10)?.iter().all(|b| b.is_ascii_digit())
    {
        return None;
    }

    let yyyy = name.get(0..4)?;
    let mm = name.get(5..7)?;
    let dd = name.get(8..10)?;
    let rest = name.get(11..)?;
    if rest.is_empty() {
        return None;
    }

    Some((yyyy, mm, dd, rest))
}

pub fn extract_first_user_msg(input_messages: &Value) -> Option<String> {
    match input_messages {
        Value::Array(arr) => arr.first().and_then(|v| v.as_str()).map(|s| s.to_string()),
        Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
