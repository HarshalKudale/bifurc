//! Finding `{{variable}}` references in text.
//!
//! The UI only marks where the references are. What a reference resolves to is the engine's business
//! (`env.resolveText`, and the send commands resolve on their own), so nothing here looks a name up.

use std::ops::Range;

/// The byte ranges of every `{{name}}` in `text`, braces included.
///
/// A reference is `{{`, a name without braces or line breaks, and `}}`. An unclosed `{{` is not a reference,
/// and neither is an empty `{{}}`.
pub fn variable_ranges(text: &str) -> Vec<Range<usize>> {
    let bytes = text.as_bytes();
    let mut ranges = Vec::new();
    let mut ix = 0;
    while ix + 1 < bytes.len() {
        if bytes[ix] != b'{' || bytes[ix + 1] != b'{' {
            ix += 1;
            continue;
        }
        let name_start = ix + 2;
        let mut end = name_start;
        let mut closed = false;
        while end < bytes.len() {
            match bytes[end] {
                b'}' if end + 1 < bytes.len() && bytes[end + 1] == b'}' => {
                    closed = true;
                    break;
                }
                b'{' | b'}' | b'\n' | b'\r' => break,
                _ => end += 1,
            }
        }
        if closed && !text[name_start..end].trim().is_empty() {
            ranges.push(ix..end + 2);
            ix = end + 2;
        } else {
            // Step one byte so that `{{{name}}` still finds the reference that starts at the second brace.
            ix += 1;
        }
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(text: &str) -> Vec<&str> {
        variable_ranges(text).into_iter().map(|r| &text[r]).collect()
    }

    #[test]
    fn references_are_found_with_their_braces() {
        assert_eq!(found("{{HOST}}/users/{{id}}"), ["{{HOST}}", "{{id}}"]);
    }

    #[test]
    fn spaces_inside_the_braces_belong_to_the_reference() {
        assert_eq!(found("Bearer {{ token }}"), ["{{ token }}"]);
    }

    #[test]
    fn unclosed_and_empty_braces_are_not_references() {
        assert!(found("{{HOST/users").is_empty());
        assert!(found("{{}} and {{  }}").is_empty());
        assert!(found("{{a\nb}}").is_empty());
    }

    #[test]
    fn json_braces_around_a_reference_do_not_hide_it() {
        assert_eq!(found(r#"{"a":{"b":"{{x}}"}}"#), ["{{x}}"]);
        assert_eq!(found("{{{x}}}"), ["{{x}}"]);
    }

    #[test]
    fn ranges_are_byte_offsets_and_survive_text_that_is_not_ascii() {
        let text = "né {{clé}} ü";
        let ranges = variable_ranges(text);
        assert_eq!(&text[ranges[0].clone()], "{{clé}}");
    }
}
