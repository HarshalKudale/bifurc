//! Content types: which are text, which are images, and what to call a file of each.
//!
//! The engine decides how a body is stored and sent; this is only what the views need to pick the right
//! control for a body (a text editor, an image, a file) and a file name when one is saved.

use crate::i18n::t_with;

/// How a body of some content type is shown and edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyClass {
    /// Text of any kind: JSON, HTML, XML, JavaScript, plain.
    Text,
    /// An image a preview can be drawn for.
    Image,
    /// Any other data: shown by its size, replaced by choosing a file.
    Binary,
}

/// The content type without its parameters, lower-cased: `Text/HTML; charset=utf-8` reads `text/html`.
pub fn essence(content_type: &str) -> String {
    content_type.split(';').next().unwrap_or_default().trim().to_ascii_lowercase()
}

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

/// The class of a body with this `Content-Type`. An empty or unknown type reads as text: that is what a
/// body with no label has always been edited as.
pub fn classify(content_type: &str) -> BodyClass {
    let kind = essence(content_type);
    if kind.is_empty() || kind.starts_with("text/") || TEXT_APPLICATION_TYPES.contains(&kind.as_str()) {
        return BodyClass::Text;
    }
    // Structured-syntax suffixes (`application/problem+json`, `image/svg+xml`) are text whatever the prefix.
    if kind.ends_with("+json") || kind.ends_with("+xml") {
        return BodyClass::Text;
    }
    if kind.starts_with("image/") {
        return BodyClass::Image;
    }
    if kind.starts_with("multipart/") {
        return BodyClass::Binary;
    }
    match kind.split('/').next() {
        Some("audio" | "video" | "font" | "application") => BodyClass::Binary,
        _ => BodyClass::Text,
    }
}

const EXTENSIONS: [(&str, &str); 28] = [
    ("image/png", "png"),
    ("image/jpeg", "jpg"),
    ("image/jpeg", "jpeg"),
    ("image/gif", "gif"),
    ("image/webp", "webp"),
    ("image/bmp", "bmp"),
    ("image/x-icon", "ico"),
    ("image/tiff", "tiff"),
    ("image/svg+xml", "svg"),
    ("application/pdf", "pdf"),
    ("application/zip", "zip"),
    ("application/gzip", "gz"),
    ("application/json", "json"),
    ("application/xml", "xml"),
    ("application/javascript", "js"),
    ("application/wasm", "wasm"),
    ("text/html", "html"),
    ("text/css", "css"),
    ("text/csv", "csv"),
    ("text/plain", "txt"),
    ("audio/mpeg", "mp3"),
    ("audio/wav", "wav"),
    ("video/mp4", "mp4"),
    ("video/webm", "webm"),
    ("font/woff", "woff"),
    ("font/woff2", "woff2"),
    ("font/ttf", "ttf"),
    ("application/octet-stream", "bin"),
];

/// The file extension a body of this type is saved with, without the dot.
pub fn extension_for(content_type: &str) -> &'static str {
    let kind = essence(content_type);
    EXTENSIONS.iter().find(|(mime, _)| *mime == kind).map_or("bin", |(_, extension)| extension)
}

/// The content type of a file, from its name. A name that says nothing is `application/octet-stream`.
pub fn content_type_for(file_name: &str) -> &'static str {
    let extension = file_name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    EXTENSIONS.iter().find(|(_, e)| *e == extension).map_or("application/octet-stream", |(mime, _)| mime)
}

/// A short word for a content type, for a narrow column: `json`, `html`, `png`, `font`.
pub fn short_label(content_type: &str) -> String {
    let kind = essence(content_type);
    if kind.is_empty() {
        return String::new();
    }
    if kind.contains("json") {
        return "json".to_owned();
    }
    if kind.contains("javascript") || kind.contains("ecmascript") {
        return "js".to_owned();
    }
    // Not "stream": that names the WebSocket and event-stream rows.
    if kind == "application/octet-stream" {
        return "binary".to_owned();
    }
    let (family, subtype) = kind.split_once('/').unwrap_or(("", &kind));
    match family {
        "font" | "audio" | "video" => family.to_owned(),
        _ => subtype.rsplit(['+', '.', '-']).next().unwrap_or(subtype).to_owned(),
    }
}

/// A size in the unit a person would pick.
pub fn size_label(bytes: usize) -> String {
    if bytes < 1024 {
        t_with("model.size.bytes", &[("n", &bytes)])
    } else if bytes < 1024 * 1024 {
        t_with("model.size.kilobytes", &[("n", &format!("{:.1}", bytes as f64 / 1024.0))])
    } else {
        t_with("model.size.megabytes", &[("n", &format!("{:.1}", bytes as f64 / (1024.0 * 1024.0)))])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_types_are_text_whatever_their_parameters_or_case() {
        assert_eq!(classify("Application/JSON; charset=utf-8"), BodyClass::Text);
        assert_eq!(classify("text/html"), BodyClass::Text);
        assert_eq!(classify("application/problem+json"), BodyClass::Text);
        assert_eq!(classify("image/svg+xml"), BodyClass::Text);
        assert_eq!(classify(""), BodyClass::Text);
    }

    #[test]
    fn images_and_other_data_are_told_apart() {
        assert_eq!(classify("image/png"), BodyClass::Image);
        assert_eq!(classify("application/pdf"), BodyClass::Binary);
        assert_eq!(classify("application/octet-stream"), BodyClass::Binary);
        assert_eq!(classify("font/woff2"), BodyClass::Binary);
        assert_eq!(classify("multipart/form-data; boundary=x"), BodyClass::Binary);
    }

    #[test]
    fn a_file_name_and_a_content_type_name_each_other() {
        assert_eq!(content_type_for("Logo.PNG"), "image/png");
        assert_eq!(content_type_for("photo.jpeg"), "image/jpeg");
        assert_eq!(content_type_for("mystery"), "application/octet-stream");
        assert_eq!(extension_for("image/jpeg; q=1"), "jpg");
        assert_eq!(extension_for("application/x-unknown"), "bin");
    }

    #[test]
    fn short_labels_fit_a_narrow_column() {
        assert_eq!(short_label("application/json; charset=utf-8"), "json");
        assert_eq!(short_label("text/html"), "html");
        assert_eq!(short_label("image/svg+xml"), "xml");
        assert_eq!(short_label("font/woff2"), "font");
        assert_eq!(short_label("application/javascript"), "js");
        assert_eq!(short_label(""), "");
        assert_eq!(short_label("application/octet-stream"), "binary");
    }

    #[test]
    fn sizes_read_in_the_unit_a_person_would_pick() {
        assert_eq!(size_label(10), "10 B");
        assert_eq!(size_label(2048), "2.0 KB");
        assert_eq!(size_label(3 * 1024 * 1024), "3.0 MB");
    }
}
