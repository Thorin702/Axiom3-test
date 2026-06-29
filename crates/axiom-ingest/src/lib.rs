//! Document text extraction for the reading pipeline.
//!
//! Turns an uploaded file into plain text the verification engine can chew on.
//! Supports plain text / Markdown, DOCX (an Office Open XML zip whose
//! `word/document.xml` holds the body), and PDF (best-effort — extraction quality
//! varies with how the PDF was produced).

use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("unsupported file type: .{0}")]
    Unsupported(String),
    #[error("docx parse error: {0}")]
    Docx(String),
    #[error("pdf parse error: {0}")]
    Pdf(String),
    #[error("document too large: {0}")]
    TooLarge(String),
}

pub type Result<T> = std::result::Result<T, IngestError>;

// Resource bounds for UNTRUSTED input — even a user-chosen file may be a zip bomb
// or pathologically huge. Every read is bounded so we error instead of OOM-ing.
const MAX_TXT_BYTES: u64 = 32 * 1024 * 1024; // a plain text/markdown file
const MAX_DOCX_XML: u64 = 64 * 1024 * 1024; // DECOMPRESSED word/document.xml (zip-bomb guard)
const MAX_PDF_BYTES: u64 = 200 * 1024 * 1024; // a PDF file on disk
const MAX_OUTPUT: usize = 16 * 1024 * 1024; // the extracted text we return

/// Extract plain text from a document, dispatching on the file extension. The
/// returned text is capped at [`MAX_OUTPUT`].
pub fn extract_text(path: &Path) -> Result<String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let text = match ext.as_str() {
        "txt" | "text" | "md" | "markdown" => read_text_file(path, MAX_TXT_BYTES)?,
        "docx" => extract_docx(path)?,
        "pdf" => extract_pdf(path)?,
        other => return Err(IngestError::Unsupported(other.to_string())),
    };
    if text.len() > MAX_OUTPUT {
        return Err(IngestError::TooLarge(format!(
            "extracted text is {} bytes (limit {MAX_OUTPUT})",
            text.len()
        )));
    }
    Ok(text)
}

/// Read a text file, bounded to `max` bytes (so a huge file errors, not OOMs).
/// Decoded leniently so a non-UTF-8 file degrades instead of failing outright.
fn read_text_file(path: &Path, max: u64) -> Result<String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(max + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        return Err(IngestError::TooLarge(format!("file exceeds {max} bytes")));
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn extract_pdf(path: &Path) -> Result<String> {
    let size = std::fs::metadata(path)?.len();
    if size > MAX_PDF_BYTES {
        return Err(IngestError::TooLarge(format!("pdf is {size} bytes (limit {MAX_PDF_BYTES})")));
    }
    match pdf_extract::extract_text(path) {
        Ok(text) => Ok(text),
        Err(first) => {
            // Some exporters (notably CNKI and other Chinese journal databases) append
            // a metadata blob AFTER the PDF's `%%EOF`, which trips strict trailer
            // parsing ("invalid file trailer"). The bytes up to and including the last
            // `%%EOF` are a complete, valid PDF — retry on that truncated view before
            // giving up.
            let bytes = std::fs::read(path)?;
            if let Some(end) = end_of_last_eof(&bytes) {
                if end < bytes.len() {
                    if let Ok(text) = pdf_extract::extract_text_from_mem(&bytes[..end]) {
                        return Ok(text);
                    }
                }
            }
            Err(IngestError::Pdf(first.to_string()))
        }
    }
}

/// Byte index just past the last `%%EOF` marker, if any — the natural end of a valid
/// PDF. Used to discard trailing junk that some exporters append after `%%EOF`.
fn end_of_last_eof(bytes: &[u8]) -> Option<usize> {
    const EOF: &[u8] = b"%%EOF";
    bytes.windows(EOF.len()).rposition(|w| w == EOF).map(|p| p + EOF.len())
}

fn extract_docx(path: &Path) -> Result<String> {
    use std::io::Read;
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| IngestError::Docx(e.to_string()))?;
    // Bound the DECOMPRESSED size: a tiny docx can inflate to gigabytes (zip bomb).
    let mut bytes = Vec::new();
    archive
        .by_name("word/document.xml")
        .map_err(|e| IngestError::Docx(format!("no word/document.xml: {e}")))?
        .take(MAX_DOCX_XML + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_DOCX_XML {
        return Err(IngestError::TooLarge("word/document.xml exceeds the size limit".into()));
    }
    let xml = String::from_utf8_lossy(&bytes);
    Ok(docx_xml_to_text(&xml))
}

/// Pull readable text out of a WordprocessingML body: `<w:t>` holds text runs,
/// `<w:p>` ends a paragraph, `<w:tab>` a tab, `<w:br>`/`<w:cr>` a line break.
/// `local_name` ignores the `w:` namespace prefix.
fn docx_xml_to_text(xml: &str) -> String {
    use quick_xml::events::Event;
    use quick_xml::reader::Reader;

    let mut reader = Reader::from_str(xml);
    let mut out = String::new();
    let mut in_text = false;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) if e.local_name().as_ref() == b"t" => in_text = true,
            Ok(Event::End(e)) => match e.local_name().as_ref() {
                b"t" => in_text = false,
                b"p" => out.push('\n'),
                _ => {}
            },
            Ok(Event::Empty(e)) => match e.local_name().as_ref() {
                b"tab" => out.push('\t'),
                b"br" | b"cr" => out.push('\n'),
                _ => {}
            },
            Ok(Event::Text(t)) if in_text => {
                if let Ok(decoded) = t.decode() {
                    match quick_xml::escape::unescape(&decoded) {
                        Ok(u) => out.push_str(&u),
                        Err(_) => out.push_str(&decoded),
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn reads_plain_text_and_markdown() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("draft.md");
        std::fs::write(&p, "# Title\n\nThe yield was 94%.").unwrap();
        let text = extract_text(&p).unwrap();
        assert!(text.contains("yield was 94%"));
    }

    #[test]
    fn oversize_input_is_bounded() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("big.txt");
        std::fs::write(&p, "x".repeat(1000)).unwrap();
        // The read is bounded — too-big errors instead of allocating unboundedly.
        assert!(matches!(read_text_file(&p, 100), Err(IngestError::TooLarge(_))));
        assert!(read_text_file(&p, 100_000).is_ok());
    }

    #[test]
    fn end_of_last_eof_finds_the_trailer_end() {
        assert_eq!(end_of_last_eof(b"...%%EOF"), Some(8));
        // Trailing junk after %%EOF (CNKI-style): the index points just past %%EOF.
        let b = b"head %%EOF\nWebFastLoad<FileProperty/>";
        assert_eq!(&b[..end_of_last_eof(b).unwrap()], b"head %%EOF");
        // No marker → None (nothing to truncate).
        assert_eq!(end_of_last_eof(b"no marker here"), None);
    }

    #[test]
    fn unsupported_extension_errors() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("x.rtf");
        std::fs::write(&p, "x").unwrap();
        assert!(matches!(extract_text(&p), Err(IngestError::Unsupported(_))));
    }

    #[test]
    fn extracts_text_from_a_minimal_docx() {
        // Build a minimal valid .docx (zip with word/document.xml).
        let dir = tempdir().unwrap();
        let p = dir.path().join("paper.docx");
        let body = r#"<?xml version="1.0"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
 <w:body>
  <w:p><w:r><w:t>The catalyst is</w:t></w:r><w:r><w:t xml:space="preserve"> novel.</w:t></w:r></w:p>
  <w:p><w:r><w:t>Yield was 94 percent.</w:t></w:r></w:p>
 </w:body>
</w:document>"#;
        {
            let f = std::fs::File::create(&p).unwrap();
            let mut zw = zip::ZipWriter::new(f);
            let opts: zip::write::FileOptions<'_, ()> =
                zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            zw.start_file("word/document.xml", opts).unwrap();
            zw.write_all(body.as_bytes()).unwrap();
            zw.finish().unwrap();
        }
        let text = extract_text(&p).unwrap();
        assert!(text.contains("The catalyst is novel."), "got: {text:?}");
        assert!(text.contains("Yield was 94 percent."), "got: {text:?}");
    }
}
