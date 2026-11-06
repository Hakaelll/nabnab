mod analyze;
mod rename;

pub use analyze::{analyze, Analysis, AnalyzeOptions};
pub use rename::{
    approve_rename, execute_rename, plan_rename, preflight_rename, rename_history, undo_rename,
    RenameJournal, RenameOptions, RenamePlan,
};

use crate::{library::LibraryItem, store::Repository};
use std::{collections::HashSet, path::Path};
use tokio::sync::watch;

fn cancelled(cancel: &watch::Receiver<bool>) -> Result<(), String> {
    if *cancel.borrow() {
        Err(crate::engine::CANCELLED.into())
    } else {
        Ok(())
    }
}
fn path_key(path: &Path) -> String {
    let value = path.to_string_lossy().into_owned();
    if cfg!(windows) {
        value.to_lowercase()
    } else {
        value
    }
}
fn selected<'a>(
    library: &'a [LibraryItem],
    ids: &[String],
) -> Result<Vec<&'a LibraryItem>, String> {
    if ids.is_empty() || ids.len() > 200 {
        return Err("Choose 1–200 library files for this operation.".into());
    }
    let mut seen = HashSet::new();
    ids.iter()
        .filter(|id| seen.insert(id.as_str()))
        .map(|id| {
            library
                .iter()
                .find(|item| &item.id == id)
                .ok_or_else(|| "A selected file is no longer in the library.".into())
        })
        .collect()
}

pub fn organize(repo: &mut Repository, ids: &[String], collection: &str) -> Result<(), String> {
    let collection = collection.trim();
    if collection.is_empty() || collection.len() > 120 || collection.chars().any(char::is_control) {
        return Err("Use a review collection name of 1–120 characters.".into());
    }
    selected(&repo.data.library, ids)?;
    let old_library = repo.data.library.clone();
    let old_collections = repo.data.collections.clone();
    for item in repo
        .data
        .library
        .iter_mut()
        .filter(|item| ids.contains(&item.id))
    {
        if !item
            .collections
            .iter()
            .any(|name| name.eq_ignore_ascii_case(collection))
        {
            item.collections.push(collection.to_owned());
        }
    }
    if !repo
        .data
        .collections
        .iter()
        .any(|name| name.eq_ignore_ascii_case(collection))
    {
        repo.data.collections.push(collection.to_owned());
    }
    if let Err(error) = repo.save() {
        repo.data.library = old_library;
        repo.data.collections = old_collections;
        return Err(error);
    }
    Ok(())
}
