//! Cloze expansion: turn the `==…==` shorthand into Anki's `{{cN::…}}`.
//!
//! Runs *before* the inline grammar, so the markdown inside a deletion
//! (`==**bold**==`) is still rendered. The scan is code-aware: `==` inside an
//! inline `` `code` `` span is left literal. Mixing `==` with native
//! `{{cN::…}}` in one field is rejected.

use std::ops::Range;

#[derive(Debug)]
pub struct MixedCloze;

/// Replace each `==inner==` with `{{cN::inner}}` (auto-incrementing `N`).
/// `inner` is copied verbatim, so `==a::hint==` becomes `{{c1::a::hint}}`.
pub fn expand(field: &str) -> Result<String, MixedCloze> {
    let spans = shorthand_spans(field);
    if !spans.is_empty() && has_native_cloze(field) {
        return Err(MixedCloze);
    }
    let mut out = String::with_capacity(field.len());
    let mut copied = 0;
    for (n, span) in spans.iter().enumerate() {
        out.push_str(&field[copied..span.start]);
        out.push_str("{{c");
        out.push_str(&(n + 1).to_string());
        out.push_str("::");
        out.push_str(&field[span.start + 2..span.end - 2]);
        out.push_str("}}");
        copied = span.end;
    }
    out.push_str(&field[copied..]);
    Ok(out)
}

/// The byte range of every `==inner==` deletion in `field`, delimiters
/// included, in order. `==` inside inline code or math is not a deletion.
pub fn shorthand_spans(field: &str) -> Vec<Range<usize>> {
    let mut spans = Vec::new();
    let mut pos = 0;
    let mut in_code = false;

    while let Some(ch) = field[pos..].chars().next() {
        let rest = &field[pos..];
        if ch == '`' {
            in_code = !in_code;
            pos += 1;
            continue;
        }
        // Skip `$…$` / `$$…$$` math so a body containing `==` (e.g.
        // `$a == b$`) is not mistaken for a cloze. Mirrors the `in_code` guard.
        if !in_code && ch == '$' {
            pos += math_len(rest);
            continue;
        }
        if !in_code
            && let Some(after_open) = rest.strip_prefix("==")
            && let Some(close) = after_open.find("==")
        {
            let end = pos + 2 + close + 2;
            spans.push(pos..end);
            pos = end;
            continue;
        }
        pos += ch.len_utf8();
    }
    spans
}

/// The length of the `$…$` / `$$…$$` math span at the start of `rest` (which
/// begins with `$`). If there is no matching closer, only the opening `$` is
/// counted so the scan can continue past it.
fn math_len(rest: &str) -> usize {
    let (delim, len) = if rest.starts_with("$$") {
        ("$$", 2)
    } else {
        ("$", 1)
    };
    match rest[len..].find(delim) {
        Some(close) => len + close + len,
        None => 1,
    }
}

/// Does the text already contain a native `{{cN::…}}` deletion?
pub fn has_native_cloze(s: &str) -> bool {
    let mut rest = s;
    while let Some(pos) = rest.find("{{c") {
        let tail = &rest[pos + 3..];
        let digits: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !digits.is_empty() && tail[digits.len()..].starts_with("::") {
            return true;
        }
        rest = &rest[pos + 3..];
    }
    false
}

/// Does the text contain the `==…==` shorthand (outside inline code)?
pub fn has_shorthand_cloze(s: &str) -> bool {
    let mut rest = s;
    let mut in_code = false;
    while let Some(ch) = rest.chars().next() {
        if ch == '`' {
            in_code = !in_code;
        } else if !in_code
            && let Some(after) = rest.strip_prefix("==")
            && after.contains("==")
        {
            return true;
        }
        rest = &rest[ch.len_utf8()..];
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_left_to_right() {
        assert_eq!(
            expand("a ==b== and ==c==").unwrap(),
            "a {{c1::b}} and {{c2::c}}"
        );
    }

    #[test]
    fn hint_passthrough() {
        assert_eq!(
            expand("==Paris::capital==").unwrap(),
            "{{c1::Paris::capital}}"
        );
    }

    #[test]
    fn keeps_markdown_inside() {
        assert_eq!(expand("==**SYN**==").unwrap(), "{{c1::**SYN**}}");
    }

    #[test]
    fn skips_code_spans() {
        assert_eq!(expand("`==x==` ==y==").unwrap(), "`==x==` {{c1::y}}");
    }

    #[test]
    fn native_passthrough() {
        assert_eq!(expand("{{c1::Paris}}").unwrap(), "{{c1::Paris}}");
    }

    #[test]
    fn mixed_is_error() {
        assert!(expand("{{c1::Paris}} and ==Rome==").is_err());
    }

    #[test]
    fn skips_math_spans() {
        // `==` inside math is not a cloze.
        assert_eq!(expand("$a == b$ ==y==").unwrap(), "$a == b$ {{c1::y}}");
        assert_eq!(expand("$$a == b$$ ==y==").unwrap(), "$$a == b$$ {{c1::y}}");
    }

    #[test]
    fn cloze_around_math() {
        // A deletion wrapping math keeps the math for the inline stage.
        assert_eq!(expand("==$x^2$==").unwrap(), "{{c1::$x^2$}}");
    }

    #[test]
    fn spans_cover_the_delimiters() {
        let s = "a ==b== `==c==` $x == y$ ==d::hint==";
        let got: Vec<&str> = shorthand_spans(s).into_iter().map(|r| &s[r]).collect();
        assert_eq!(got, ["==b==", "==d::hint=="]);
    }
}
