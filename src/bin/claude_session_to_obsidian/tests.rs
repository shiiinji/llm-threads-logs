use ai_log_exporter::{fallback_title, sanitize_title, safe_name, yaml_quote};
use super::*;

// ========================================
// safe_name tests
// ========================================

#[test]
fn test_safe_name_replaces_special_chars() {
    assert_eq!(safe_name("foo/bar"), "foo_bar");
    assert_eq!(safe_name("foo\\bar"), "foo_bar");
    assert_eq!(safe_name("foo:bar"), "foo_bar");
    assert_eq!(safe_name("foo\nbar"), "foo_bar");
    assert_eq!(safe_name("foo\rbar"), "foo_bar");
    assert_eq!(safe_name("foo\tbar"), "foo_bar");
}

#[test]
fn test_safe_name_collapses_whitespace() {
    assert_eq!(safe_name("foo   bar"), "foo bar");
    assert_eq!(safe_name("  foo  bar  "), "foo bar");
}

#[test]
fn test_safe_name_truncates_long_strings() {
    let long_str = "a".repeat(150);
    let result = safe_name(&long_str);
    assert_eq!(result.chars().count(), 120);
}

#[test]
fn test_safe_name_preserves_normal_chars() {
    assert_eq!(safe_name("hello-world_123"), "hello-world_123");
}

// ========================================
// yaml_quote tests
// ========================================

#[test]
fn test_yaml_quote_escapes_backslash() {
    assert_eq!(yaml_quote("foo\\bar"), "foo\\\\bar");
}

#[test]
fn test_yaml_quote_escapes_quotes() {
    assert_eq!(yaml_quote(r#"foo"bar"#), r#"foo\"bar"#);
}

#[test]
fn test_yaml_quote_escapes_both() {
    assert_eq!(yaml_quote(r#"a\b"c"#), r#"a\\b\"c"#);
}

#[test]
fn test_yaml_quote_no_change_for_normal_string() {
    assert_eq!(yaml_quote("hello world"), "hello world");
}

// ========================================
// sanitize_title tests
// ========================================

#[test]
fn test_sanitize_title_lowercase() {
    assert_eq!(sanitize_title("HelloWorld"), "helloworld");
}

#[test]
fn test_sanitize_title_replaces_spaces_with_hyphens() {
    assert_eq!(sanitize_title("hello world"), "hello-world");
}

#[test]
fn test_sanitize_title_replaces_underscores() {
    assert_eq!(sanitize_title("hello_world"), "hello-world");
}

#[test]
fn test_sanitize_title_removes_special_chars() {
    assert_eq!(sanitize_title("hello!@#world"), "hello-world");
}

#[test]
fn test_sanitize_title_collapses_consecutive_hyphens() {
    assert_eq!(sanitize_title("hello---world"), "hello-world");
    assert_eq!(sanitize_title("a!!b##c"), "a-b-c");
}

#[test]
fn test_sanitize_title_trims_hyphens() {
    assert_eq!(sanitize_title("---hello---"), "hello");
    assert_eq!(sanitize_title("!hello!"), "hello");
}

#[test]
fn test_sanitize_title_truncates_to_30_chars() {
    let long_title = "a".repeat(50);
    let result = sanitize_title(&long_title);
    assert!(result.chars().count() <= 30);
}

#[test]
fn test_sanitize_title_preserves_numbers() {
    assert_eq!(sanitize_title("test123"), "test123");
}

// ========================================
// fallback_title tests
// ========================================

#[test]
fn test_fallback_title_basic() {
    assert_eq!(fallback_title("Hello World"), "hello-world");
}

#[test]
fn test_fallback_title_truncates() {
    let long_text = "a".repeat(100);
    let result = fallback_title(&long_text);
    assert!(result.chars().count() <= 30);
}

// ========================================
// extract_text tests
// ========================================

#[test]
fn test_extract_text_from_string() {
    let v = serde_json::Value::String("hello".to_string());
    assert_eq!(extract_text(&v), Some("hello".to_string()));
}

#[test]
fn test_extract_text_from_array_with_text_blocks() {
    let v = serde_json::json!([
        {"type": "text", "text": "hello"},
        {"type": "text", "text": "world"}
    ]);
    assert_eq!(extract_text(&v), Some("hello\nworld".to_string()));
}

#[test]
fn test_extract_text_skips_non_text_blocks() {
    let v = serde_json::json!([
        {"type": "text", "text": "hello"},
        {"type": "tool_use", "name": "bash"},
        {"type": "text", "text": "world"}
    ]);
    assert_eq!(extract_text(&v), Some("hello\nworld".to_string()));
}

#[test]
fn test_extract_text_returns_none_for_empty_array() {
    let v = serde_json::json!([]);
    assert_eq!(extract_text(&v), None);
}

#[test]
fn test_extract_text_returns_none_for_null() {
    let v = serde_json::Value::Null;
    assert_eq!(extract_text(&v), None);
}

#[test]
fn test_extract_text_skips_empty_text() {
    let v = serde_json::json!([
        {"type": "text", "text": ""},
        {"type": "text", "text": "hello"}
    ]);
    assert_eq!(extract_text(&v), Some("hello".to_string()));
}

// ========================================
// upsert_block tests
// ========================================

#[test]
fn test_upsert_block_replaces_existing() {
    let existing = format!("# Title\n\n{}\nold content\n{}\n\n# Footer", BEGIN, END);
    let new_block = format!("{}\nnew content\n{}\n", BEGIN, END);
    let result = upsert_block(&existing, &new_block);

    assert!(result.contains("new content"));
    assert!(!result.contains("old content"));
    assert!(result.contains("# Title"));
    assert!(result.contains("# Footer"));
}

#[test]
fn test_upsert_block_appends_when_no_markers() {
    let existing = "# Title\n\nSome content";
    let new_block = format!("{}\nnew content\n{}\n", BEGIN, END);
    let result = upsert_block(existing, &new_block);

    assert!(result.contains("# Title"));
    assert!(result.contains("new content"));
}

// ========================================
// build_claude_note_skeleton tests
// ========================================

#[test]
fn test_build_claude_note_skeleton_contains_required_fields() {
    let result = build_claude_note_skeleton("my-project", "session-123", "/path/to/cwd", None);

    assert!(result.contains("tool: \"Claude Code\""));
    assert!(result.contains("project: \"my-project\""));
    assert!(result.contains("session_id: \"session-123\""));
    assert!(result.contains("cwd: \"/path/to/cwd\""));
    assert!(result.contains("tags:"));
    assert!(result.contains("- ai-log"));
    assert!(result.contains("- claude"));
}

#[test]
fn test_build_claude_note_skeleton_escapes_special_chars() {
    let result = build_claude_note_skeleton("project\"with\"quotes", "session", "/cwd", None);
    assert!(result.contains(r#"project: "project\"with\"quotes""#));
}

// ========================================
// build_transcript_block tests
// ========================================

#[test]
fn test_build_transcript_block_structure() {
    let msgs = vec![
        Msg {
            role: "user",
            text: "Hello".to_string(),
            ts: None,
        },
        Msg {
            role: "assistant",
            text: "Hi there".to_string(),
            ts: None,
        },
    ];

    let result = build_transcript_block("2024-01-01", "source.jsonl", &msgs);

    assert!(result.starts_with(BEGIN));
    assert!(result.ends_with(&format!("{}\n", END)));
    assert!(result.contains("## Transcript (auto)"));
    assert!(result.contains("- Exported: 2024-01-01"));
    assert!(result.contains("- Source transcript: source.jsonl"));
    assert!(result.contains("User"));
    assert!(result.contains("Assistant"));
    assert!(result.contains("Hello"));
    assert!(result.contains("Hi there"));
}
