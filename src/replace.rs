//! Placeholder replacement (`__[A-Z0-9]+__`) applied before formula evaluation.

use std::collections::HashMap;

/// Value substituted for a placeholder key.
///
/// Construct only via [`ReplacementValue::from_i64`], [`from_f64`](Self::from_f64),
/// or [`from_text`](Self::from_text) so numeric inserts stay canonical literals
/// (not arbitrary formula fragments).
#[derive(Debug, Clone, PartialEq)]
pub struct ReplacementValue {
    kind: ReplacementKind,
}

#[derive(Debug, Clone, PartialEq)]
enum ReplacementKind {
    /// Canonical numeric literal text (from `from_i64` / `from_f64` only).
    Number(String),
    /// Text (quoted when placed inside a formula).
    Text(String),
}

impl ReplacementValue {
    pub fn from_i64(n: i64) -> Self {
        Self {
            kind: ReplacementKind::Number(n.to_string()),
        }
    }

    pub fn from_f64(n: f64) -> Self {
        Self {
            kind: ReplacementKind::Number(format_number(n)),
        }
    }

    pub fn from_text(s: impl Into<String>) -> Self {
        Self {
            kind: ReplacementKind::Text(s.into()),
        }
    }
}

/// Canonical text for a floating cell / replacement number (whole values without `.0`).
///
/// Used by the PHP extension when stringifying numeric cell inputs.
pub fn format_number(n: f64) -> String {
    if n.is_finite() && n.fract() == 0.0 && n.abs() < (1i64 << 53) as f64 {
        format!("{}", n as i64)
    } else {
        n.to_string()
    }
}

/// `true` when `key` matches `__[A-Z0-9]+__`.
pub(crate) fn is_valid_placeholder_key(key: &str) -> bool {
    let bytes = key.as_bytes();
    if bytes.len() < 5 || !bytes.starts_with(b"__") || !bytes.ends_with(b"__") {
        return false;
    }
    let inner = &bytes[2..bytes.len() - 2];
    !inner.is_empty()
        && inner
            .iter()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

/// Keys that are not `__[A-Z0-9]+__` (sorted). Does not copy values.
pub(crate) fn ignored_replacement_keys(
    replacements: &HashMap<String, ReplacementValue>,
) -> Vec<String> {
    let mut ignored_keys: Vec<String> = replacements
        .keys()
        .filter(|key| !is_valid_placeholder_key(key))
        .cloned()
        .collect();
    ignored_keys.sort();
    ignored_keys
}

/// Replace `__[A-Z0-9]+__` tokens, then quote bare text so the engine does not
/// treat it as a cell name. Original formula cells (`=` prefix) are not wrapped.
pub(crate) fn prepare_cell(text: &str, replacements: &HashMap<String, ReplacementValue>) -> String {
    let is_formula = text.trim_start().starts_with('=');
    let replaced = apply_replacements(text, replacements, is_formula);
    if is_formula {
        replaced
    } else {
        quote_if_bare_text(&replaced)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PreparedCells {
    pub cells: Vec<(String, String)>,
    pub ignored_replacement_keys: Vec<String>,
}

/// Apply [`prepare_cell`] to every cell; invalid keys in `replacements` are ignored.
///
/// Uses `replacements` by reference (no map clone). Invalid keys never match
/// `__[A-Z0-9]+__` tokens, so leaving them in the map is harmless.
pub(crate) fn prepare_cells(
    cells: &[(&str, &str)],
    replacements: &HashMap<String, ReplacementValue>,
) -> PreparedCells {
    let ignored_keys = ignored_replacement_keys(replacements);
    let cells = cells
        .iter()
        .map(|(name, expr)| ((*name).to_string(), prepare_cell(expr, replacements)))
        .collect();
    PreparedCells {
        cells,
        ignored_replacement_keys: ignored_keys,
    }
}

fn apply_replacements(
    text: &str,
    replacements: &HashMap<String, ReplacementValue>,
    is_formula: bool,
) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let mut in_string = false;
    while i < chars.len() {
        let ch = chars[i];
        if in_string {
            out.push(ch);
            if ch == '"' {
                if i + 1 < chars.len() && chars[i + 1] == '"' {
                    // Excel-escaped quote inside a string literal.
                    out.push('"');
                    i += 2;
                    continue;
                }
                in_string = false;
            }
            i += 1;
            continue;
        }
        if ch == '"' {
            in_string = true;
            out.push('"');
            i += 1;
            continue;
        }
        if ch == '_' {
            if let Some(end) = find_placeholder_end(&chars, i) {
                let key: String = chars[i..=end].iter().collect();
                if let Some(value) = replacements.get(&key) {
                    out.push_str(&format_insert(value, is_formula));
                    i = end + 1;
                    continue;
                }
            }
        }
        out.push(ch);
        i += 1;
    }
    out
}

fn find_placeholder_end(chars: &[char], start: usize) -> Option<usize> {
    if start + 4 >= chars.len() {
        return None;
    }
    if chars[start] != '_' || chars[start + 1] != '_' {
        return None;
    }
    let mut j = start + 2;
    if j >= chars.len() || !is_placeholder_inner(chars[j]) {
        return None;
    }
    j += 1;
    while j < chars.len() && is_placeholder_inner(chars[j]) {
        j += 1;
    }
    if j + 1 < chars.len() && chars[j] == '_' && chars[j + 1] == '_' {
        Some(j + 1)
    } else {
        None
    }
}

fn is_placeholder_inner(ch: char) -> bool {
    ch.is_ascii_uppercase() || ch.is_ascii_digit()
}

fn format_insert(value: &ReplacementValue, is_formula: bool) -> String {
    match &value.kind {
        ReplacementKind::Number(n) => n.clone(),
        ReplacementKind::Text(s) if is_formula => excel_quote(s),
        ReplacementKind::Text(s) => s.clone(),
    }
}

fn quote_if_bare_text(text: &str) -> String {
    if is_numeric_literal(text) || is_excel_string_literal(text) {
        text.to_string()
    } else {
        excel_quote(text.trim())
    }
}

fn is_numeric_literal(text: &str) -> bool {
    let text = text.trim();
    if text.is_empty() {
        return false;
    }
    let mut chars = text.chars().peekable();
    if matches!(chars.peek(), Some('+' | '-')) {
        chars.next();
    }
    let rest: String = chars.collect();
    if rest.is_empty() {
        return false;
    }
    let mut seen_digit = false;
    let mut seen_dot = false;
    for ch in rest.chars() {
        if ch.is_ascii_digit() {
            seen_digit = true;
        } else if ch == '.' && !seen_dot {
            seen_dot = true;
        } else {
            return false;
        }
    }
    seen_digit
}

fn is_excel_string_literal(text: &str) -> bool {
    let text = text.trim().strip_prefix('=').unwrap_or(text.trim());
    let mut chars = text.chars().peekable();
    if chars.next() != Some('"') {
        return false;
    }
    while let Some(ch) = chars.next() {
        if ch == '"' {
            if chars.peek() == Some(&'"') {
                chars.next();
            } else {
                return chars.peek().is_none();
            }
        }
    }
    false
}

fn excel_quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        if ch == '"' {
            out.push_str("\"\"");
        } else {
            out.push(ch);
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, ReplacementValue)]) -> HashMap<String, ReplacementValue> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn replaces_dunder_uppercase_key() {
        let replacements = map(&[("__NAME__", ReplacementValue::from_text("Alice"))]);
        assert_eq!(prepare_cell("__NAME__", &replacements), "\"Alice\"");
    }

    #[test]
    fn does_not_replace_plain_text() {
        let replacements = map(&[("__NAME__", ReplacementValue::from_text("Alice"))]);
        assert_eq!(prepare_cell("NAME", &replacements), "\"NAME\"");
    }

    #[test]
    fn skips_lowercase_placeholder() {
        let replacements = map(&[("__NAME__", ReplacementValue::from_text("Alice"))]);
        assert_eq!(prepare_cell("__name__", &replacements), "\"__name__\"");
    }

    #[test]
    fn skips_single_underscore_wrap() {
        let replacements = map(&[("__NAME__", ReplacementValue::from_text("Alice"))]);
        assert_eq!(prepare_cell("_NAME_", &replacements), "\"_NAME_\"");
    }

    #[test]
    fn skips_unknown_key() {
        let replacements = map(&[("__NAME__", ReplacementValue::from_text("Alice"))]);
        assert_eq!(
            prepare_cell("__UNKNOWN__", &replacements),
            "\"__UNKNOWN__\""
        );
    }

    #[test]
    fn unknown_in_formula_is_left_as_identifier() {
        let replacements = HashMap::new();
        assert_eq!(
            prepare_cell("=__MISS__*2", &replacements),
            "=__MISS__*2"
        );
    }

    #[test]
    fn does_not_partial_match_namespace() {
        let replacements = map(&[("__NAME__", ReplacementValue::from_text("Alice"))]);
        assert_eq!(
            prepare_cell("__NAMESPACE__", &replacements),
            "\"__NAMESPACE__\""
        );
    }

    #[test]
    fn ignores_invalid_map_keys() {
        let mut raw = HashMap::new();
        raw.insert("NAME".into(), ReplacementValue::from_text("Alice"));
        raw.insert("__name__".into(), ReplacementValue::from_text("nope"));
        raw.insert("_NAME_".into(), ReplacementValue::from_text("x"));
        raw.insert("[NAME]".into(), ReplacementValue::from_text("old"));
        raw.insert("__USER_NAME__".into(), ReplacementValue::from_text("x"));
        raw.insert("__OK__".into(), ReplacementValue::from_i64(1));
        assert_eq!(
            ignored_replacement_keys(&raw),
            vec![
                "NAME".to_string(),
                "[NAME]".to_string(),
                "_NAME_".to_string(),
                "__USER_NAME__".to_string(),
                "__name__".to_string(),
            ]
        );
        assert_eq!(prepare_cell("__OK__", &raw), "1");
        assert_eq!(prepare_cell("__name__", &raw), "\"__name__\"");
    }

    #[test]
    fn inserts_number_in_formula() {
        let replacements = map(&[("__RATE__", ReplacementValue::from_i64(10))]);
        assert_eq!(prepare_cell("=__RATE__*2", &replacements), "=10*2");
    }

    #[test]
    fn inserts_quoted_string_in_formula() {
        let replacements = map(&[("__NAME__", ReplacementValue::from_text("Alice"))]);
        assert_eq!(
            prepare_cell("=\"Hi \"&__NAME__", &replacements),
            "=\"Hi \"&\"Alice\""
        );
    }

    #[test]
    fn numeric_whole_cell_stays_unquoted() {
        let replacements = map(&[("__RATE__", ReplacementValue::from_i64(10))]);
        assert_eq!(prepare_cell("__RATE__", &replacements), "10");
    }

    #[test]
    fn formula_like_replacement_is_text() {
        let replacements = map(&[("__NAME__", ReplacementValue::from_text("=SUM(1)"))]);
        assert_eq!(prepare_cell("__NAME__", &replacements), "\"=SUM(1)\"");
    }

    #[test]
    fn replacement_value_placeholders_are_not_rescanned() {
        let replacements = map(&[
            ("__NAME__", ReplacementValue::from_text("__RATE__")),
            ("__RATE__", ReplacementValue::from_i64(10)),
        ]);
        assert_eq!(prepare_cell("__NAME__", &replacements), "\"__RATE__\"");
    }

    #[test]
    fn a1_cell_ref_is_not_a_placeholder() {
        let replacements = map(&[("__A1__", ReplacementValue::from_i64(9))]);
        assert_eq!(prepare_cell("=A1+1", &replacements), "=A1+1");
        assert_eq!(prepare_cell("=__A1__+1", &replacements), "=9+1");
    }

    #[test]
    fn already_quoted_text_is_kept() {
        let replacements = HashMap::new();
        assert_eq!(prepare_cell("\"keep\"", &replacements), "\"keep\"");
    }

    #[test]
    fn escapes_quotes_in_text() {
        let replacements = map(&[("__Q__", ReplacementValue::from_text("a\"b"))]);
        assert_eq!(prepare_cell("__Q__", &replacements), "\"a\"\"b\"");
    }

    #[test]
    fn from_f64_formats_whole_and_fractional() {
        assert_eq!(
            ReplacementValue::from_f64(10.0),
            ReplacementValue::from_i64(10)
        );
        let frac = map(&[("__N__", ReplacementValue::from_f64(1.5))]);
        assert_eq!(prepare_cell("__N__", &frac), "1.5");
        let huge = (1i64 << 53) as f64;
        let huge_map = map(&[("__N__", ReplacementValue::from_f64(huge))]);
        assert_eq!(prepare_cell("__N__", &huge_map), huge.to_string());
        assert!(format_number(f64::NAN).eq_ignore_ascii_case("nan"));
        assert!(format_number(f64::INFINITY)
            .to_ascii_lowercase()
            .contains("inf"));
    }

    #[test]
    fn does_not_replace_inside_excel_string_literal() {
        let replacements = map(&[("__NAME__", ReplacementValue::from_text("Alice"))]);
        assert_eq!(
            prepare_cell("=\"Hi __NAME__\"", &replacements),
            "=\"Hi __NAME__\""
        );
        assert_eq!(
            prepare_cell("=\"Hi \"&__NAME__", &replacements),
            "=\"Hi \"&\"Alice\""
        );
        assert_eq!(
            prepare_cell("=\"a\"\"__NAME__\"\"b\"&__NAME__", &replacements),
            "=\"a\"\"__NAME__\"\"b\"&\"Alice\""
        );
    }

    #[test]
    fn leading_whitespace_formula_is_still_formula() {
        let replacements = map(&[("__RATE__", ReplacementValue::from_i64(10))]);
        assert_eq!(prepare_cell(" =__RATE__*2", &replacements), " =10*2");
    }

    #[test]
    fn multiple_placeholders_in_one_formula() {
        let replacements = map(&[
            ("__A__", ReplacementValue::from_i64(2)),
            ("__B__", ReplacementValue::from_i64(3)),
        ]);
        assert_eq!(prepare_cell("=__A__+__B__", &replacements), "=2+3");
    }

    #[test]
    fn adjacent_placeholders_without_separator() {
        // Documented footgun: adjacent text inserts become one Excel string with
        // an embedded quote (`="X""Y"` → X"Y), not concatenation. Prefer `&`.
        let replacements = map(&[
            ("__A__", ReplacementValue::from_text("X")),
            ("__B__", ReplacementValue::from_text("Y")),
        ]);
        assert_eq!(
            prepare_cell("=__A____B__", &replacements),
            "=\"X\"\"Y\""
        );
    }

    #[test]
    fn keeps_escaped_excel_string_literal_cell() {
        let replacements = HashMap::new();
        assert_eq!(prepare_cell("\"a\"\"b\"", &replacements), "\"a\"\"b\"");
    }

    #[test]
    fn unclosed_quote_is_wrapped_as_bare_text() {
        let replacements = HashMap::new();
        assert_eq!(prepare_cell("\"open", &replacements), "\"\"\"open\"");
    }

    #[test]
    fn prepare_cells_drops_invalid_keys_and_prepares_all() {
        let mut replacements = HashMap::new();
        replacements.insert("__OK__".into(), ReplacementValue::from_i64(5));
        replacements.insert("bad".into(), ReplacementValue::from_i64(9));
        let prepared = prepare_cells(&[("A1", "=__OK__"), ("B1", "hi")], &replacements);
        assert_eq!(prepared.cells[0], ("A1".into(), "=5".into()));
        assert_eq!(prepared.cells[1], ("B1".into(), "\"hi\"".into()));
        assert_eq!(prepared.ignored_replacement_keys, vec!["bad".to_string()]);
    }

    #[test]
    fn is_valid_placeholder_key_edges() {
        assert!(is_valid_placeholder_key("__A__"));
        assert!(is_valid_placeholder_key("__A1__"));
        assert!(!is_valid_placeholder_key("____"));
        assert!(!is_valid_placeholder_key("__"));
        assert!(!is_valid_placeholder_key("__a__"));
    }
}
