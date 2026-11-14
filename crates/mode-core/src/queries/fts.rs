//! FTS5 query construction (09 §6.3). The app never passes raw user input to
//! `MATCH`: the text is tokenized, every token is quoted, the last token gets
//! `*` while typing, tokens are joined with implicit AND, and a leading
//! `tag:` / `from:` / `subject:` / `title:` / `body:` chip becomes a column
//! filter. An `in:` chip has no FTS column in either table and is returned
//! separately as a scope for the caller (`notes.path` prefix, message
//! `channel_name`).

/// Which FTS table the query targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fts {
    Notes,
    Messages,
}

impl Fts {
    /// `chip` → FTS column.
    fn column(self, chip: &str) -> Option<&'static str> {
        match (self, chip) {
            (Fts::Notes, "tag") | (Fts::Notes, "tags") => Some("tags_text"),
            (Fts::Notes, "title") => Some("title"),
            (Fts::Notes, "body") => Some("body"),
            (Fts::Messages, "from") | (Fts::Messages, "sender") => Some("sender_name"),
            (Fts::Messages, "subject") => Some("subject"),
            (Fts::Messages, "body") => Some("body_text"),
            _ => None,
        }
    }

    /// `bm25()` column weights (09 §6.3).
    pub fn weights(self) -> &'static str {
        match self {
            Fts::Notes => "4.0, 1.0, 2.0, 0.0",
            Fts::Messages => "3.0, 1.0, 1.0, 2.0",
        }
    }
}

/// A built query.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    /// The `MATCH` expression; `None` when only an `in:` chip was given.
    pub match_expr: Option<String>,
    /// The `in:` chip value, for the caller to apply outside FTS.
    pub in_scope: Option<String>,
}

fn quote(token: &str) -> String {
    format!("\"{}\"", token.replace('"', "\"\""))
}

/// Build the `MATCH` expression. `prefix_last` appends `*` to the last plain
/// token (the query bar while typing). Returns `None` for blank input.
pub fn build(text: &str, table: Fts, prefix_last: bool) -> Option<Query> {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    if tokens.is_empty() {
        return None;
    }
    let mut parts: Vec<String> = Vec::new();
    let mut in_scope = None;
    let last_index = tokens.len() - 1;
    for (i, raw) in tokens.iter().enumerate() {
        let is_last = i == last_index;
        if let Some((chip, rest)) = raw.split_once(':')
            && !rest.is_empty()
            && !chip.is_empty()
        {
            let chip = chip.to_ascii_lowercase();
            if chip == "in" {
                in_scope = Some(rest.to_string());
                continue;
            }
            if let Some(col) = table.column(&chip) {
                parts.push(format!("{col}:{}", quote(rest)));
                continue;
            }
        }
        let token: String = raw.chars().filter(|c| *c != '*').collect();
        if token.is_empty() {
            continue;
        }
        let mut q = quote(&token);
        if is_last && prefix_last {
            q.push('*');
        }
        parts.push(q);
    }
    Some(Query {
        match_expr: if parts.is_empty() {
            None
        } else {
            Some(parts.join(" "))
        },
        in_scope,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_quoted_and_joined() {
        let q = build("buy  milk", Fts::Notes, false).unwrap();
        assert_eq!(q.match_expr.as_deref(), Some(r#""buy" "milk""#));
        assert_eq!(q.in_scope, None);
        let q = build("buy mi", Fts::Notes, true).unwrap();
        assert_eq!(q.match_expr.as_deref(), Some(r#""buy" "mi"*"#));
        assert!(build("   ", Fts::Notes, true).is_none());
    }

    #[test]
    fn chips_become_column_filters_and_syntax_is_neutralised() {
        let q = build(r#"tag:gym OR "x" NOT y*"#, Fts::Notes, false).unwrap();
        assert_eq!(
            q.match_expr.as_deref(),
            Some(r#"tags_text:"gym" "OR" """x""" "NOT" "y""#)
        );
        let q = build("from:bjorn in:general hello", Fts::Messages, true).unwrap();
        assert_eq!(
            q.match_expr.as_deref(),
            Some(r#"sender_name:"bjorn" "hello"*"#)
        );
        assert_eq!(q.in_scope.as_deref(), Some("general"));
        let q = build("in:Work/", Fts::Notes, true).unwrap();
        assert_eq!(q.match_expr, None);
        assert_eq!(q.in_scope.as_deref(), Some("Work/"));
        // A chip the table does not know is a plain token.
        let q = build("from:x", Fts::Notes, false).unwrap();
        assert_eq!(q.match_expr.as_deref(), Some(r#""from:x""#));
        assert_eq!(Fts::Notes.weights(), "4.0, 1.0, 2.0, 0.0");
    }
}
