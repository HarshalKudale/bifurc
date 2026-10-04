//! Classifying a content type as text or binary.

const TEXT_APPLICATION_TYPES: [&str; 8] = [
    "application/json",
    "application/xml",
    "application/javascript",
    "application/ecmascript",
    "application/x-www-form-urlencoded",
    "application/graphql",
    "application/x-ndjson",
    "application/yaml",
];

/// Whether a body of this type is text a mock can replace: JSON, HTML, XML, JavaScript, plain text and the
/// other text formats. An empty type counts as text. Images, audio, video, fonts, archives and any other
/// `application/*` data are not.
pub fn is_text_content_type(content_type: &str) -> bool {
    let kind = content_type.split(';').next().unwrap_or("").trim().to_lowercase();
    if kind.is_empty() || kind.starts_with("text/") || TEXT_APPLICATION_TYPES.contains(&kind.as_str()) {
        return true;
    }
    if kind.ends_with("+json") || kind.ends_with("+xml") {
        return true;
    }
    let family = kind.split('/').next().unwrap_or("");
    !matches!(family, "image" | "audio" | "video" | "font" | "application" | "multipart")
}

/// Whether a body of this type must be kept as base64 rather than decoded to text. The list is short and
/// prefix-based on purpose: a binary type missing from it would be decoded as UTF-8 and corrupted.
pub fn is_binary_content_type(content_type: &str) -> bool {
    let lower = content_type.to_lowercase();
    [
        "image/",
        "audio/",
        "video/",
        "font/",
        "application/octet-stream",
        "application/pdf",
        "application/zip",
        "application/gzip",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_types() {
        for ct in [
            "",
            "text/html; charset=utf-8",
            "TEXT/PLAIN",
            "application/json",
            "application/json; charset=utf-8",
            "application/vnd.api+json",
            "application/atom+xml",
            "application/x-www-form-urlencoded",
            "message/rfc822",
        ] {
            assert!(is_text_content_type(ct), "{ct}");
        }
    }

    #[test]
    fn non_text_types() {
        for ct in ["image/png", "audio/mpeg", "video/mp4", "font/woff2", "application/pdf", "multipart/form-data"] {
            assert!(!is_text_content_type(ct), "{ct}");
        }
    }

    #[test]
    fn binary_types() {
        for ct in ["image/svg+xml", "Audio/mpeg", "application/octet-stream", "application/pdf", "font/ttf"] {
            assert!(is_binary_content_type(ct), "{ct}");
        }
        for ct in ["", "text/html", "application/json", "multipart/form-data"] {
            assert!(!is_binary_content_type(ct), "{ct}");
        }
    }
}
