//! Cloze expansion: turn the `==…==` shorthand into Anki's `{{cN::…}}`.
//!
//! Runs *before* the inline grammar, so the markdown inside a deletion
//! (`==**bold**==`) is still rendered. The scan is code-aware: `==` inside an
//! inline `` `code` `` span is left literal. Mixing `==` with native
//! `{{cN::…}}` in one field is rejected.

#[derive(Debug)]
pub struct MixedCloze;

/// Replace each `==inner==` with `{{cN::inner}}` (auto-incrementing `N`).
/// `inner` is copied verbatim, so `==a::hint==` becomes `{{c1::a::hint}}`.
pub fn expand(field: &str) -> Result<String, MixedCloze> {
    let native = has_native_cloze(field);
    let mut out = String::with_capacity(field.len());
    let mut rest = field;
    let mut n = 1u32;
    let mut found_auto = false;
    let mut in_code = false;

    while let Some(ch) = rest.chars().next() {
        if ch == '`' {
            in_code = !in_code;
            out.push('`');
            rest = &rest[1..];
            continue;
        }
        if !in_code {
            if let Some(after_open) = rest.strip_prefix("==") {
                if let Some(close) = after_open.find("==") {
                    let inner = &after_open[..close];
                    out.push_str("{{c");
                    out.push_str(&n.to_string());
                    out.push_str("::");
                    out.push_str(inner);
                    out.push_str("}}");
                    n += 1;
                    found_auto = true;
                    rest = &after_open[close + 2..];
                    continue;
                }
            }
        }
        out.push(ch);
        rest = &rest[ch.len_utf8()..];
    }

    if found_auto && native {
        return Err(MixedCloze);
    }
    Ok(out)
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
        } else if !in_code {
            if let Some(after) = rest.strip_prefix("==") {
                if after.contains("==") {
                    return true;
                }
            }
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
        assert_eq!(expand("a ==b== and ==c==").unwrap(), "a {{c1::b}} and {{c2::c}}");
    }

    #[test]
    fn hint_passthrough() {
        assert_eq!(expand("==Paris::capital==").unwrap(), "{{c1::Paris::capital}}");
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
}
