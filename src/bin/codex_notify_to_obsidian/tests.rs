use super::*;

// ========================================
// safe_name tests
// ========================================

#[test]
fn test_safe_name_replaces_special_chars() {
    assert_eq!(safe_name("foo/bar"), "foo_bar");
    assert_eq!(safe_name("foo\\bar"), "foo_bar");
    assert_eq!(safe_name("foo:bar"), "foo_bar");
}

#[test]
fn test_safe_name_collapses_whitespace() {
    assert_eq!(safe_name("foo   bar"), "foo bar");
}

#[test]
fn test_safe_name_truncates_long_strings() {
    let long_str = "a".repeat(150);
    let result = safe_name(&long_str);
    assert_eq!(result.chars().count(), 120);
}

// ========================================
// yaml_quote tests
// ========================================

#[test]
fn test_yaml_quote_escapes_special_chars() {
    assert_eq!(yaml_quote("foo\\bar"), "foo\\\\bar");
    assert_eq!(yaml_quote(r#"foo"bar"#), r#"foo\"bar"#);
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
fn test_sanitize_title_collapses_consecutive_hyphens() {
    assert_eq!(sanitize_title("hello---world"), "hello-world");
}

#[test]
fn test_sanitize_title_trims_hyphens() {
    assert_eq!(sanitize_title("---hello---"), "hello");
}

// ========================================
// extract_first_user_msg tests
// ========================================

#[test]
fn test_extract_first_user_msg_from_array() {
    let v = serde_json::json!(["first message", "second message"]);
    assert_eq!(
        extract_first_user_msg(&v),
        Some("first message".to_string())
    );
}

#[test]
fn test_extract_first_user_msg_from_string() {
    let v = serde_json::json!("single message");
    assert_eq!(
        extract_first_user_msg(&v),
        Some("single message".to_string())
    );
}

#[test]
fn test_extract_first_user_msg_from_empty_array() {
    let v = serde_json::json!([]);
    assert_eq!(extract_first_user_msg(&v), None);
}

#[test]
fn test_extract_first_user_msg_from_null() {
    let v = serde_json::Value::Null;
    assert_eq!(extract_first_user_msg(&v), None);
}

// ========================================
// ensure_turns_block tests
// ========================================

#[test]
fn test_ensure_turns_block_adds_markers_when_missing() {
    let input = "# Title\n\nSome content";
    let result = ensure_turns_block(input);

    assert!(result.contains(BEGIN));
    assert!(result.contains(END));
    assert!(result.contains("## Turns (auto)"));
}

#[test]
fn test_ensure_turns_block_preserves_existing() {
    let input = format!("# Title\n\n{}\nexisting\n{}", BEGIN, END);
    let result = ensure_turns_block(&input);

    assert_eq!(result, input);
}

// ========================================
// insert_before_end tests
// ========================================

#[test]
fn test_insert_before_end_inserts_correctly() {
    let input = format!("# Title\n\n{}\n{}", BEGIN, END);
    let block = "new content";
    let result = insert_before_end(&input, block);

    assert!(result.contains("new content"));
    let end_pos = result.find(END).unwrap();
    let content_pos = result.find("new content").unwrap();
    assert!(content_pos < end_pos);
}

#[test]
fn test_insert_before_end_appends_when_no_end_marker() {
    let input = "# Title\n\nSome content";
    let block = "new content";
    let result = insert_before_end(input, block);

    assert!(result.contains("new content"));
    assert!(result.contains("# Title"));
}

// ========================================
// build_turn_block tests
// ========================================

#[test]
fn test_build_turn_block_with_array_input() {
    let input = serde_json::json!(["user message 1", "user message 2"]);
    let sentinel = "<!-- turn-id:test123 -->";
    let result = build_turn_block("test123", &input, "assistant response", sentinel);

    assert!(result.contains(sentinel));
    assert!(result.contains("- user message 1"));
    assert!(result.contains("- user message 2"));
    assert!(result.contains("assistant response"));
    assert!(result.contains("User"));
    assert!(result.contains("Assistant"));
}

#[test]
fn test_build_turn_block_with_string_input() {
    let input = serde_json::json!("single user message");
    let sentinel = "<!-- turn-id:test456 -->";
    let result = build_turn_block("test456", &input, "response", sentinel);

    assert!(result.contains("single user message"));
    assert!(result.contains("response"));
}

#[test]
fn test_build_turn_block_with_empty_array() {
    let input = serde_json::json!([]);
    let sentinel = "<!-- turn-id:test -->";
    let result = build_turn_block("test", &input, "response", sentinel);

    assert!(result.contains("- (empty)"));
}

// ========================================
// build_codex_note_skeleton tests
// ========================================

#[test]
fn test_build_codex_note_skeleton_contains_required_fields() {
    let result = build_codex_note_skeleton("my-project", "thread-123", "/path/to/cwd");

    assert!(result.contains("tool: \"Codex CLI\""));
    assert!(result.contains("project: \"my-project\""));
    assert!(result.contains("thread_id: \"thread-123\""));
    assert!(result.contains("cwd: \"/path/to/cwd\""));
    assert!(result.contains("tags:"));
    assert!(result.contains("- ai-log"));
    assert!(result.contains("- codex"));
}
