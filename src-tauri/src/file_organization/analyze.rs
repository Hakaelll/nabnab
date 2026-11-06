use super::{cancelled, path_key, selected};
use crate::{
    library::{self, LibraryItem},
    model::Progress,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
};
use tokio::sync::watch;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AnalyzeOptions {
    Exact,
    Similar { max_distance: u8 },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileRecord {
    pub id: String,
    pub record_ids: Vec<String>,
    pub name: String,
    pub path: PathBuf,
    pub size: u64,
    pub collections: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateGroup {
    pub sha256: String,
    pub members: Vec<String>,
    pub bytes_per_file: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimilarPair {
    pub left: String,
    pub right: String,
    pub distance: u32,
    pub bits: u8,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Analysis {
    pub options: AnalyzeOptions,
    pub files: Vec<FileRecord>,
    pub exact_groups: Vec<DuplicateGroup>,
    pub similar_pairs: Vec<SimilarPair>,
    pub warnings: Vec<String>,
}

pub(super) fn fingerprint(path: &Path, cancel: &watch::Receiver<bool>) -> Result<String, String> {
    let before = fs::metadata(path).map_err(|e| e.to_string())?;
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        cancelled(cancel)?;
        let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    let after = fs::metadata(path).map_err(|e| e.to_string())?;
    if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        return Err("A selected file changed during analysis. Run the analysis again.".into());
    }
    Ok(format!("{:x}", hash.finalize()))
}
async fn streamed_hash(path: PathBuf, cancel: watch::Receiver<bool>) -> Result<String, String> {
    tokio::task::spawn_blocking(move || fingerprint(&path, &cancel))
        .await
        .map_err(|e| e.to_string())?
}
fn records(library: &[LibraryItem], ids: &[String]) -> Result<Vec<FileRecord>, String> {
    let mut records = BTreeMap::new();
    for item in selected(library, ids)? {
        let path = library::authorized_path(&item.path)?;
        let metadata = fs::metadata(&path).map_err(|e| e.to_string())?;
        if !metadata.is_file() {
            return Err("Choose regular local files from the library.".into());
        }
        let aliases = library
            .iter()
            .filter(|alias| path_key(&alias.path) == path_key(&path))
            .collect::<Vec<_>>();
        let mut collections = Vec::new();
        for alias in &aliases {
            for collection in &alias.collections {
                if !collections.contains(collection) {
                    collections.push(collection.clone());
                }
            }
        }
        records
            .entry(path_key(&path))
            .or_insert_with(|| FileRecord {
                id: item.id.clone(),
                record_ids: aliases.into_iter().map(|alias| alias.id.clone()).collect(),
                name: item.name.clone(),
                path,
                size: metadata.len(),
                collections,
            });
    }
    Ok(records.into_values().collect())
}
async fn image_hash(
    ffmpeg: &Path,
    staging: &Path,
    file: &FileRecord,
    position: usize,
    cancel: watch::Receiver<bool>,
    run_id: u64,
) -> Result<(u64, f64), String> {
    let probe = ffmpeg.with_file_name(if cfg!(windows) {
        "ffprobe.exe"
    } else {
        "ffprobe"
    });
    let raw = crate::engine::run_process(
        probe,
        vec![
            "-v".into(),
            "error".into(),
            "-protocol_whitelist".into(),
            "file,pipe".into(),
            "-select_streams".into(),
            "v:0".into(),
            "-show_entries".into(),
            "stream=width,height".into(),
            "-of".into(),
            "json".into(),
            file.path.to_string_lossy().into_owned(),
        ],
        cancel.clone(),
        run_id,
        true,
        |_| {},
    )
    .await?;
    let value: serde_json::Value =
        serde_json::from_str(&raw).map_err(|_| "The image dimensions could not be read.")?;
    let width = value["streams"][0]["width"]
        .as_u64()
        .ok_or("The image has no width.")?;
    let height = value["streams"][0]["height"]
        .as_u64()
        .ok_or("The image has no height.")?;
    if width == 0 || height == 0 || width.saturating_mul(height) > 40_000_000 {
        return Err("Image similarity accepts images up to 40 million pixels.".into());
    }
    let output = staging.join(format!("dhash-{position}.gray"));
    let result = crate::engine::run_process(
        ffmpeg.to_owned(),
        vec![
            "-nostdin".into(),
            "-hide_banner".into(),
            "-v".into(),
            "error".into(),
            "-protocol_whitelist".into(),
            "file,pipe".into(),
            "-i".into(),
            file.path.to_string_lossy().into_owned(),
            "-vf".into(),
            "scale=9:8:flags=area,format=gray".into(),
            "-frames:v".into(),
            "1".into(),
            "-threads".into(),
            "2".into(),
            "-f".into(),
            "rawvideo".into(),
            "-n".into(),
            output.to_string_lossy().into_owned(),
        ],
        cancel,
        run_id,
        true,
        |_| {},
    )
    .await;
    if let Err(error) = result {
        let _ = fs::remove_file(&output);
        return Err(error);
    }
    let output = crate::checked_output(staging, &output.to_string_lossy())?;
    let bytes = fs::read(&output).map_err(|e| e.to_string())?;
    fs::remove_file(output).map_err(|e| e.to_string())?;
    if bytes.len() != 72 {
        return Err("The image comparison renderer returned an invalid 9×8 sample.".into());
    }
    let mut hash = 0;
    for row in 0..8 {
        for column in 0..8 {
            hash = (hash << 1) | u64::from(bytes[row * 9 + column] > bytes[row * 9 + column + 1]);
        }
    }
    Ok((hash, width as f64 / height as f64))
}
pub async fn analyze(
    ffmpeg: &Path,
    staging: &Path,
    library: &[LibraryItem],
    ids: &[String],
    options: &AnalyzeOptions,
    cancel: watch::Receiver<bool>,
    run_id: u64,
    emit: impl Fn(Progress) + Send + Sync + Clone + 'static,
) -> Result<Analysis, String> {
    if matches!(options,AnalyzeOptions::Similar{max_distance}if *max_distance>16) {
        return Err("Use a visual difference threshold from 0–16 of 64 bits.".into());
    }
    cancelled(&cancel)?;
    let files = records(library, ids)?;
    let mut result = Analysis {
        options: options.clone(),
        files: files.clone(),
        exact_groups: vec![],
        similar_pairs: vec![],
        warnings: vec![],
    };
    let report = |done: usize, total: usize| {
        emit(Progress {
            request_id: run_id,
            phase: "processing".into(),
            percent: Some(done as f64 / total.max(1) as f64 * 100.0),
            speed: None,
            downloaded: None,
            total: None,
        })
    };
    match options {
        AnalyzeOptions::Exact => {
            let mut sizes: BTreeMap<u64, Vec<&FileRecord>> = BTreeMap::new();
            for file in &files {
                sizes.entry(file.size).or_default().push(file);
            }
            let candidates = sizes
                .into_values()
                .filter(|group| group.len() > 1)
                .flatten()
                .collect::<Vec<_>>();
            let mut hashes: BTreeMap<(u64, String), Vec<String>> = BTreeMap::new();
            for (position, file) in candidates.iter().enumerate() {
                let hash = streamed_hash(file.path.clone(), cancel.clone()).await?;
                hashes
                    .entry((file.size, hash))
                    .or_default()
                    .push(file.id.clone());
                report(position + 1, candidates.len());
            }
            result.exact_groups = hashes
                .into_iter()
                .filter(|(_, ids)| ids.len() > 1)
                .map(|((size, hash), members)| DuplicateGroup {
                    sha256: hash,
                    members,
                    bytes_per_file: size,
                })
                .collect();
        }
        AnalyzeOptions::Similar { max_distance } => {
            let staging = library::authorized_path(staging)?;
            if !staging.is_dir() {
                return Err("Choose an owned image comparison workspace.".into());
            }
            let images = files
                .iter()
                .filter(|file| {
                    library
                        .iter()
                        .any(|item| item.id == file.id && item.kind == "image")
                })
                .collect::<Vec<_>>();
            if images.len() != files.len() {
                result.warnings.push("Only selected image files were compared visually; other file types were kept outside this comparison.".into());
            }
            let mut hashes = Vec::new();
            for (position, file) in images.iter().enumerate() {
                cancelled(&cancel)?;
                let before = streamed_hash(file.path.clone(), cancel.clone()).await?;
                let (hash, aspect) =
                    image_hash(ffmpeg, &staging, file, position, cancel.clone(), run_id).await?;
                if streamed_hash(file.path.clone(), cancel.clone()).await? != before {
                    return Err("An image changed during comparison. Compare it again.".into());
                }
                hashes.push((file.id.clone(), hash, aspect));
                report(position + 1, images.len());
            }
            for left in 0..hashes.len() {
                cancelled(&cancel)?;
                for right in left + 1..hashes.len() {
                    let (a, ah, ar) = &hashes[left];
                    let (b, bh, br) = &hashes[right];
                    if ar.max(*br) / ar.min(*br) > 1.05 {
                        continue;
                    }
                    let distance = (ah ^ bh).count_ones();
                    if distance <= u32::from(*max_distance) {
                        result.similar_pairs.push(SimilarPair {
                            left: a.clone(),
                            right: b.clone(),
                            distance,
                            bits: 64,
                        });
                    }
                }
            }
            result.warnings.push("Visual similarity is a 64-bit dHash estimate, not evidence of identical content. Aspect ratios must be within 5%. Compare both images before organizing them. Animated images use their first frame.".into());
        }
    }
    if files.iter().any(|file| file.record_ids.len() > 1) {
        result.warnings.push("Some paths have multiple library records. Renaming a path updates all its records; adding a review collection only affects the records you select.".into());
    }
    cancelled(&cancel)?;
    report(1, 1);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn same_size_needs_equal_content_and_shared_records_are_not_duplicate_files() {
        let root = std::env::temp_dir().join(format!(
            "nabnab-file-analysis-{}-{}",
            std::process::id(),
            crate::queue::now()
        ));
        fs::create_dir_all(&root).unwrap();
        let mut items = Vec::new();
        for (name, bytes) in [("a.txt", b"same"), ("b.txt", b"same"), ("c.txt", b"else")] {
            let path = root.join(name);
            fs::write(&path, bytes).unwrap();
            items.push(library::index(&path, None, None, None).unwrap());
        }
        let mut alias = items[0].clone();
        alias.id = "alias".into();
        items.push(alias);
        let ids = items.iter().map(|i| i.id.clone()).collect::<Vec<_>>();
        let (_sender, cancel) = watch::channel(false);
        let result = analyze(
            Path::new("unused"),
            &root,
            &items,
            &ids,
            &AnalyzeOptions::Exact,
            cancel,
            1,
            |_| {},
        )
        .await
        .unwrap();
        assert_eq!(result.files.len(), 3);
        assert_eq!(result.exact_groups.len(), 1);
        assert_eq!(result.exact_groups[0].members.len(), 2);
        assert!(result.files.iter().any(|f| f.record_ids.len() == 2));
        assert_eq!(fs::read(root.join("a.txt")).unwrap(), b"same");
        fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    #[ignore = "Requires the verified FFmpeg and FFprobe executables"]
    async fn real_image_similarity_matches_a_resized_copy_without_changing_originals() {
        let Some(ffmpeg) = std::env::var_os("NABNAB_TEST_FFMPEG").map(PathBuf::from) else {
            return;
        };
        let root = std::env::temp_dir().join(format!(
            "nabnab-dhash-{}-{}",
            std::process::id(),
            crate::queue::now()
        ));
        fs::create_dir_all(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let a = root.join("original.png");
        let b = root.join("resized.jpg");
        let (_sender, cancel) = watch::channel(false);
        crate::engine::run_process(
            ffmpeg.clone(),
            vec![
                "-v".into(),
                "error".into(),
                "-f".into(),
                "lavfi".into(),
                "-i".into(),
                "testsrc2=size=320x240:rate=1".into(),
                "-frames:v".into(),
                "1".into(),
                "-threads".into(),
                "2".into(),
                "-n".into(),
                a.to_string_lossy().into_owned(),
            ],
            cancel.clone(),
            1,
            true,
            |_| {},
        )
        .await
        .unwrap();
        crate::engine::run_process(
            ffmpeg.clone(),
            vec![
                "-v".into(),
                "error".into(),
                "-i".into(),
                a.to_string_lossy().into_owned(),
                "-vf".into(),
                "scale=160:120".into(),
                "-frames:v".into(),
                "1".into(),
                "-q:v".into(),
                "4".into(),
                "-threads".into(),
                "2".into(),
                "-n".into(),
                b.to_string_lossy().into_owned(),
            ],
            cancel.clone(),
            2,
            true,
            |_| {},
        )
        .await
        .unwrap();
        let original = fs::read(&a).unwrap();
        let items = [
            library::index(&a, None, None, None).unwrap(),
            library::index(&b, None, None, None).unwrap(),
        ];
        let ids = items.iter().map(|i| i.id.clone()).collect::<Vec<_>>();
        let result = analyze(
            &ffmpeg,
            &root,
            &items,
            &ids,
            &AnalyzeOptions::Similar { max_distance: 8 },
            cancel,
            3,
            |_| {},
        )
        .await
        .unwrap();
        assert_eq!(result.similar_pairs.len(), 1);
        assert!(result.similar_pairs[0].distance <= 8);
        assert!(result.warnings.iter().any(|w| w.contains("estimate")));
        assert_eq!(fs::read(&a).unwrap(), original);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
        fs::remove_dir_all(root).unwrap();
    }
}
