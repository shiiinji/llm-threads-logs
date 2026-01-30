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

pub const BEGIN: &str = "<!-- BEGIN AUTO TURNS -->";
pub const END: &str = "<!-- END AUTO TURNS -->";

fn main() -> Result<()> {
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

    let vault_path = PathBuf::from(&vault);
    let base_dir = vault_path.join(&ai_root).join("Codex").join(&project);
    let md_dir = base_dir.join("Threads");
    let raw_dir = base_dir.join("_raw").join("notify");
    fs::create_dir_all(&md_dir).context("failed to create md_dir")?;
    fs::create_dir_all(&raw_dir).context("failed to create raw_dir")?;

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

    let first_user_msg = extract_first_user_msg(&input_messages);
    let md_path = find_or_create_md_path(&md_dir, thread_id, first_user_msg.as_deref());
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
        let block = build_turn_block("(no turn-id)", &input_messages, last_assistant, &sentinel);
        text = insert_before_end(&text, &block);
    }

    fs::write(&md_path, text).context("failed to write md")?;
    Ok(())
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

pub fn safe_name(s: &str) -> String {
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

pub fn yaml_quote(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn find_or_create_md_path(md_dir: &Path, thread_id: &str, first_user_msg: Option<&str>) -> PathBuf {
    if let Ok(entries) = fs::read_dir(md_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            if let Some(name_str) = name.to_str() {
                if name_str.contains(thread_id) && name_str.ends_with(".md") {
                    return entry.path();
                }
            }
        }
    }

    let date = Local::now().format("%Y-%m-%d").to_string();
    let title = generate_title(first_user_msg);
    let filename = format!("{date}_{title}_{thread_id}.md");
    md_dir.join(filename)
}

pub fn extract_first_user_msg(input_messages: &Value) -> Option<String> {
    match input_messages {
        Value::Array(arr) => arr.first().and_then(|v| v.as_str()).map(|s| s.to_string()),
        Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

fn generate_title(text: Option<&str>) -> String {
    let text = match text {
        Some(t) if !t.trim().is_empty() => t,
        _ => return "untitled".to_string(),
    };

    if let Some(title) = generate_title_with_llm(text) {
        return title;
    }

    fallback_title(text)
}

fn generate_title_with_llm(text: &str) -> Option<String> {
    let prompt = format!(
        "Generate a short filename-safe title (English, max 20 chars, lowercase, hyphens only, no spaces) for this conversation. Output ONLY the title, nothing else:\n\n{}",
        text.chars().take(500).collect::<String>()
    );

    let tmp_dir = std::env::temp_dir();
    let tmp_file = tmp_dir.join(format!("title_{}.txt", std::process::id()));

    let status = Command::new("codex")
        .args(["exec", "-c", "notify=[]", "-o", tmp_file.to_str()?, &prompt])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .ok()?;

    if !status.success() {
        let _ = fs::remove_file(&tmp_file);
        return None;
    }

    let title = fs::read_to_string(&tmp_file).ok()?;
    let _ = fs::remove_file(&tmp_file);
    let title = sanitize_title(&title);

    if title.is_empty() || title.len() > 50 {
        return None;
    }

    Some(title)
}

pub fn sanitize_title(s: &str) -> String {
    let title: String = s
        .trim()
        .chars()
        .take(30)
        .map(|c| match c {
            'a'..='z' | '0'..='9' | '-' => c,
            'A'..='Z' => c.to_ascii_lowercase(),
            ' ' | '_' => '-',
            _ => '-',
        })
        .collect();

    let mut result = String::new();
    let mut prev_hyphen = false;
    for c in title.chars() {
        if c == '-' {
            if !prev_hyphen && !result.is_empty() {
                result.push('-');
            }
            prev_hyphen = true;
        } else {
            result.push(c);
            prev_hyphen = false;
        }
    }

    result.trim_matches('-').to_string()
}

pub fn fallback_title(text: &str) -> String {
    sanitize_title(&text.chars().take(40).collect::<String>())
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
