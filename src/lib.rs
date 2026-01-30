use anyhow::{anyhow, Context, Result};
use std::{
    fs,
    fs::OpenOptions,
    io::{self, Write},
    path::Path,
    process::Command,
    thread,
    time::{Duration, Instant, SystemTime},
};

pub fn git_project_name(cwd: &str) -> String {
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

pub fn safe_id(raw: &str, fallback: &str) -> String {
    let raw = raw.trim();
    if raw.is_empty() {
        return fallback.to_string();
    }

    let mut base = safe_name(raw);
    if base.is_empty() {
        base = fallback.to_string();
    }

    // Preserve existing filenames when already safe.
    if base == raw {
        return base;
    }

    // Add a stable suffix to reduce collisions when sanitization changes the ID.
    let hash = fnv1a_64(raw);
    let suffix = format!("-{:08x}", (hash & 0xffff_ffff) as u32);
    let max_base_len = 120usize.saturating_sub(suffix.chars().count());
    if base.chars().count() > max_base_len {
        base = base.chars().take(max_base_len).collect();
    }

    format!("{base}{suffix}")
}

fn fnv1a_64(input: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in input.as_bytes() {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

pub fn yaml_quote(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

pub fn generate_title(text: Option<&str>) -> String {
    let text = match text {
        Some(t) if !t.trim().is_empty() => t,
        _ => return "untitled".to_string(),
    };

    if let Some(title) = generate_title_with_llm(text) {
        return title;
    }

    fallback_title(text)
}

pub fn generate_title_with_llm(text: &str) -> Option<String> {
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

pub fn with_lock_file<T, F>(lock_path: &Path, action: F) -> Result<T>
where
    F: FnOnce() -> Result<T>,
{
    const TIMEOUT: Duration = Duration::from_secs(10);
    const RETRY_DELAY: Duration = Duration::from_millis(50);
    const STALE_AFTER: Duration = Duration::from_secs(120);

    let started = Instant::now();

    loop {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(lock_path)
        {
            Ok(mut f) => {
                let _ = writeln!(f, "pid={}", std::process::id());
                let _ = f.flush();
                drop(f);

                let result = action();

                let _ = fs::remove_file(lock_path);
                return result;
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                if is_stale_lock(lock_path, STALE_AFTER) {
                    let _ = fs::remove_file(lock_path);
                    continue;
                }

                if started.elapsed() > TIMEOUT {
                    return Err(anyhow!(
                        "timeout waiting for lock file: {}",
                        lock_path.display()
                    ));
                }

                thread::sleep(RETRY_DELAY);
            }
            Err(e) => {
                return Err(e)
                    .with_context(|| format!("failed to create lock file: {}", lock_path.display()))
            }
        }
    }
}

fn is_stale_lock(lock_path: &Path, stale_after: Duration) -> bool {
    let meta = match fs::metadata(lock_path) {
        Ok(m) => m,
        Err(_) => return false,
    };

    let modified = match meta.modified() {
        Ok(m) => m,
        Err(_) => return false,
    };

    SystemTime::now()
        .duration_since(modified)
        .unwrap_or(Duration::ZERO)
        > stale_after
}

