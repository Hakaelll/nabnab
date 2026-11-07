use crate::{library::LibraryItem, queue, workspaces::Workspace};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};
use tauri::AppHandle;
use tokio::sync::watch;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum Input {
    Text { title: String, text: String },
    Library { id: String },
    Note { id: String, annotation_id: String },
}
pub struct Resolved {
    pub title: String,
    pub text: String,
    pub source_version: String,
    pub library_id: Option<String>,
    pub kind: crate::voice::OriginKind,
}
fn selected<'a>(library: &'a [LibraryItem], id: &str) -> Result<&'a LibraryItem, String> {
    library.iter().find(|item| item.id == id).ok_or_else(|| "Choose an existing library source for narration.".into())
}
fn version(kind: &str, identity: &str, text: &str) -> String {
    let mut digest = Sha256::new();
    for value in [kind, identity, text] { digest.update((value.len() as u64).to_le_bytes()); digest.update(value.as_bytes()); }
    format!("{:x}", digest.finalize())
}
fn bounded(text: String) -> Result<String, String> {
    if text.trim().is_empty() || text.len() > 4 * 1024 * 1024 || text.chars().count() > 1_000_000 {
        return Err("Choose nonempty text of up to one million characters for narration.".into());
    }
    Ok(text)
}
pub async fn resolve(app: &AppHandle, root: &Path, library: &[LibraryItem], input: Input, cancel: watch::Receiver<bool>, request_id: u64) -> Result<Resolved, String> {
    if *cancel.borrow() { return Err(crate::engine::CANCELLED.into()); }
    match input {
        Input::Text { title, text } => {
            let title = title.trim();
            if title.is_empty() || title.chars().count() > 160 || title.chars().any(char::is_control) { return Err("Give this narration a title of 1–160 characters.".into()); }
            let text = bounded(text)?;
            Ok(Resolved { title: title.to_owned(), source_version: version("text", title, &text), text, library_id: None, kind: crate::voice::OriginKind::Text })
        }
        Input::Note { id, annotation_id } => {
            let item = selected(library, &id)?;
            let reference = item.details.web_archive_ref.as_deref().ok_or("Choose a note from a saved article.")?;
            let article=crate::web_archive::load(root, reference)?;
            let note = item.details.article_annotations.iter().find(|note| note.id == annotation_id && note.version == reference).ok_or("This note does not belong to the selected immutable article version.")?;
            let text = bounded(note.note.clone())?;
            Ok(Resolved { title: format!("{} · nota", article.title), source_version: version("note", &format!("{reference}:{annotation_id}"), &text), text, library_id: Some(id), kind: crate::voice::OriginKind::Note })
        }
        Input::Library { id } => {
            let item = selected(library, &id)?;
            let (text, identity) = if let Some(reference) = &item.details.web_archive_ref {
                let article = crate::web_archive::load(root, reference)?;
                (article.blocks.iter().map(|block| block.text()).filter(|text| !text.trim().is_empty()).collect::<Vec<_>>().join("\n\n"), reference.clone())
            } else if let Some(reference) = &item.details.ocr_ref {
                let index = crate::ocr::read_index(root, reference)?;
                (crate::ocr::corrected_text(&index), format!("{reference}:{}", index.source_fingerprint))
            } else {
                let path = crate::library::authorized_path(&item.path)?;
                let before = queue::digest(path.clone(), cancel.clone()).await?;
                let extension = path.extension().and_then(|extension| extension.to_str()).unwrap_or_default().to_ascii_lowercase();
                let text = match extension.as_str() {
                    "txt" | "md" => {
                        if fs::metadata(&path).map_err(|e| e.to_string())?.len() > 4 * 1024 * 1024 { return Err("This text file exceeds the 4 MiB narration limit.".into()); }
                        fs::read_to_string(&path).map_err(|_| "Choose a UTF-8 text file for local narration.")?.trim_start_matches('\u{feff}').to_owned()
                    }
                    "pdf" | "epub" => {
                        let workspace = Workspace::new(root)?;
                        crate::documents::narration_text(&crate::components::executable(app, "documents")?, &path, &workspace.path, cancel.clone(), request_id).await?
                    }
                    _ => return Err("Choose an archived article, note, OCR document, PDF, EPUB, TXT or Markdown source.".into()),
                };
                if queue::digest(path, cancel.clone()).await? != before { return Err("The source changed during text preparation. Review it again.".into()); }
                (text, before)
            };
            let text = bounded(text)?;
            let title=if let Some(reference)=&item.details.web_archive_ref{crate::web_archive::load(root,reference)?.title}else{Path::new(&item.name).file_stem().and_then(|stem|stem.to_str()).unwrap_or(&item.name).to_owned()};
            Ok(Resolved { title, source_version: version("library", &format!("{id}:{identity}"), &text), text, library_id: Some(id), kind: crate::voice::OriginKind::Library })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn version_changes_with_content_and_source_identity_without_ambiguous_concatenation() {
        assert_ne!(version("note", "ab", "c"), version("note", "a", "bc"));
        assert_ne!(version("note", "first", "draft"), version("note", "first", "edited"));
        assert_ne!(version("note", "first", "draft"), version("note", "second", "draft"));
        assert!(bounded("   ".into()).is_err());
        assert_eq!(bounded("Texto local".into()).unwrap(), "Texto local");
    }
}
