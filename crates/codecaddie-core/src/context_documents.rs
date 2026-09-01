//! Device-local inspection and bounded text extraction for product-context
//! documents. Raw document text exists only in memory and is never serialized
//! across the desktop/core boundary or written to CodeCaddie storage.

use quick_xml::{Reader, events::Event};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
};

pub const MAX_CONTEXT_FILES: usize = 10;
pub const MAX_FILE_BYTES: u64 = 25 * 1024 * 1024;
pub const MAX_TOTAL_FILE_BYTES: u64 = 100 * 1024 * 1024;
pub const MAX_EXTRACTED_CHARS: usize = 100_000;
const MAX_ARCHIVE_ENTRY_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContextMediaType {
    Pdf,
    Pptx,
    Docx,
    Txt,
    Md,
}

impl ContextMediaType {
    fn from_path(path: &Path) -> anyhow::Result<Self> {
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        match extension.as_str() {
            "pdf" => Ok(Self::Pdf),
            "pptx" => Ok(Self::Pptx),
            "docx" => Ok(Self::Docx),
            "txt" => Ok(Self::Txt),
            "md" | "markdown" => Ok(Self::Md),
            _ => anyhow::bail!(
                "unsupported project-context file type; use PDF, PPTX, DOCX, TXT, or Markdown"
            ),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Pdf => "PDF",
            Self::Pptx => "PPTX",
            Self::Docx => "DOCX",
            Self::Txt => "TXT",
            Self::Md => "Markdown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextFileReference {
    pub display_name: String,
    pub path: String,
    pub media_type: ContextMediaType,
    pub size_bytes: u64,
    pub content_hash: String,
    pub unit_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextSourceMetadata {
    pub display_name: String,
    pub media_type: ContextMediaType,
    pub size_bytes: u64,
    pub content_hash: String,
    pub unit_count: u32,
}

#[derive(Debug, Clone)]
pub struct ContextSection {
    pub source_id: String,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct ExtractedContext {
    pub sources: Vec<ContextSourceMetadata>,
    pub sections: Vec<ContextSection>,
}

#[derive(Debug, Clone)]
struct ExtractedUnit {
    identifier: String,
    text: String,
}

impl ExtractedContext {
    pub fn prompt_text(&self) -> String {
        let mut output = String::new();
        for section in &self.sections {
            output.push_str("\n\n[SOURCE ");
            output.push_str(&section.source_id);
            output.push_str("]\n");
            output.push_str(&section.text);
        }
        output.trim().to_string()
    }
}

pub fn inspect_paths(paths: &[String]) -> anyhow::Result<Vec<ContextFileReference>> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    if paths.len() > MAX_CONTEXT_FILES {
        anyhow::bail!("choose no more than {MAX_CONTEXT_FILES} project-context files");
    }
    let mut total_bytes = 0_u64;
    let mut total_chars = 0_usize;
    let mut canonical_paths = BTreeSet::new();
    let mut references = Vec::with_capacity(paths.len());
    for raw_path in paths {
        let path = PathBuf::from(raw_path);
        if !path.is_absolute() {
            anyhow::bail!("project-context files must use absolute paths");
        }
        let input_metadata = fs::symlink_metadata(&path)
            .map_err(|_| anyhow::anyhow!("{} is missing or unreadable", safe_name(&path)))?;
        if input_metadata.file_type().is_symlink() {
            anyhow::bail!(
                "{} is a symbolic link; attach the regular file directly",
                safe_name(&path)
            );
        }
        let canonical = fs::canonicalize(&path)
            .map_err(|_| anyhow::anyhow!("{} is missing or unreadable", safe_name(&path)))?;
        if !canonical_paths.insert(canonical.clone()) {
            anyhow::bail!("{} was selected more than once", safe_name(&canonical));
        }
        let metadata = fs::metadata(&canonical)?;
        if !metadata.is_file() {
            anyhow::bail!("{} is not a regular file", safe_name(&canonical));
        }
        if metadata.len() > MAX_FILE_BYTES {
            anyhow::bail!(
                "{} exceeds the 25 MiB per-file limit",
                safe_name(&canonical)
            );
        }
        total_bytes = total_bytes.saturating_add(metadata.len());
        if total_bytes > MAX_TOTAL_FILE_BYTES {
            anyhow::bail!("project-context files exceed the 100 MiB combined limit");
        }
        let media_type = ContextMediaType::from_path(&canonical)?;
        let bytes = fs::read(&canonical)?;
        let sections = extract_bytes(media_type, &bytes, &safe_name(&canonical))?;
        total_chars = total_chars.saturating_add(
            sections
                .iter()
                .map(|section| section.text.chars().count())
                .sum::<usize>(),
        );
        if total_chars > MAX_EXTRACTED_CHARS {
            anyhow::bail!(
                "project-context text exceeds the 100,000-character limit; remove or shorten a file"
            );
        }
        let unit_count = u32::try_from(sections.len()).unwrap_or(u32::MAX);
        references.push(ContextFileReference {
            display_name: safe_name(&canonical),
            path: canonical.to_string_lossy().into_owned(),
            media_type,
            size_bytes: metadata.len(),
            content_hash: blake3::hash(&bytes).to_hex().to_string(),
            unit_count,
        });
    }
    Ok(references)
}

pub fn extract_references(references: &[ContextFileReference]) -> anyhow::Result<ExtractedContext> {
    if references.len() > MAX_CONTEXT_FILES {
        anyhow::bail!("workspace has more than {MAX_CONTEXT_FILES} project-context files");
    }
    let mut total_bytes = 0_u64;
    let mut total_chars = 0_usize;
    let mut sections = Vec::new();
    let mut sources = Vec::new();
    for (file_index, reference) in references.iter().enumerate() {
        let path = Path::new(&reference.path);
        if !path.is_absolute() {
            anyhow::bail!(
                "{} must be reattached before generating goals",
                reference.display_name
            );
        }
        let canonical = fs::canonicalize(path).map_err(|_| {
            anyhow::anyhow!(
                "{} is missing; reattach it before generating goals",
                reference.display_name
            )
        })?;
        let current_metadata = fs::symlink_metadata(path)?;
        if current_metadata.file_type().is_symlink() || canonical != path {
            anyhow::bail!(
                "{} changed after it was attached; reattach it before generating goals",
                reference.display_name
            );
        }
        let metadata = fs::metadata(&canonical)?;
        if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
            anyhow::bail!(
                "{} changed or is no longer a supported regular file; reattach it",
                reference.display_name
            );
        }
        total_bytes = total_bytes.saturating_add(metadata.len());
        if total_bytes > MAX_TOTAL_FILE_BYTES {
            anyhow::bail!("project-context files exceed the 100 MiB combined limit");
        }
        let bytes = fs::read(&canonical)?;
        let hash = blake3::hash(&bytes).to_hex().to_string();
        if metadata.len() != reference.size_bytes || hash != reference.content_hash {
            anyhow::bail!(
                "{} changed after it was attached; reattach it before generating goals",
                reference.display_name
            );
        }
        let extracted = extract_bytes(reference.media_type, &bytes, &reference.display_name)?;
        if u32::try_from(extracted.len()).unwrap_or(u32::MAX) != reference.unit_count {
            anyhow::bail!(
                "{} changed after it was attached; reattach it before generating goals",
                reference.display_name
            );
        }
        for unit in extracted {
            total_chars = total_chars.saturating_add(unit.text.chars().count());
            if total_chars > MAX_EXTRACTED_CHARS {
                anyhow::bail!(
                    "project-context text exceeds the 100,000-character limit; remove or shorten a file"
                );
            }
            sections.push(ContextSection {
                source_id: format!("file-{}-{}", file_index + 1, unit.identifier),
                text: unit.text,
            });
        }
        sources.push(ContextSourceMetadata {
            display_name: reference.display_name.clone(),
            media_type: reference.media_type,
            size_bytes: reference.size_bytes,
            content_hash: reference.content_hash.clone(),
            unit_count: reference.unit_count,
        });
    }
    Ok(ExtractedContext { sources, sections })
}

fn safe_name(path: &Path) -> String {
    path.file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("selected file")
        .to_string()
}

fn normalize_text(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn require_text(text: String, display_name: &str) -> anyhow::Result<String> {
    let normalized = normalize_text(&text);
    if normalized
        .chars()
        .filter(|character| character.is_alphanumeric())
        .count()
        < 20
    {
        anyhow::bail!(
            "{display_name} contains no usable text; scanned or image-only documents require searchable text"
        );
    }
    Ok(normalized)
}

fn extract_bytes(
    media_type: ContextMediaType,
    bytes: &[u8],
    display_name: &str,
) -> anyhow::Result<Vec<ExtractedUnit>> {
    match media_type {
        ContextMediaType::Pdf => extract_pdf(bytes, display_name),
        ContextMediaType::Docx => extract_docx(bytes, display_name),
        ContextMediaType::Pptx => extract_pptx(bytes, display_name),
        ContextMediaType::Txt | ContextMediaType::Md => {
            let text = std::str::from_utf8(bytes)
                .map_err(|_| anyhow::anyhow!("{display_name} is not valid UTF-8 text"))?;
            Ok(vec![ExtractedUnit {
                identifier: "section-1".into(),
                text: require_text(text.to_string(), display_name)?,
            }])
        }
    }
}

fn extract_pdf(bytes: &[u8], display_name: &str) -> anyhow::Result<Vec<ExtractedUnit>> {
    let document = lopdf::Document::load_mem(bytes)
        .map_err(|_| anyhow::anyhow!("{display_name} is not a readable PDF"))?;
    if document.is_encrypted() {
        anyhow::bail!("{display_name} is encrypted; attach an unlocked PDF");
    }
    let mut pages = Vec::new();
    let mut extracted_chars = 0_usize;
    for page_number in document.get_pages().keys() {
        let text = document
            .extract_text_with_limit(&[*page_number], 4 * 1024 * 1024)
            .map_err(|_| {
                anyhow::anyhow!("{display_name} contains a PDF page that could not be read safely")
            })?;
        let normalized = normalize_text(&text);
        extracted_chars = extracted_chars.saturating_add(normalized.chars().count());
        if extracted_chars > MAX_EXTRACTED_CHARS {
            anyhow::bail!("{display_name} exceeds the 100,000-character extracted-text limit");
        }
        pages.push(ExtractedUnit {
            identifier: format!("page-{page_number}"),
            text: normalized,
        });
    }
    if pages.is_empty()
        || pages
            .iter()
            .flat_map(|page| page.text.chars())
            .filter(|character| character.is_alphanumeric())
            .count()
            < 20
    {
        anyhow::bail!(
            "{display_name} contains no usable text; scanned or image-only PDFs require searchable text"
        );
    }
    Ok(pages)
}

fn read_zip_entry(
    archive: &mut zip::ZipArchive<Cursor<&[u8]>>,
    name: &str,
    display_name: &str,
) -> anyhow::Result<String> {
    let entry = archive
        .by_name(name)
        .map_err(|_| anyhow::anyhow!("{display_name} is missing required document content"))?;
    if entry.encrypted() {
        anyhow::bail!("{display_name} is encrypted; attach an unlocked document");
    }
    if entry.size() > MAX_ARCHIVE_ENTRY_BYTES {
        anyhow::bail!("{display_name} contains an oversized document part");
    }
    let mut xml = String::new();
    entry
        .take(MAX_ARCHIVE_ENTRY_BYTES + 1)
        .read_to_string(&mut xml)
        .map_err(|_| anyhow::anyhow!("{display_name} contains unreadable document XML"))?;
    Ok(xml)
}

fn xml_text(xml: &str, paragraph_element: &str) -> anyhow::Result<Vec<String>> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut buffer = Vec::new();
    let mut in_paragraph = false;
    let mut paragraph = String::new();
    let mut paragraphs = Vec::new();
    loop {
        match reader.read_event_into(&mut buffer)? {
            Event::Start(start) if start.local_name().as_ref() == paragraph_element => {
                in_paragraph = true;
                paragraph.clear();
            }
            Event::End(end) if end.local_name().as_ref() == paragraph_element => {
                let normalized = normalize_text(&paragraph);
                if !normalized.is_empty() {
                    paragraphs.push(normalized);
                }
                in_paragraph = false;
            }
            Event::Text(text) if in_paragraph => {
                let value = quick_xml::escape::unescape(text.as_ref())?;
                if !paragraph.is_empty() {
                    paragraph.push(' ');
                }
                paragraph.push_str(&value);
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok(paragraphs)
}

fn extract_docx(bytes: &[u8], display_name: &str) -> anyhow::Result<Vec<ExtractedUnit>> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|_| anyhow::anyhow!("{display_name} is not a readable DOCX file"))?;
    let xml = read_zip_entry(&mut archive, "word/document.xml", display_name)?;
    let paragraphs = xml_text(&xml, "p")?;
    require_text(paragraphs.join("\n"), display_name)?;
    Ok(paragraphs
        .into_iter()
        .enumerate()
        .map(|(index, text)| ExtractedUnit {
            identifier: format!("paragraph-{}", index + 1),
            text,
        })
        .collect())
}

fn extract_pptx(bytes: &[u8], display_name: &str) -> anyhow::Result<Vec<ExtractedUnit>> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|_| anyhow::anyhow!("{display_name} is not a readable PPTX file"))?;
    let mut slide_names = (0..archive.len())
        .filter_map(|index| archive.name_for_index(index).map(str::to_string))
        .filter(|name| name.starts_with("ppt/slides/slide") && name.ends_with(".xml"))
        .collect::<Vec<_>>();
    slide_names.sort_by_key(|name| {
        name.trim_start_matches("ppt/slides/slide")
            .trim_end_matches(".xml")
            .parse::<u32>()
            .unwrap_or(u32::MAX)
    });
    let mut slides = Vec::new();
    let mut extracted_chars = 0_usize;
    for name in slide_names {
        let slide_number = name
            .trim_start_matches("ppt/slides/slide")
            .trim_end_matches(".xml")
            .parse::<u32>()
            .unwrap_or(u32::MAX);
        let xml = read_zip_entry(&mut archive, &name, display_name)?;
        let paragraphs = xml_text(&xml, "p")?;
        let text = normalize_text(&paragraphs.join("\n"));
        extracted_chars = extracted_chars.saturating_add(text.chars().count());
        if extracted_chars > MAX_EXTRACTED_CHARS {
            anyhow::bail!("{display_name} exceeds the 100,000-character extracted-text limit");
        }
        slides.push(ExtractedUnit {
            identifier: format!("slide-{slide_number}"),
            text,
        });
    }
    if slides.is_empty()
        || slides
            .iter()
            .flat_map(|slide| slide.text.chars())
            .filter(|character| character.is_alphanumeric())
            .count()
            < 20
    {
        anyhow::bail!("{display_name} contains no usable slide text");
    }
    Ok(slides)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{
        Document, EncryptionState, EncryptionVersion, Object, Permissions, Stream,
        content::{Content, Operation},
        dictionary,
    };
    use std::io::Write;

    fn pdf_with_pages(texts: &[&str]) -> Vec<u8> {
        let mut document = Document::with_version("1.5");
        let pages_id = document.new_object_id();
        let font_id = document.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Courier",
        });
        let resources_id = document.add_object(dictionary! {
            "Font" => dictionary! { "F1" => font_id },
        });
        let pages = texts
            .iter()
            .map(|text| {
                let content = Content {
                    operations: vec![
                        Operation::new("BT", vec![]),
                        Operation::new("Tf", vec!["F1".into(), 18.into()]),
                        Operation::new("Td", vec![40.into(), 700.into()]),
                        Operation::new("Tj", vec![Object::string_literal(*text)]),
                        Operation::new("ET", vec![]),
                    ],
                };
                let content_id =
                    document.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
                document
                    .add_object(dictionary! {
                        "Type" => "Page",
                        "Parent" => pages_id,
                        "Contents" => content_id,
                    })
                    .into()
            })
            .collect::<Vec<Object>>();
        let page_count = pages.len() as i64;
        document.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => pages,
                "Count" => page_count,
                "Resources" => resources_id,
                "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
            }),
        );
        let catalog_id =
            document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        document.trailer.set("Root", catalog_id);
        document.trailer.set(
            "ID",
            Object::Array(vec![
                Object::string_literal(b"ABC"),
                Object::string_literal(b"DEF"),
            ]),
        );
        let mut bytes = Vec::new();
        document.save_to(&mut bytes).unwrap();
        bytes
    }

    #[test]
    fn text_file_round_trips_without_persisting_contents_in_reference() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("brief.md");
        fs::write(
            &path,
            "# Leave management\nEnterprise customers need reliable workflows.",
        )
        .unwrap();
        let references = inspect_paths(&[path.to_string_lossy().into_owned()]).unwrap();
        assert_eq!(references[0].display_name, "brief.md");
        let serialized = serde_json::to_string(&references).unwrap();
        assert!(!serialized.contains("Enterprise customers"));
        let extracted = extract_references(&references).unwrap();
        assert!(extracted.prompt_text().contains("Enterprise customers"));
    }

    #[test]
    fn stale_file_requires_reattachment() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("brief.txt");
        fs::write(
            &path,
            "A meaningful original product strategy for customers.",
        )
        .unwrap();
        let references = inspect_paths(&[path.to_string_lossy().into_owned()]).unwrap();
        fs::write(
            &path,
            "A different meaningful product strategy for customers.",
        )
        .unwrap();
        let error = extract_references(&references).unwrap_err().to_string();
        assert!(error.contains("reattach"));
    }

    #[test]
    fn docx_text_is_extracted_by_paragraph() {
        let mut bytes = Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut bytes);
            writer
                .start_file(
                    "word/document.xml",
                    zip::write::SimpleFileOptions::default(),
                )
                .unwrap();
            writer.write_all(br#"<w:document xmlns:w="w"><w:body><w:p><w:r><w:t>Enterprise leave management</w:t></w:r></w:p><w:p><w:r><w:t>Customers need compliance workflows</w:t></w:r></w:p></w:body></w:document>"#).unwrap();
            writer.finish().unwrap();
        }
        let sections = extract_docx(bytes.get_ref(), "brief.docx").unwrap();
        assert_eq!(sections[0].identifier, "paragraph-1");
        assert_eq!(sections[1].identifier, "paragraph-2");
        assert!(sections[0].text.contains("Enterprise leave management"));
        assert!(
            sections[1]
                .text
                .contains("Customers need compliance workflows")
        );
    }

    #[test]
    fn pptx_slides_are_sorted_numerically_and_keep_slide_ids() {
        let mut bytes = Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut bytes);
            for (name, text) in [
                (
                    "ppt/slides/slide10.xml",
                    "Tenth slide strategy and outcomes",
                ),
                ("ppt/slides/slide2.xml", "Second slide customer workflow"),
                ("ppt/slides/slide1.xml", "First slide leave management"),
            ] {
                writer
                    .start_file(name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                write!(writer, "<p:sld xmlns:p=\"p\" xmlns:a=\"a\"><a:p><a:r><a:t>{text}</a:t></a:r></a:p></p:sld>").unwrap();
            }
            writer.finish().unwrap();
        }
        let slides = extract_pptx(bytes.get_ref(), "board.pptx").unwrap();
        assert_eq!(
            slides
                .iter()
                .map(|slide| slide.identifier.as_str())
                .collect::<Vec<_>>(),
            vec!["slide-1", "slide-2", "slide-10"]
        );
        assert!(slides[0].text.contains("leave management"));
    }

    #[test]
    fn pdf_pages_keep_original_page_numbers_and_unicode_text() {
        let bytes = pdf_with_pages(&[
            "ExampleLeave synthetic leave workflows for global teams",
            "Managers approve time away with confidence",
        ]);
        let pages = extract_pdf(&bytes, "board.pdf").unwrap();
        assert_eq!(pages[0].identifier, "page-1");
        assert_eq!(pages[1].identifier, "page-2");
        assert!(pages[1].text.contains("Managers approve"));
    }

    #[test]
    fn corrupt_and_image_only_documents_fail_with_actionable_messages() {
        let corrupt = extract_pdf(b"not a PDF", "broken.pdf")
            .unwrap_err()
            .to_string();
        assert!(corrupt.contains("not a readable PDF"));

        let image_only = pdf_with_pages(&[""]);
        let error = extract_pdf(&image_only, "scan.pdf")
            .unwrap_err()
            .to_string();
        assert!(error.contains("image-only PDFs"));
    }

    #[test]
    fn encrypted_pdf_requires_an_unlocked_copy() {
        let source =
            pdf_with_pages(&["Fictional leave-management strategy for example enterprises"]);
        let mut document = Document::load_mem(&source).unwrap();
        let version = EncryptionVersion::V1 {
            document: &document,
            owner_password: "owner",
            user_password: "user",
            permissions: Permissions::all(),
        };
        let state = EncryptionState::try_from(version).unwrap();
        document.encrypt(&state).unwrap();
        let mut encrypted = Vec::new();
        document.save_to(&mut encrypted).unwrap();
        let error = extract_pdf(&encrypted, "locked.pdf")
            .unwrap_err()
            .to_string();
        assert!(error.contains("encrypted"));
    }

    #[test]
    fn unsupported_extensions_and_extracted_text_limits_fail_closed() {
        let directory = tempfile::tempdir().unwrap();
        let unsupported = directory.path().join("brief.rtf");
        fs::write(
            &unsupported,
            "A sufficiently long product brief for validation.",
        )
        .unwrap();
        let error = inspect_paths(&[unsupported.to_string_lossy().into_owned()])
            .unwrap_err()
            .to_string();
        assert!(error.contains("unsupported"));

        let oversized_text = directory.path().join("oversized.md");
        fs::write(&oversized_text, "a".repeat(MAX_EXTRACTED_CHARS + 1)).unwrap();
        let error = inspect_paths(&[oversized_text.to_string_lossy().into_owned()])
            .unwrap_err()
            .to_string();
        assert!(error.contains("100,000-character"));
    }

    #[cfg(unix)]
    #[test]
    fn symbolic_links_are_rejected_even_when_the_target_is_regular() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("brief.md");
        let link = directory.path().join("linked.md");
        fs::write(
            &target,
            "A meaningful leave-management strategy for enterprise customers.",
        )
        .unwrap();
        symlink(&target, &link).unwrap();
        let error = inspect_paths(&[link.to_string_lossy().into_owned()])
            .unwrap_err()
            .to_string();
        assert!(error.contains("symbolic link"));
    }
}
