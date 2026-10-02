//! A body that is not text: an image shown as itself, anything else by its type and size, and the two
//! things done with either one — saving it to a file and replacing it with a file.
//!
//! Requests, responses, mocks and captured traffic all meet such bodies, so they share this.

use std::{path::PathBuf, sync::Arc};

use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Image, ImageFormat, InteractiveElement as _, IntoElement, ObjectFit,
    ParentElement as _, PathPromptOptions, StatefulInteractiveElement as _, Styled as _, StyledImage as _, div, img,
};

use crate::model::media::{self, BodyClass};

/// The bytes of a body, with what is needed to show them. Built once when the body arrives, not per frame:
/// an image is decoded from it.
#[derive(Clone)]
pub struct BinaryBody {
    bytes: Arc<Vec<u8>>,
    content_type: String,
    image: Option<Arc<Image>>,
}

fn image_format(content_type: &str, bytes: &[u8]) -> Option<ImageFormat> {
    match media::essence(content_type).as_str() {
        "image/png" => return Some(ImageFormat::Png),
        "image/jpeg" | "image/jpg" => return Some(ImageFormat::Jpeg),
        "image/gif" => return Some(ImageFormat::Gif),
        "image/webp" => return Some(ImageFormat::Webp),
        "image/bmp" => return Some(ImageFormat::Bmp),
        "image/svg+xml" => return Some(ImageFormat::Svg),
        "image/tiff" => return Some(ImageFormat::Tiff),
        "image/x-icon" | "image/vnd.microsoft.icon" => return Some(ImageFormat::Ico),
        _ => {}
    }
    // A mislabelled or unlabelled image is still recognised by how it starts.
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => Some(ImageFormat::Png),
        [0xFF, 0xD8, 0xFF, ..] => Some(ImageFormat::Jpeg),
        [b'G', b'I', b'F', b'8', ..] => Some(ImageFormat::Gif),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some(ImageFormat::Webp),
        [b'B', b'M', ..] if media::classify(content_type) == BodyClass::Image => Some(ImageFormat::Bmp),
        _ => None,
    }
}

impl BinaryBody {
    pub fn new(bytes: Vec<u8>, content_type: impl Into<String>) -> Self {
        let content_type = content_type.into();
        let image =
            image_format(&content_type, &bytes).map(|format| Arc::new(Image::from_bytes(format, bytes.clone())));
        Self { bytes: Arc::new(bytes), content_type, image }
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn content_type(&self) -> &str {
        &self.content_type
    }

    pub fn is_image(&self) -> bool {
        self.image.is_some()
    }

    /// One line about the body: its type and its size.
    pub fn summary(&self) -> String {
        let kind = media::essence(&self.content_type);
        let size = media::size_label(self.bytes.len());
        if kind.is_empty() { size } else { format!("{kind} · {size}") }
    }

    /// The body as it can be shown: the image, or a line saying what the data is.
    pub fn render(&self, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let caption = div().flex_none().text_sm().text_color(theme.muted_foreground).child(self.summary());
        match &self.image {
            Some(image) => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_3()
                .p_4()
                .child(
                    h_flex()
                        .flex_1()
                        .min_h_0()
                        .w_full()
                        .items_center()
                        .justify_center()
                        .child(img(image.clone()).max_w_full().max_h_full().object_fit(ObjectFit::ScaleDown)),
                )
                .child(caption)
                .into_any_element(),
            None => {
                let shown = self.bytes.len().min(HEX_PREVIEW_BYTES);
                let note = if shown < self.bytes.len() {
                    format!("Data, not text · first {} of {} as hex", media::size_label(shown), media::size_label(self.bytes.len()))
                } else {
                    "Data, not text · shown as hex".to_owned()
                };
                v_flex()
                    .size_full()
                    .gap_2()
                    .p_3()
                    .child(caption)
                    .child(div().flex_none().text_xs().text_color(theme.muted_foreground).child(note))
                    .child(
                        div()
                            .id("binary-hex")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .text_xs()
                            .font_family(theme.mono_font_family.clone())
                            .child(hex_dump(&self.bytes[..shown])),
                    )
                    .into_any_element()
            }
        }
    }
}

/// How much of a data body is laid out as hex: enough to recognise the format, small enough to draw.
const HEX_PREVIEW_BYTES: usize = 4096;

/// `bytes` as classic hex-dump lines: offset, sixteen hex bytes, and the printable characters.
fn hex_dump(bytes: &[u8]) -> String {
    let mut out = String::new();
    for (row, chunk) in bytes.chunks(16).enumerate() {
        let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02x}")).collect();
        let text: String = chunk.iter().map(|b| if (0x20..0x7f).contains(b) { *b as char } else { '.' }).collect();
        out.push_str(&format!("{:08x}  {:<47}  {text}\n", row * 16, hex.join(" ")));
    }
    out
}

fn downloads_dir() -> PathBuf {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from);
    match home {
        Some(home) if home.join("Downloads").is_dir() => home.join("Downloads"),
        Some(home) => home,
        None => PathBuf::from("."),
    }
}

/// A file name for a body: the last segment of the URL it came from when that names a file, else `name`
/// with the extension its content type calls for.
pub fn suggested_file_name(url: &str, name: &str, content_type: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or_default();
    let last = path.rsplit('/').next().unwrap_or_default();
    if last.contains('.') && !last.ends_with('.') && !path.ends_with("//") && path.matches('/').count() > 2 {
        return last.to_owned();
    }
    format!("{name}.{}", media::extension_for(content_type))
}

/// Asks where to save `bytes` and writes them there. `done` hears how it went: the path, or why not.
/// Cancelling the dialog is not an outcome; `done` is not called.
pub fn save_to_file<V: 'static>(
    bytes: Vec<u8>,
    suggested_name: String,
    cx: &mut Context<V>,
    done: impl FnOnce(&mut V, Result<PathBuf, String>, &mut Context<V>) + 'static,
) {
    let chosen = cx.prompt_for_new_path(&downloads_dir(), Some(&suggested_name));
    cx.spawn(async move |this, cx| {
        let Ok(Ok(Some(path))) = chosen.await else { return };
        let target = path.clone();
        let written = cx.background_spawn(async move { std::fs::write(&target, bytes) }).await;
        let outcome = written.map(|_| path).map_err(|error| error.to_string());
        this.update(cx, |this, cx| done(this, outcome, cx)).ok();
    })
    .detach();
}

/// A file the user chose: its name, its bytes and the content type its name suggests.
pub struct PickedFile {
    pub name: String,
    pub bytes: Vec<u8>,
    pub content_type: &'static str,
}

/// Asks for a file and reads it. `done` gets the file, or why it could not be read. Cancelling the dialog
/// is not an outcome; `done` is not called.
pub fn pick_file<V: 'static>(
    prompt: &'static str,
    cx: &mut Context<V>,
    done: impl FnOnce(&mut V, Result<PickedFile, String>, &mut Context<V>) + 'static,
) {
    let chosen = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: false,
        prompt: Some(prompt.into()),
    });
    cx.spawn(async move |this, cx| {
        let Ok(Ok(Some(paths))) = chosen.await else { return };
        let Some(path) = paths.into_iter().next() else { return };
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let read = cx.background_spawn(async move { std::fs::read(&path) }).await;
        let outcome = read
            .map(|bytes| PickedFile { content_type: media::content_type_for(&name), name, bytes })
            .map_err(|error| error.to_string());
        this.update(cx, |this, cx| done(this, outcome, cx)).ok();
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_name_comes_from_the_url_when_it_names_a_file() {
        assert_eq!(suggested_file_name("https://x.test/img/logo.png?v=2", "response", "image/png"), "logo.png");
        assert_eq!(suggested_file_name("https://x.test/avatar", "response", "image/jpeg"), "response.jpg");
        assert_eq!(suggested_file_name("https://x.test", "response", "application/pdf"), "response.pdf");
        assert_eq!(suggested_file_name("", "body", ""), "body.bin");
    }

    #[test]
    fn a_hex_dump_has_offset_bytes_and_printable_text() {
        assert_eq!(hex_dump(b"PDF-1.7\n"), format!("00000000  {:<47}  PDF-1.7.\n", "50 44 46 2d 31 2e 37 0a"));
        assert_eq!(hex_dump(&[0u8; 17]).lines().count(), 2);
    }

    #[test]
    fn an_image_is_known_by_its_label_or_by_how_it_starts() {
        assert_eq!(image_format("image/png", &[]), Some(ImageFormat::Png));
        assert_eq!(image_format("application/octet-stream", &[0xFF, 0xD8, 0xFF, 0xE0]), Some(ImageFormat::Jpeg));
        assert_eq!(image_format("application/pdf", b"%PDF-1.7"), None);
    }
}
