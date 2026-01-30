use anyhow::{Context, Result};
use chrono::{Local, SecondsFormat};
use serde_json::Value;
use std::{
    env, fs,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

const BEGIN: &str = "<!-- BEGIN AUTO TURNS -->";
const END: &str = "<!-- END AUTO TURNS -->";

fn main() -> Result<()> {
    // Codex passes a single JSON notification payload in argv[1].
    let payload = env::args().nth(1);
    let payload = match payload {
        Some(p) if !p.trim().is_empty() => p,
        _ => return Ok(()),
    };

    let notification: Value =
        serde_json::from_str(&payload).context("failed to parse notify JSON from argv[1]")?;

    if notification.get("type").and_then(|v| v.as_str()) != Some("agent-turn-complete") {
        return Ok(());
    }

    let thread_id = notification
        .get("thread-id")
        .or_else(|| notification.get("thread_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown-thread");

    let turn_id = notification
        .get("turn-id")
        .or_else(|| notification.get("turn_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let cwd = notification
        .get("cwd")
        .and_then(|v| v.as_str())
        .unwrap_or(".");

    let input_messages = notification
        .get("input-messages")
        .or_else(|| notification.get("input_messages"))
        .cloned()
        .unwrap_or(Value::Null);

    let last_assistant = notification
        .get("last-assistant-message")
        .or_else(|| notification.get("last_assistant_message"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let vault = env::var("OBSIDIAN_VAULT").context("Missing OBSIDIAN_VAULT env var")?;
    let ai_root = env::var("OBSIDIAN_AI_ROOT").context("Missing OBSIDIAN_AI_ROOT env var")?;

    let project = safe_name(&git_project_name(cwd));

    // Paths
    let vault_path = PathBuf::from(&vault);
    let base_dir = vault_path.join(&ai_root).join("Codex").join(&project);
    let md_dir = base_dir.join("Threads");
    let raw_dir = base_dir.join("_raw").join("notify");
    fs::create_dir_all(&md_dir).context("failed to create md_dir")?;
    fs::create_dir_all(&raw_dir).context("failed to create raw_dir")?;

    // Raw notify log (jsonl)
    let raw_path = raw_dir.join(format!("{thread_id}.jsonl"));
    {
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&raw_path)
            .context("failed to open raw notify jsonl")?;
        writeln!(
            f,
            "{}",
            serde_json::to_string(&notification).unwrap_or(payload)
        )?;
    }

    // Markdown note
    let md_path = md_dir.join(format!("{thread_id}.md"));
    let mut text = if md_path.exists() {
        fs::read_to_string(&md_path).context("failed to read existing md")?
    } else {
        build_codex_note_skeleton(&project, thread_id, cwd)
    };

    text = ensure_turns_block(&text);

    // Dedupe by turn-id (avoid double writes)
    if !turn_id.is_empty() {
        let sentinel = format!("<!-- turn-id:{turn_id} -->");
        if text.contains(&sentinel) {
            return Ok(());
        }
        let block = build_turn_block(turn_id, &input_messages, last_assistant, &sentinel);
        text = insert_before_end(&text, &block);
    } else {
        // No turn-id: still append, but without strict dedupe
        let sentinel = "<!-- turn-id:(missing) -->".to_string();
        let block = build_turn_block("(no turn-id)", &input_messages, last_assistant, &sentinel);
        text = insert_before_end(&text, &block);
    }

    fs::write(&md_path, text).context("failed to write md")?;
    Ok(())
}

fn build_codex_note_skeleton(project: &str, thread_id: &str, cwd: &str) -> String {
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

# Codex thread — {project}

"#
    )
}

fn ensure_turns_block(s: &str) -> String {
    if s.contains(BEGIN) && s.contains(END) {
        return s.to_string();
    }
    format!("{}\n\n{}\n## Turns (auto)\n{}\n", s.trim_end(), BEGIN, END)
}

fn build_turn_block(
    turn_id: &str,
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

fn insert_before_end(s: &str, block: &str) -> String {
    if let Some(pos) = s.find(END) {
        let (pre, post) = s.split_at(pos);
        format!(
            "{}{}\n{}",
            pre.trim_end(),
            format!("\n\n{}", block.trim_end()),
            post
        )
    } else {
        format!("{}\n\n{}", s.trim_end(), block.trim_end())
    }
}

fn git_project_name(cwd: &str) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .arg("rev-parse")
        .arg("--show-toplevel")
        .output();

    if let Ok(out) = out {
        if out.status.success() {
            if let Ok(s) = String::from_utf8(out.stdout) {
                let p = Path::new(s.trim());
                if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                    if !name.trim().is_empty() {
                        return name.to_string();
                    }
                }
            }
        }
    }

    Path::new(cwd)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown-project")
        .to_string()
}

fn safe_name(s: &str) -> String {
    let mut tmp = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '/' | '\\' | ':' | '\n' | '\r' | '\t' => tmp.push('_'),
            _ => tmp.push(c),
        }
    }
    let collapsed = tmp.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() > 120 {
        collapsed.chars().take(120).collect()
    } else {
        collapsed
    }
}

fn yaml_quote(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}
