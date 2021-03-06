use std::{
    collections::{HashMap, HashSet},
    ops::Range,
};

use lsp_document::{Pos, TextAdapter};
use lsp_types::{Diagnostic, DiagnosticSeverity, PublishDiagnosticsParams, Url};
use tracing::debug;

use crate::{
    facts::{Facts, FactsDB, NoteFacts, NoteFactsDB, NoteFactsExt},
    parser::{Heading, Node, NoteName},
    store::NoteFile,
};

#[derive(Debug, Default)]
pub struct DiagCollection {
    pub store: HashMap<NoteFile, HashSet<DiagWithLoc>>,
}

pub fn to_publish(
    file: &NoteFile,
    diags: &HashSet<DiagWithLoc>,
    facts: &FactsDB,
) -> Option<PublishDiagnosticsParams> {
    let index = facts.note_index();

    let note = facts.note_facts(index.find_by_path(&file.path)?);
    let text_version = note.text().version.to_lsp_version();
    let indexed_text = note.indexed_text();

    let lsp_diags: Vec<Diagnostic> = diags
        .iter()
        .filter_map(|(d, r)| {
            let range = match indexed_text.range_to_lsp_range(r) {
                Some(r) => r,
                _ => return None,
            };

            Some(Diagnostic {
                range,
                severity: Some(DiagnosticSeverity::ERROR),
                message: d.to_message(),
                ..Diagnostic::default()
            })
        })
        .collect();

    let param = PublishDiagnosticsParams {
        uri: Url::from_file_path(file.path.clone()).unwrap(),
        diagnostics: lsp_diags,
        version: text_version,
    };

    Some(param)
}

pub type DiagWithLoc = (Diag, Range<Pos>);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Diag {
    DupTitle {
        title: Node<Heading>,
    },
    DupHeading {
        heading: Node<Heading>,
    },
    BrokenInternLinkToNote {
        linked_note: NoteName,
    },
    BrokenInternLinkToHeading {
        linked_note: NoteName,
        heading: String,
    },
}

impl Diag {
    pub fn to_message(&self) -> String {
        match self {
            Diag::DupTitle { title } => format!(
                "Duplicate title `{}`. Each note should have at most one title",
                title.text
            ),
            Diag::DupHeading { heading } => format!("Duplicate heading `{}`", heading.text),
            Diag::BrokenInternLinkToNote { linked_note } => {
                format!("Reference to non-existent note `{}`", linked_note)
            }
            Diag::BrokenInternLinkToHeading {
                linked_note,
                heading,
            } => format!(
                "Reference to non-existent heading `{}`{}",
                linked_note, heading
            ),
        }
    }
}

pub fn check_title(note: &impl NoteFactsExt) -> Vec<DiagWithLoc> {
    debug!("check_title: start");

    let hd_ids = note.headings_matching(|hd| hd.level == 1);
    debug!("check_title: found {} title ids", hd_ids.len());

    let strukt = note.structure();
    let duplicates = strukt.headings_with_ids(&hd_ids).into_iter().skip(1);

    let duplicate_diags = duplicates
        .map(|t| (Diag::DupTitle { title: t.clone() }, t.span.clone()))
        .collect::<Vec<_>>();

    debug!("check_title: reporting {}", duplicate_diags.len());
    duplicate_diags
}

pub fn check_headings(note: &impl NoteFactsExt) -> Vec<DiagWithLoc> {
    debug!("check_headings: start");

    let mut hd_ids_to_inspect = note
        .headings_matching(|hd| hd.level > 1)
        .into_iter()
        .collect::<HashSet<_>>();
    debug!(
        "check_headings: found {} heading ids",
        hd_ids_to_inspect.len()
    );

    let strukt = note.structure();
    let mut duplicates = Vec::new();
    while let Some(&cur_id) = hd_ids_to_inspect.iter().next() {
        hd_ids_to_inspect.remove(&cur_id);
        let cur_hd = strukt.heading_by_id(cur_id);

        let similar_text_ids = note
            .headings_matching(|hd| hd.text == cur_hd.text)
            .into_iter()
            .filter(|&id| id != cur_id)
            .collect::<Vec<_>>();

        if !similar_text_ids.is_empty() {
            for id in &similar_text_ids {
                hd_ids_to_inspect.remove(id);
            }

            duplicates.append(&mut strukt.headings_with_ids(&similar_text_ids));
        }
    }

    let duplicate_diags = duplicates
        .into_iter()
        .map(|h| (Diag::DupHeading { heading: h.clone() }, h.span.clone()))
        .collect::<Vec<_>>();

    debug!("check_headings: reporting {}", duplicate_diags.len());
    duplicate_diags
}

pub fn check_intern_links(facts: &dyn Facts, note: &impl NoteFactsExt) -> Vec<DiagWithLoc> {
    let mut diags = Vec::new();

    let strukt = note.structure();
    let intern_link_ids = note.intern_link_ids();
    let intern_links = strukt.intern_links_with_ids(&intern_link_ids);

    for intern_link in intern_links {
        let target_name = intern_link
            .note_name
            .clone()
            .unwrap_or_else(|| (*note.file().name).clone());
        let target_id = facts.note_index(()).find_by_name(&target_name);
        match target_id {
            Some(id) => {
                let target_note = NoteFactsDB::new(facts, id);
                if let Some(heading) = &intern_link.heading {
                    if target_note.heading_with_text(heading).is_none() {
                        diags.push((
                            Diag::BrokenInternLinkToHeading {
                                linked_note: target_name,
                                heading: heading.to_string(),
                            },
                            intern_link.span.clone(),
                        ));
                    }
                }
            }
            _ => {
                diags.push((
                    Diag::BrokenInternLinkToNote {
                        linked_note: target_name,
                    },
                    intern_link.span.clone(),
                ));
            }
        }
    }

    diags
}
