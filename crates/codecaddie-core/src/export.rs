use crate::local_state::HeatmapWeek;
use std::{fs, path::Path};

/// Writes an Office Open XML Word report containing only goals, derived
/// findings, dates, and immutable repository coordinates. Repository source
/// text is never included.
pub fn write_goal_report(
    workspace_name: &str,
    analyses: &[HeatmapWeek],
    destination: &Path,
) -> anyhow::Result<()> {
    if destination.extension().and_then(|value| value.to_str()) != Some("docx") {
        anyhow::bail!("Word reports must use the .docx extension");
    }
    let document = goal_report_document(workspace_name, analyses);
    let entries = [
        (
            "[Content_Types].xml",
            br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/></Types>"#.as_slice(),
        ),
        (
            "_rels/.rels",
            br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.as_slice(),
        ),
        ("word/document.xml", document.as_bytes()),
        (
            "word/_rels/document.xml.rels",
            br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#.as_slice(),
        ),
        (
            "word/styles.xml",
            br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:rPr><w:rFonts w:ascii="IBM Plex Sans" w:hAnsi="IBM Plex Sans" w:cs="Calibri"/><w:color w:val="161B18"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/><w:pPr><w:keepNext/><w:outlineLvl w:val="0"/></w:pPr><w:rPr><w:rFonts w:ascii="IBM Plex Sans" w:hAnsi="IBM Plex Sans" w:cs="Calibri"/><w:b/><w:color w:val="161B18"/><w:sz w:val="32"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/><w:pPr><w:keepNext/><w:outlineLvl w:val="1"/></w:pPr><w:rPr><w:rFonts w:ascii="IBM Plex Sans" w:hAnsi="IBM Plex Sans" w:cs="Calibri"/><w:b/><w:color w:val="161B18"/><w:sz w:val="26"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Heading3"><w:name w:val="heading 3"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/><w:pPr><w:keepNext/><w:outlineLvl w:val="2"/></w:pPr><w:rPr><w:rFonts w:ascii="IBM Plex Sans" w:hAnsi="IBM Plex Sans" w:cs="Calibri"/><w:b/><w:color w:val="161B18"/></w:rPr></w:style><w:style w:type="character" w:styleId="Mono"><w:name w:val="Mono"/><w:rPr><w:rFonts w:ascii="IBM Plex Mono" w:hAnsi="IBM Plex Mono" w:cs="Consolas"/></w:rPr></w:style></w:styles>"#.as_slice(),
        ),
    ];
    // On case-insensitive filesystems a pre-existing file with different
    // case would keep its old name; remove it first so the file on disk
    // matches the name the app announces, exactly.
    match fs::remove_file(destination) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    fs::write(destination, stored_zip(&entries))?;
    Ok(())
}

fn goal_report_document(workspace_name: &str, analyses: &[HeatmapWeek]) -> String {
    let mut body = String::new();
    // Eyebrow: small all-caps mono line above the title.
    body.push_str(
        "<w:p><w:r><w:rPr><w:rFonts w:ascii=\"IBM Plex Mono\" w:hAnsi=\"IBM Plex Mono\" w:cs=\"Consolas\"/><w:color w:val=\"55605A\"/><w:spacing w:val=\"20\"/><w:sz w:val=\"18\"/></w:rPr><w:t xml:space=\"preserve\">CODECADDIE \u{b7} GOAL ANALYSIS</w:t></w:r></w:p>",
    );
    // Title: Heading1 with the single green rule underneath — the one green
    // moment in an otherwise ink-only document.
    body.push_str(
        "<w:p><w:pPr><w:pStyle w:val=\"Heading1\"/><w:pBdr><w:bottom w:val=\"single\" w:sz=\"8\" w:space=\"4\" w:color=\"0B5E4A\"/></w:pBdr></w:pPr><w:r><w:t xml:space=\"preserve\">CodeCaddie goal analysis</w:t></w:r></w:p>",
    );
    word_heading(&mut body, workspace_name, 2);
    word_paragraph(
        &mut body,
        "Progress over time uses five qualitative levels: Missing, Broken, Incomplete, Functional, and Strong. N/A means the goal did not exist when that analysis ran.",
        false,
    );
    if analyses.is_empty() {
        word_paragraph(&mut body, "No completed analyses are available yet.", false);
    } else {
        body.push_str("<w:tbl><w:tblPr><w:tblCaption w:val=\"Goal progress by analysis run\"/><w:tblDescription w:val=\"Rows are goals. Columns are saved analysis runs. Cells use Missing, Broken, Incomplete, Functional, Strong, or N/A.\"/><w:tblBorders><w:top w:val=\"single\" w:sz=\"4\"/><w:left w:val=\"single\" w:sz=\"4\"/><w:bottom w:val=\"single\" w:sz=\"4\"/><w:right w:val=\"single\" w:sz=\"4\"/><w:insideH w:val=\"single\" w:sz=\"4\"/><w:insideV w:val=\"single\" w:sz=\"4\"/></w:tblBorders></w:tblPr>");
        body.push_str("<w:tr><w:trPr><w:tblHeader/></w:trPr>");
        word_header_cell(&mut body, "Goal");
        for analysis in analyses {
            word_header_cell(&mut body, &analysis.label);
        }
        body.push_str("</w:tr>");
        if let Some(latest) = analyses.last() {
            for latest_cell in &latest.cells {
                body.push_str("<w:tr>");
                word_cell(&mut body, &latest_cell.goal_title);
                for analysis in analyses {
                    let status = analysis
                        .cells
                        .iter()
                        .find(|cell| cell.goal_id == latest_cell.goal_id)
                        .map(|cell| report_category_label(&cell.verdict))
                        .unwrap_or("N/A");
                    word_cell(&mut body, status);
                }
                body.push_str("</w:tr>");
            }
        }
        body.push_str("</w:tbl>");

        word_heading(&mut body, "Analysis provenance", 1);
        for analysis in analyses {
            word_heading(&mut body, &analysis.label, 2);
            word_mono_paragraph(&mut body, &format!("Analysis ID: {}", analysis.report_id));
            word_paragraph(
                &mut body,
                &format!("Completed: {}", analysis.week_start),
                false,
            );
            word_paragraph(
                &mut body,
                &format!(
                    "Provider: {} {}",
                    analysis.provider, analysis.provider_version
                ),
                false,
            );
            if analysis.repositories.is_empty() {
                word_paragraph(&mut body, "Repository commit: not recorded", false);
            } else {
                for repository in &analysis.repositories {
                    word_mono_paragraph(&mut body, &format!("Repository commit: {repository}"));
                }
            }
            word_paragraph(
                &mut body,
                &format!("Unverified criteria: {}", analysis.unverified_criteria),
                false,
            );
            if let Some(coverage) = analysis.coverage {
                word_paragraph(
                    &mut body,
                    &format!(
                        "Weighted coverage: {:.0}% of assessed criteria, weighted by goal priority",
                        coverage * 100.0
                    ),
                    false,
                );
            }
            word_paragraph(
                &mut body,
                if analysis.partial {
                    "Completion: completed with gaps"
                } else {
                    "Completion: complete"
                },
                false,
            );
        }

        if analyses
            .iter()
            .any(|analysis| !analysis.architecture.is_empty())
        {
            word_heading(&mut body, "Architecture findings", 1);
            for analysis in analyses {
                if analysis.architecture.is_empty() {
                    continue;
                }
                word_heading(&mut body, &format!("Analysis from {}", analysis.label), 2);
                for claim in &analysis.architecture {
                    word_heading(&mut body, &claim.component, 3);
                    word_paragraph(&mut body, &claim.summary, false);
                    if let Some(relationship) = &claim.relationship {
                        word_paragraph(&mut body, relationship, false);
                    }
                    let affected = claim
                        .affected_goal_version_ids
                        .iter()
                        .filter_map(|version_id| {
                            analysis
                                .cells
                                .iter()
                                .find(|cell| cell.goal_version_id == *version_id)
                                .map(|cell| cell.goal_title.as_str())
                        })
                        .collect::<Vec<_>>();
                    if !affected.is_empty() {
                        word_paragraph(
                            &mut body,
                            &format!("Supports goals: {}", affected.join(", ")),
                            false,
                        );
                    }
                    for evidence in &claim.evidence {
                        word_mono_paragraph(
                            &mut body,
                            &format!(
                                "Repository reference: {}:{}-{} @ {}",
                                evidence.path,
                                evidence.start_line,
                                evidence.end_line,
                                evidence
                                    .commit_sha
                                    .get(..12)
                                    .unwrap_or(&evidence.commit_sha)
                            ),
                        );
                    }
                }
            }
        }

        word_heading(&mut body, "Finding details", 1);
        for analysis in analyses {
            word_heading(&mut body, &format!("Analysis from {}", analysis.label), 2);
            for cell in &analysis.cells {
                word_heading(
                    &mut body,
                    &format!(
                        "{} — {}",
                        cell.goal_title,
                        report_category_label(&cell.verdict)
                    ),
                    3,
                );
                word_paragraph(&mut body, &cell.summary, false);
                if !cell.architecture_narrative.is_empty() {
                    word_paragraph(
                        &mut body,
                        &format!("Architecture support: {}", cell.architecture_narrative),
                        false,
                    );
                }
                word_paragraph(&mut body, &format!("Change: {}", cell.change), false);
                for criterion in &cell.criteria {
                    word_paragraph(
                        &mut body,
                        &format!(
                            "{}: {}",
                            criterion_result_label(
                                &criterion.verdict,
                                !criterion.evidence.is_empty()
                            ),
                            criterion.text
                        ),
                        true,
                    );
                    word_paragraph(&mut body, &criterion.rationale, false);
                    if criterion.evidence.is_empty() {
                        word_paragraph(
                            &mut body,
                            "No verified repository reference was found for this result.",
                            false,
                        );
                    } else {
                        for evidence in &criterion.evidence {
                            word_mono_paragraph(
                                &mut body,
                                &format!(
                                    "Repository reference: {}:{}-{} @ {}",
                                    evidence.path,
                                    evidence.start_line,
                                    evidence.end_line,
                                    evidence
                                        .commit_sha
                                        .get(..12)
                                        .unwrap_or(&evidence.commit_sha)
                                ),
                            );
                        }
                    }
                }
            }
        }
    }
    // Muted provenance line at the end of the document (the package has no
    // footer part, and adding one is not worth the extra OPC plumbing).
    body.push_str(
        "<w:p><w:r><w:rPr><w:color w:val=\"55605A\"/><w:sz w:val=\"16\"/></w:rPr><w:t xml:space=\"preserve\">generated locally \u{b7} CodeCaddie</w:t></w:r></w:p>",
    );
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>{body}<w:sectPr><w:pgSz w:w=\"12240\" w:h=\"15840\"/><w:pgMar w:top=\"1440\" w:right=\"1440\" w:bottom=\"1440\" w:left=\"1440\"/></w:sectPr></w:body></w:document>"
    )
}

fn report_category_label(value: &str) -> &str {
    match value {
        "missing" => "Missing",
        "broken" => "Broken",
        "incomplete" => "Incomplete",
        "functional" => "Functional",
        "strong" => "Strong",
        _ => "N/A",
    }
}

fn criterion_result_label(verdict: &str, has_evidence: bool) -> &'static str {
    match verdict {
        "supported" => "Found",
        "partial" => "Partly found",
        "unsupported" if has_evidence => "Evidence shows a gap",
        "unsupported" => "Could not find evidence",
        _ => "Could not verify",
    }
}

fn word_paragraph(output: &mut String, value: &str, bold: bool) {
    output.push_str("<w:p><w:r>");
    if bold {
        output.push_str("<w:rPr><w:b/></w:rPr>");
    }
    output.push_str("<w:t xml:space=\"preserve\">");
    output.push_str(&xml_escape(value));
    output.push_str("</w:t></w:r></w:p>");
}

/// Writes a paragraph whose run uses the `Mono` character style, for commit
/// hashes and file:line coordinates.
fn word_mono_paragraph(output: &mut String, value: &str) {
    output.push_str(
        "<w:p><w:r><w:rPr><w:rStyle w:val=\"Mono\"/></w:rPr><w:t xml:space=\"preserve\">",
    );
    output.push_str(&xml_escape(value));
    output.push_str("</w:t></w:r></w:p>");
}

fn word_heading(output: &mut String, value: &str, level: u8) {
    let style = match level {
        1 => "Heading1",
        2 => "Heading2",
        _ => "Heading3",
    };
    output.push_str("<w:p><w:pPr><w:pStyle w:val=\"");
    output.push_str(style);
    output.push_str("\"/></w:pPr><w:r><w:t xml:space=\"preserve\">");
    output.push_str(&xml_escape(value));
    output.push_str("</w:t></w:r></w:p>");
}

fn word_cell(output: &mut String, value: &str) {
    output.push_str("<w:tc><w:tcPr/><w:p><w:r><w:t xml:space=\"preserve\">");
    output.push_str(&xml_escape(value));
    output.push_str("</w:t></w:r></w:p></w:tc>");
}

fn word_header_cell(output: &mut String, value: &str) {
    output.push_str("<w:tc><w:tcPr><w:shd w:fill=\"F2F4F1\"/></w:tcPr><w:p><w:r><w:rPr><w:b/></w:rPr><w:t xml:space=\"preserve\">");
    output.push_str(&xml_escape(value));
    output.push_str("</w:t></w:r></w:p></w:tc>");
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn stored_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut output = Vec::new();
    let mut central = Vec::new();
    for (name, bytes) in entries {
        let offset = output.len() as u32;
        let checksum = crc32(bytes);
        write_u32(&mut output, 0x0403_4b50);
        write_u16(&mut output, 20);
        write_u16(&mut output, 0);
        write_u16(&mut output, 0);
        write_u16(&mut output, 0);
        write_u16(&mut output, 0);
        write_u32(&mut output, checksum);
        write_u32(&mut output, bytes.len() as u32);
        write_u32(&mut output, bytes.len() as u32);
        write_u16(&mut output, name.len() as u16);
        write_u16(&mut output, 0);
        output.extend_from_slice(name.as_bytes());
        output.extend_from_slice(bytes);

        write_u32(&mut central, 0x0201_4b50);
        write_u16(&mut central, 20);
        write_u16(&mut central, 20);
        write_u16(&mut central, 0);
        write_u16(&mut central, 0);
        write_u16(&mut central, 0);
        write_u16(&mut central, 0);
        write_u32(&mut central, checksum);
        write_u32(&mut central, bytes.len() as u32);
        write_u32(&mut central, bytes.len() as u32);
        write_u16(&mut central, name.len() as u16);
        write_u16(&mut central, 0);
        write_u16(&mut central, 0);
        write_u16(&mut central, 0);
        write_u16(&mut central, 0);
        write_u32(&mut central, 0);
        write_u32(&mut central, offset);
        central.extend_from_slice(name.as_bytes());
    }
    let central_offset = output.len() as u32;
    let central_size = central.len() as u32;
    output.extend_from_slice(&central);
    write_u32(&mut output, 0x0605_4b50);
    write_u16(&mut output, 0);
    write_u16(&mut output, 0);
    write_u16(&mut output, entries.len() as u16);
    write_u16(&mut output, entries.len() as u16);
    write_u32(&mut output, central_size);
    write_u32(&mut output, central_offset);
    write_u16(&mut output, 0);
    output
}

fn write_u16(output: &mut Vec<u8>, value: u16) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn write_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff_u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (0_u32.wrapping_sub(crc & 1)));
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;
    use codecaddie_domain::{EvidenceKind, EvidenceRef};

    #[test]
    fn privacy_adversarial_word_reports_are_metadata_only_docx_packages() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("analysis.docx");
        fs::write(
            directory.path().join("adversarial_repository.rs"),
            crate::privacy_test_support::REPOSITORY_FIXTURE,
        )
        .unwrap();
        let analyses = vec![HeatmapWeek {
            week_start: "2026-08-09T00:00:00Z".into(),
            label: "Aug 9".into(),
            report_id: "report-1".into(),
            report_event_id: "event-1".into(),
            run_number: 1,
            origin: codecaddie_domain::ReportOrigin::Scan,
            provider: "codex".into(),
            provider_version: "0.146.1".into(),
            repositories: vec!["repo-1 @ 0123456789abcdef".into()],
            unverified_criteria: 0,
            partial: false,
            analysis_warnings: vec![],
            coverage: Some(0.87),
            architecture: vec![codecaddie_domain::ArchitectureClaim {
                id: "claim-1".into(),
                component: "Local analysis boundary".into(),
                relationship: Some("Keeps repository source on the device".into()),
                summary: "Repository reads stay behind the local process boundary.".into(),
                affected_goal_version_ids: vec!["goal-version-1".into()],
                component_id: None,
                evidence: vec![EvidenceRef {
                    repository_id: "repo-1".into(),
                    commit_sha: "0123456789abcdef".into(),
                    blob_oid: "blob".into(),
                    path: "adversarial_repository.rs".into(),
                    start_line: 5,
                    end_line: 9,
                    content_hash: "hash".into(),
                    kind: EvidenceKind::Architecture,
                }],
            }],
            cells: vec![crate::local_state::HeatmapCell {
                goal_title: "Keep analysis private".into(),
                goal_id: "privacy".into(),
                goal_version_id: "goal-version-1".into(),
                verdict: "strong".into(),
                summary: "Yes — Repository source stays on the device across every checked path."
                    .into(),
                rationale: "All checked paths keep source on device.".into(),
                architecture_narrative: "The local analysis boundary keeps source on the device."
                    .into(),
                change: "First assessment for this goal".into(),
                checks: vec!["Repository content stays local".into()],
                references: vec!["crates/core.rs:10-20 @ 0123456789ab".into()],
                criteria: vec![crate::local_state::HeatmapCriterion {
                    criterion_id: "criterion-private".into(),
                    text: "Repository content stays local".into(),
                    verdict: "supported".into(),
                    change_kind: "first".into(),
                    change: "First saved assessment".into(),
                    previous_verdict: None,
                    previous_evidence: vec![],
                    rationale: "The local repository boundary is enforced.".into(),
                    confidence: 0.9,
                    evidence: vec![EvidenceRef {
                        repository_id: "repo-1".into(),
                        commit_sha: "0123456789abcdef".into(),
                        blob_oid: "blob".into(),
                        path: "crates/core.rs".into(),
                        start_line: 10,
                        end_line: 20,
                        content_hash: "hash".into(),
                        kind: EvidenceKind::Implementation,
                    }],
                }],
            }],
        }];
        write_goal_report("CodeCaddie", &analyses, &destination).unwrap();
        let bytes = fs::read(destination).unwrap();
        assert!(bytes.starts_with(b"PK\x03\x04"));
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("word/document.xml"));
        assert!(text.contains("word/styles.xml"));
        assert!(text.contains("w:tblHeader"));
        assert!(text.contains("w:pStyle w:val=\"Heading1\""));
        assert!(text.contains("Goal progress by analysis run"));
        assert!(text.contains("Provider: codex 0.146.1"));
        assert!(text.contains("Keep analysis private"));
        assert!(text.contains("Found: Repository content stays local"));
        assert!(text.contains("crates/core.rs:10-20 @ 0123456789ab"));
        assert!(text.contains("Weighted coverage: 87%"));
        assert!(text.contains("Architecture findings"));
        assert!(text.contains("Local analysis boundary"));
        assert!(text.contains("Supports goals: Keep analysis private"));
        assert!(text.contains("adversarial_repository.rs:5-9 @ 0123456789ab"));
        assert!(!text.contains("sourceExcerpt"));
        crate::privacy_test_support::assert_private_payload_absent(&bytes);
        assert!(!text.contains(crate::privacy_test_support::INJECTION_TEXT));
        // Green Ink: declared fonts, record-ink text, one green rule under the
        // title, paper-neutral header shading, mono coordinates, eyebrow, and
        // the muted closing line.
        assert!(text.contains("w:rFonts w:ascii=\"IBM Plex Sans\""));
        assert!(text.contains("w:cs=\"Calibri\""));
        assert!(text.contains("w:color w:val=\"161B18\""));
        assert!(text.contains("w:rFonts w:ascii=\"IBM Plex Mono\""));
        assert!(text.contains("w:cs=\"Consolas\""));
        assert!(text.contains(
            "<w:pBdr><w:bottom w:val=\"single\" w:sz=\"8\" w:space=\"4\" w:color=\"0B5E4A\"/></w:pBdr>"
        ));
        assert!(text.contains("w:shd w:fill=\"F2F4F1\""));
        assert!(!text.contains("E8F4EF"));
        assert!(text.contains("CODECADDIE \u{b7} GOAL ANALYSIS"));
        assert!(text.contains("w:rStyle w:val=\"Mono\""));
        assert!(text.contains("generated locally \u{b7} CodeCaddie"));
    }
}
