use anyhow::{Context, Result};
use chrono::{DateTime, Local, SecondsFormat};
use serde_json::Value;
use std::{
    env, fs,
    io::{self, BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::Command,
};

const BEGIN: &str = "<!-- BEGIN AUTO TRANSCRIPT -->";
const END: &str = "<!-- END AUTO TRANSCRIPT -->";

#[derive(Debug, Clone)]
struct Msg {
    role: &'static str, // "user" | "assistant"
    text: String,
    ts: Option<DateTime<Local>>,
}

fn main() -> Result<()> {
    // Hook payload arrives on stdin as JSON.
    let mut stdin = String::new();
    io::stdin()
        .read_to_string(&mut stdin)
        .context("failed to read stdin")?;

    let stdin = stdin.trim();
    if stdin.is_empty() {
        // Nothing to do.
        return Ok(());
    }

    let payload: Value =
        serde_json::from_str(stdin).context("failed to parse hook JSON from stdin")?;

    let session_id = payload
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown-session")
        .to_string();

    let transcript_path = payload
        .get("transcript_path")
        .and_then(|v| v.as_str())
        .context("missing transcript_path in hook payload")?;

    let cwd = payload.get("cwd").and_then(|v| v.as_str()).unwrap_or(".");

    let vault = env::var("OBSIDIAN_VAULT").context("Missing OBSIDIAN_VAULT env var")?;
    let ai_root = env::var("OBSIDIAN_AI_ROOT").context("Missing OBSIDIAN_AI_ROOT env var")?;

    let project = safe_name(&git_project_name(cwd));

    // Prepare Obsidian paths
    let vault_path = PathBuf::from(&vault);
    let base_dir = vault_path.join(&ai_root).join("Claude Code").join(&project);
    let md_dir = base_dir.join("Threads");
    let raw_dir = base_dir.join("_raw");
    fs::create_dir_all(&md_dir).context("failed to create md_dir")?;
    fs::create_dir_all(&raw_dir).context("failed to create raw_dir")?;

    // Copy raw transcript
    let raw_copy = raw_dir.join(format!("{session_id}.jsonl"));
    let _ = fs::copy(transcript_path, &raw_copy);

    // Parse transcript JSONL
    let msgs = parse_claude_jsonl(transcript_path).context("failed to parse transcript JSONL")?;
    let started_at = msgs.iter().find_map(|m| m.ts);

    // Markdown note path
    let md_path = md_dir.join(format!("{session_id}.md"));

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
}

fn build_claude_note_skeleton(
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

# Claude Code session — {project}

"#
    )
}

fn build_transcript_block(exported: &str, source: &str, msgs: &[Msg]) -> String {
    let mut out = String::new();
    out.push_str(BEGIN);
    out.push('\n');
    out.push_str("## Transcript (auto)\n");
    out.push_str(&format!("- Exported: {exported}\n"));
    out.push_str(&format!("- Source transcript: {source}\n\n"));

    for m in msgs {
        let ts =
            m.ts.map(|t| t.format("%Y-%m-%d %H:%M:%S %z").to_string())
                .unwrap_or_else(|| "".to_string());
        let who = if m.role == "user" { "User" } else { "Assistant" };
        out.push_str(&format!("### {ts} {who}\n"));
        out.push_str(m.text.trim_end());
        out.push_str("\n\n");
    }

    out.push_str(END);
    out.push('\n');
    out
}

fn upsert_block(existing: &str, new_block: &str) -> String {
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

fn extract_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Array(arr) => {
            // Join only {type:"text", text:"..."} blocks
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

    // Fallback: directory name of cwd
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
