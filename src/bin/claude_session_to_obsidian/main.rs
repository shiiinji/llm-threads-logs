use ai_log_exporter::{
    find_md_file_containing_id, generate_title, git_project_name, safe_id, safe_name, with_lock_file,
    yaml_quote,
};
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
    fs::create_dir_all(&md_dir).context("failed to create md_dir")?;

    let lock_path = md_dir.join(format!(".lock_{session_id_safe}"));
    with_lock_file(&lock_path, || {
        let msgs = parse_claude_jsonl(transcript_path).context("failed to parse transcript JSONL")?;
        let started_at = msgs.iter().find_map(|m| m.ts);
        let first_user_msg = msgs.iter().find(|m| m.role == "user").map(|m| m.text.as_str());

        let md_path =
            find_or_create_md_path(&md_dir, &session_id_safe, first_user_msg, started_at)
                .context("failed to find or create md path")?;

        let existing = if md_path.exists() {
            fs::read_to_string(&md_path).context("failed to read existing md note")?
        } else {
            build_claude_note_skeleton(&project, &session_id, cwd, started_at)
        };

        let exported = Local::now().to_rfc3339_opts(SecondsFormat::Secs, true);
        let source_rel = transcript_path.to_string();

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
) -> Result<PathBuf> {
    if let Some(existing) = find_md_file_containing_id(md_dir, session_id) {
        if let Some(migrated) = maybe_migrate_legacy_md_path(md_dir, &existing) {
            return Ok(migrated);
        }
        return Ok(existing);
    }

    let started_at = started_at.unwrap_or_else(Local::now);
    let day_dir = md_dir
        .join(started_at.format("%Y").to_string())
        .join(started_at.format("%m").to_string())
        .join(started_at.format("%d").to_string());
    fs::create_dir_all(&day_dir).context("failed to create dated Threads dir")?;

    let title = generate_title(first_user_msg);
    let filename = format!("{title}_{session_id}.md");
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

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
