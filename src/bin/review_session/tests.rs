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
// extract_user_messages tests
// ========================================

#[test]
fn test_extract_user_messages_basic() {
    let md = r#"
# Title

### 2024-01-01 10:00:00 User
Hello, this is my question.

### 2024-01-01 10:01:00 Assistant
Here is my response.

### 2024-01-01 10:02:00 User
Follow up question.

### 2024-01-01 10:03:00 Assistant
Another response.
"#;
    let messages = extract_user_messages(md);
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0], "Hello, this is my question.");
    assert_eq!(messages[1], "Follow up question.");
}

#[test]
fn test_extract_user_messages_multiline() {
    let md = r#"
### 2024-01-01 10:00:00 User
First line.
Second line.
Third line.

### 2024-01-01 10:01:00 Assistant
Response.
"#;
    let messages = extract_user_messages(md);
    assert_eq!(messages.len(), 1);
    assert!(messages[0].contains("First line."));
    assert!(messages[0].contains("Second line."));
    assert!(messages[0].contains("Third line."));
}

#[test]
fn test_extract_user_messages_empty() {
    let md = r#"
# Just a title

Some content without user messages.
"#;
    let messages = extract_user_messages(md);
    assert!(messages.is_empty());
}

#[test]
fn test_extract_user_messages_only_assistant() {
    let md = r#"
### 2024-01-01 10:00:00 Assistant
Just an assistant message.
"#;
    let messages = extract_user_messages(md);
    assert!(messages.is_empty());
}

#[test]
fn test_extract_user_messages_user_at_end() {
    let md = r#"
### 2024-01-01 10:00:00 User
Last user message without assistant response.
"#;
    let messages = extract_user_messages(md);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0], "Last user message without assistant response.");
}

#[test]
fn test_extract_user_messages_skips_empty_blocks() {
    let md = r#"
### 2024-01-01 10:00:00 User


### 2024-01-01 10:01:00 Assistant
Response.

### 2024-01-01 10:02:00 User
Real message.

### 2024-01-01 10:03:00 Assistant
Response.
"#;
    let messages = extract_user_messages(md);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0], "Real message.");
}

#[test]
fn test_extract_user_messages_consecutive_users() {
    let md = r#"
### 2024-01-01 10:00:00 User
First user message.

### 2024-01-01 10:01:00 User
Second user message.

### 2024-01-01 10:02:00 Assistant
Response.
"#;
    let messages = extract_user_messages(md);
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0], "First user message.");
    assert_eq!(messages[1], "Second user message.");
}
