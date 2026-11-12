//! Minimal frontmatter parsing for skills and templates.

use std::collections::BTreeMap;

/// A parsed frontmatter block and the Markdown body that follows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Frontmatter {
    /// Scalar fields, with lowercased keys and unquoted values.
    pub(crate) fields: BTreeMap<String, String>,
    /// The body after the closing delimiter, with leading newlines trimmed.
    pub(crate) body: String,
}

/// Parses an optional `---` delimited frontmatter block.
///
/// Content without frontmatter is returned as the body. A block that is opened
/// but never closed is an error so the caller can skip the file and warn.
pub(crate) fn parse(content: &str) -> Result<Frontmatter, String> {
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    let Some(rest) = content.strip_prefix("---") else {
        return Ok(Frontmatter {
            fields: BTreeMap::new(),
            body: content.to_owned(),
        });
    };
    let rest = rest
        .strip_prefix('\n')
        .or_else(|| rest.strip_prefix("\r\n"))
        .ok_or_else(|| "frontmatter must start on its own line".to_owned())?;

    let mut fields = BTreeMap::new();
    let mut offset = 0usize;
    for line in rest.split_inclusive('\n') {
        let text = line.trim_end_matches(['\r', '\n']);
        if text.trim() == "---" {
            let body = rest.get(offset + line.len()..).unwrap_or_default();
            return Ok(Frontmatter {
                fields,
                body: body.trim_start_matches(['\r', '\n']).to_owned(),
            });
        }
        let text = text.trim();
        if !text.is_empty() && !text.starts_with('#') {
            let Some((key, value)) = text.split_once(':') else {
                return Err(format!("invalid frontmatter line: {text}"));
            };
            fields.insert(key.trim().to_lowercase(), unquote(value.trim()));
        }
        offset += line.len();
    }

    Err("frontmatter is missing its closing `---`".to_owned())
}

/// Removes one layer of matching single or double quotes.
fn unquote(value: &str) -> String {
    for quote in ['"', '\''] {
        if let Some(inner) = value
            .strip_prefix(quote)
            .and_then(|value| value.strip_suffix(quote))
        {
            return inner.to_owned();
        }
    }
    value.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fields_and_body() {
        let parsed =
            parse("---\nname: review\ndescription: \"Check a diff\"\n---\nBody here\n").unwrap();
        assert_eq!(parsed.fields["name"], "review");
        assert_eq!(parsed.fields["description"], "Check a diff");
        assert_eq!(parsed.body, "Body here\n");
    }

    #[test]
    fn content_without_frontmatter_is_all_body() {
        let parsed = parse("# Title\nText").unwrap();
        assert!(parsed.fields.is_empty());
        assert_eq!(parsed.body, "# Title\nText");
    }

    #[test]
    fn unclosed_frontmatter_is_an_error() {
        assert!(parse("---\nname: x\n").is_err());
    }

    #[test]
    fn blank_and_comment_lines_are_ignored() {
        let parsed = parse("---\n# comment\n\nname: x\n---\nbody").unwrap();
        assert_eq!(parsed.fields.len(), 1);
        assert_eq!(parsed.body, "body");
    }
}
