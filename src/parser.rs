//! Formula parsing helpers (string literals). AST lives in [`crate::ast`].

pub(crate) fn parse_standalone_string_literal(expression: &str) -> Option<String> {
    let expression = expression.trim().strip_prefix('=').unwrap_or(expression.trim());
    let mut chars = expression.chars().peekable();
    if chars.next() != Some('"') {
        return None;
    }

    let mut value = String::new();
    while let Some(ch) = chars.next() {
        if ch == '"' {
            if chars.peek() == Some(&'"') {
                chars.next();
                value.push('"');
            } else if chars.peek().is_none() {
                return Some(value);
            } else {
                return None;
            }
        } else {
            value.push(ch);
        }
    }
    None
}
