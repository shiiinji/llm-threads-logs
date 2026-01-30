use ai_log_exporter::{generate_title, git_project_name, safe_id, safe_name, with_lock_file, yaml_quote};
use anyhow::{Context, Result};
use chrono::{DateTime, Local, SecondsFormat};
use serde_json::Value;
use std::{
    env, fs,
    io::{self, BufRead, BufReader, Read},
    path::{Path, PathBuf},
};

pub const BEGIN: &str = "<!-- BEGIN AUTO TRANSCRIPT -->";
pub const END: &str = "<!-- END AUTO TRANSCRIPT -->";

#[derive(Debug, Clone)]
pub struct Msg {
    pub role: &'static str,
    pub text: String,
    pub ts: Option<DateTime<Local>>,
}

fn main() -> Result<()> {
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
        .unwrap_or("unknown-session")
        .to_string();
    let session_id_safe = safe_id(&session_id, "unknown-session");

    let transcript_path = payload
        .get("transcript_path")
        .and_then(|v| v.as_str())
        .context("missing transcript_path in hook payload")?;

    let cwd = payload.get("cwd").and_then(|v| v.as_str()).unwrap_or(".");

    let vault = env::var("OBSIDIAN_VAULT").context("Missing OBSIDIAN_VAULT env var")?;
    let ai_root = env::var("OBSIDIAN_AI_ROOT").context("Missing OBSIDIAN_AI_ROOT env var")?;

    let project = safe_name(&git_project_name(cwd));

    let vault_path = PathBuf::from(&vault);
    let base_dir = vault_path.join(&ai_root).join("Claude Code").join(&project);
    let md_dir = base_dir.join("Threads");
    let raw_dir = base_dir.join("_raw");
    fs::create_dir_all(&md_dir).context("failed to create md_dir")?;
    fs::create_dir_all(&raw_dir).context("failed to create raw_dir")?;

    let lock_path = md_dir.join(format!(".lock_{session_id_safe}"));
    with_lock_file(&lock_path, || {
        let raw_copy = raw_dir.join(format!("{session_id_safe}.jsonl"));
        let _ = fs::copy(transcript_path, &raw_copy);

        let msgs = parse_claude_jsonl(transcript_path).context("failed to parse transcript JSONL")?;
        let started_at = msgs.iter().find_map(|m| m.ts);
        let first_user_msg = msgs.iter().find(|m| m.role == "user").map(|m| m.text.as_str());

        let md_path = find_or_create_md_path(&md_dir, &session_id_safe, first_user_msg, started_at);

        let existing = if md_path.exists() {
            fs::read_to_string(&md_path).context("failed to read existing md note")?
        } else {
            build_claude_note_skeleton(&project, &session_id, cwd, started_at)
        };

        let exported = Local::now().to_rfc3339_opts(SecondsFormat::Secs, true);
        let source_rel = raw_copy
            .strip_prefix(&vault_path)
            .ok()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| raw_copy.display().to_string());

        let new_block = build_transcript_block(&exported, &source_rel, &msgs);
        let updated = upsert_block(&existing, &new_block);

        fs::write(&md_path, updated).context("failed to write md note")?;
        Ok(())
    })?;
    Ok(())
}

pub fn build_claude_note_skeleton(
    project: &str,
    session_id: &str,
    cwd: &str,
    created: Option<DateTime<Local>>,
) -> String {
    let created = created.unwrap_or_else(Local::now);
    let created = created.to_rfc3339_opts(SecondsFormat::Secs, true);

    let project_q = yaml_quote(project);
    let session_q = yaml_quote(session_id);
    let cwd_q = yaml_quote(cwd);

    format!(
        r#"---
tool: "Claude Code"
project: "{project_q}"
session_id: "{session_q}"
cwd: "{cwd_q}"
created: "{created}"
tags:
  - ai-log
  - claude
  - {project_q}
---

"#
    )
}

pub fn build_transcript_block(exported: &str, source: &str, msgs: &[Msg]) -> String {
    let mut out = String::new();
    out.push_str(BEGIN);
    out.push('\n');
    out.push_str("## Transcript (auto)\n");
    out.push_str(&format!("- Exported: {exported}\n"));
    out.push_str(&format!("- Source transcript: {source}\n\n"));

    for m in msgs {
        let ts = m
            .ts
            .map(|t| t.format("%Y-%m-%d %H:%M:%S %z").to_string())
            .unwrap_or_default();
        let who = if m.role == "user" { "User" } else { "Assistant" };
        out.push_str(&format!("### {ts} {who}\n"));
        out.push_str(m.text.trim_end());
        out.push_str("\n\n");
    }

    out.push_str(END);
    out.push('\n');
    out
}

pub fn upsert_block(existing: &str, new_block: &str) -> String {
    let b = existing.find(BEGIN);
    let e = existing.find(END);

    match (b, e) {
        (Some(bi), Some(ei)) if ei >= bi => {
            let pre = &existing[..bi];
            let post = &existing[ei + END.len()..];
            format!("{pre}{new_block}{post}")
        }
        _ => {
            let mut s = existing.trim_end().to_string();
            s.push_str("\n\n");
            s.push_str(new_block);
            s
        }
    }
}

fn parse_claude_jsonl(path: &str) -> Result<Vec<Msg>> {
    let f = fs::File::open(path).with_context(|| format!("failed to open transcript: {path}"))?;
    let reader = BufReader::new(f);

    let mut msgs = Vec::new();

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let obj: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let typ = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let ts = obj
            .get("timestamp")
            .and_then(|v| v.as_str())
            .and_then(parse_rfc3339_local);

        if typ == "user" || typ == "assistant" {
            let role = if typ == "user" { "user" } else { "assistant" };

            let content = obj
                .get("message")
                .and_then(|m| m.get("content"))
                .unwrap_or(&Value::Null);

            if let Some(text) = extract_text(content) {
                let text = text.trim().to_string();
                if !text.is_empty() {
                    msgs.push(Msg { role, text, ts });
                }
            }
        }
    }

    Ok(msgs)
}

pub fn extract_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Array(arr) => {
            let mut parts: Vec<String> = Vec::new();
            for item in arr {
                if item.get("type").and_then(|x| x.as_str()) == Some("text") {
                    if let Some(t) = item.get("text").and_then(|x| x.as_str()) {
                        let t = t.trim();
                        if !t.is_empty() {
                            parts.push(t.to_string());
                        }
                    }
                }
            }
            if parts.is_empty() {
                None
            } else {
                Some(parts.join("\n"))
            }
        }
        _ => None,
    }
}

fn parse_rfc3339_local(s: &str) -> Option<DateTime<Local>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&Local))
}

fn find_or_create_md_path(
    md_dir: &Path,
    session_id: &str,
    first_user_msg: Option<&str>,
    started_at: Option<DateTime<Local>>,
) -> PathBuf {
    if let Ok(entries) = fs::read_dir(md_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            if let Some(name_str) = name.to_str() {
                if name_str.contains(session_id) && name_str.ends_with(".md") {
                    return entry.path();
                }
            }
        }
    }

    let date = started_at
        .unwrap_or_else(Local::now)
        .format("%Y-%m-%d")
        .to_string();
    let title = generate_title(first_user_msg);
    let filename = format!("{date}_{title}_{session_id}.md");
    md_dir.join(filename)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
