//! Response body decompression driven by `Content-Encoding`.

use std::io::Read;

use flate2::read::{DeflateDecoder, GzDecoder, ZlibDecoder};

use crate::kernel::traffic::Headers;

fn read_all(mut reader: impl Read) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    reader.read_to_end(&mut out).ok()?;
    Some(out)
}

/// One decoding step; `None` when the data does not decode. Unknown encodings pass through.
fn decode(encoding: &str, data: &[u8]) -> Option<Vec<u8>> {
    match encoding {
        "gzip" | "x-gzip" => read_all(GzDecoder::new(data)),
        // HTTP "deflate" is ambiguous: zlib-wrapped (RFC 1950) first, then raw (RFC 1951).
        "deflate" => read_all(ZlibDecoder::new(data)).or_else(|| read_all(DeflateDecoder::new(data))),
        "br" => read_all(brotli::Decompressor::new(data, 4096)),
        _ => Some(data.to_vec()),
    }
}

/// Decompress a body according to a `Content-Encoding` value. Stacked encodings (comma separated, in the
/// order they were applied) are peeled outermost first. `identity`, `zstd` and unknown encodings pass
/// through; when a step fails the original input is returned.
pub fn decompress_body(body: &[u8], content_encoding: &str) -> Vec<u8> {
    if content_encoding.is_empty() || body.is_empty() {
        return body.to_vec();
    }
    let encodings: Vec<String> =
        content_encoding.split(',').map(|e| e.trim().to_lowercase()).filter(|e| !e.is_empty()).rev().collect();
    let mut result = body.to_vec();
    for encoding in encodings {
        match decode(&encoding, &result) {
            Some(next) => result = next,
            None => return body.to_vec(),
        }
    }
    result
}

/// A copy of `headers` without `content-encoding` (any case). Used when a decompressed body is stored, so a
/// viewer does not decode it a second time.
pub fn strip_content_encoding(headers: &Headers) -> Headers {
    headers
        .iter()
        .filter(|(name, _)| !name.eq_ignore_ascii_case("content-encoding"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use flate2::{
        Compression,
        write::{DeflateEncoder, GzEncoder, ZlibEncoder},
    };

    use super::*;

    const TEXT: &[u8] = b"hello hello hello hello, compressed world";

    fn gzip(data: &[u8]) -> Vec<u8> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    fn zlib(data: &[u8]) -> Vec<u8> {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    fn raw_deflate(data: &[u8]) -> Vec<u8> {
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    fn brotli_compress(data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut writer = brotli::CompressorWriter::new(&mut out, 4096, 5, 22);
            writer.write_all(data).unwrap();
        }
        out
    }

    #[test]
    fn round_trips() {
        assert_eq!(decompress_body(&gzip(TEXT), "gzip"), TEXT);
        assert_eq!(decompress_body(&gzip(TEXT), "X-GZIP"), TEXT);
        assert_eq!(decompress_body(&zlib(TEXT), "deflate"), TEXT);
        assert_eq!(decompress_body(&raw_deflate(TEXT), "deflate"), TEXT);
        assert_eq!(decompress_body(&brotli_compress(TEXT), "br"), TEXT);
    }

    #[test]
    fn stacked_encodings_peel_the_last_first() {
        let stacked = gzip(&zlib(TEXT));
        assert_eq!(decompress_body(&stacked, "deflate, gzip"), TEXT);
    }

    #[test]
    fn failures_and_unknowns_return_the_input() {
        assert_eq!(decompress_body(b"not gzip at all", "gzip"), b"not gzip at all");
        assert_eq!(decompress_body(b"not brotli \xff\xfe", "br"), b"not brotli \xff\xfe");
        assert_eq!(decompress_body(TEXT, "zstd"), TEXT);
        assert_eq!(decompress_body(TEXT, "identity"), TEXT);
        assert_eq!(decompress_body(TEXT, ""), TEXT);
        assert_eq!(decompress_body(b"", "gzip"), b"");
        // The second step fails: the original input, not the half-decoded one.
        let once = gzip(TEXT);
        assert_eq!(decompress_body(&once, "gzip, gzip"), once);
    }

    #[test]
    fn strip_removes_content_encoding_only() {
        let mut headers = Headers::new();
        headers.insert("Content-Encoding".into(), "gzip".into());
        headers.insert("content-type".into(), "text/plain".into());
        let stripped = strip_content_encoding(&headers);
        assert_eq!(stripped.len(), 1);
        assert!(stripped.contains_key("content-type"));
    }
}
