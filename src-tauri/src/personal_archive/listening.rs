use super::{parse::Record, PAGE};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum By {
    Creators,
    Items,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct View {
    pub by: By,
    pub key: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Group {
    key: String,
    title: String,
    creator: Option<String>,
    album: Option<String>,
    episode: bool,
    identified: bool,
    events: usize,
    played_ms: Option<u64>,
    completions: Option<usize>,
    completion_known: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GroupPage {
    groups: Vec<Group>,
    total: usize,
}

fn digest(data: impl AsRef<[u8]>) -> String {
    format!("{:x}", Sha256::digest(data))
}

pub(super) fn key(row: &Record, by: By) -> Option<String> {
    match by {
        By::Creators => Some(digest(row.author.as_deref()?)),
        By::Items => {
            let item = row.listening_item.as_ref();
            if row.title.is_empty()
                || item.is_none() && row.author.as_deref() == Some(row.title.as_str())
            {
                return None;
            }
            if let Some(uri) = item.and_then(|item| item.uri.as_ref()) {
                return Some(digest(uri));
            }
            let labels = (
                &row.title,
                &row.author,
                item.and_then(|item| item.album.as_ref()),
                item.is_some_and(|item| item.episode),
            );
            Some(digest(serde_json::to_vec(&labels).ok()?))
        }
    }
}

pub(super) fn group_page(rows: &[Record], by: By, offset: usize) -> GroupPage {
    let mut groups = HashMap::<String, Group>::new();
    for row in rows {
        let Some(key) = key(row, by) else {
            continue;
        };
        let group = groups.entry(key.clone()).or_insert_with(|| {
            let item = row
                .listening_item
                .as_ref()
                .filter(|_| matches!(by, By::Items));
            let (title, creator) = match by {
                By::Creators => (row.author.clone().unwrap_or_default(), None),
                By::Items => (row.title.clone(), row.author.clone()),
            };
            Group {
                key,
                title,
                creator,
                album: item.and_then(|item| item.album.clone()),
                episode: item.is_some_and(|item| item.episode),
                identified: item.is_some_and(|item| item.uri.is_some()),
                events: 0,
                played_ms: None,
                completions: None,
                completion_known: 0,
            }
        });
        group.events += 1;
        if let Some(played) = row.played_ms {
            group.played_ms = Some(group.played_ms.unwrap_or(0).saturating_add(played));
        }
        if let Some(complete) = row.completed {
            group.completion_known += 1;
            group.completions = Some(group.completions.unwrap_or(0) + usize::from(complete));
        }
    }
    let mut groups: Vec<_> = groups.into_values().collect();
    groups.sort_by(|a, b| {
        b.played_ms
            .cmp(&a.played_ms)
            .then_with(|| b.events.cmp(&a.events))
            .then_with(|| a.title.cmp(&b.title))
            .then_with(|| a.key.cmp(&b.key))
    });
    GroupPage {
        total: groups.len(),
        groups: groups.into_iter().skip(offset).take(PAGE).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::personal_archive::parse::{self, Category, Classification, Format, Provider};

    #[test]
    fn history_keeps_distinct_recordings_and_counts_only_known_completions() {
        let class = Classification {
            category: Category::Listening,
            format: Format::Json,
        };
        let rows = parse::parse(Provider::Spotify, class, "history", r#"[
            {"master_metadata_track_name":"Same title","master_metadata_album_artist_name":"Artist","master_metadata_album_album_name":"Album","spotify_track_uri":"spotify:track:AAAAAAAAAAAAAAAAAAAAAA","ms_played":1000,"reason_end":"trackdone"},
            {"master_metadata_track_name":"Same title","master_metadata_album_artist_name":"Artist","spotify_track_uri":"spotify:track:AAAAAAAAAAAAAAAAAAAAAA","ms_played":2000,"completed":true},
            {"master_metadata_track_name":"Same title","master_metadata_album_artist_name":"Artist","spotify_track_uri":"spotify:track:BBBBBBBBBBBBBBBBBBBBBB","ms_played":0},
            {"trackName":"Same title","artistName":"Artist","msPlayed":100},
            {"episode_name":"Same title","episode_show_name":"Show","spotify_episode_uri":"spotify:episode:AAAAAAAAAAAAAAAAAAAAAA","ms_played":1400}
        ]"#).unwrap();
        let items = group_page(&rows, By::Items, 0);
        assert_eq!(items.total, 4);
        assert_eq!(
            (
                items.groups[0].events,
                items.groups[0].played_ms,
                items.groups[0].completions,
                items.groups[0].completion_known
            ),
            (2, Some(3000), Some(1), 1)
        );
        assert_eq!(items.groups[0].album.as_deref(), Some("Album"));
        assert!(items.groups[1].episode);
        assert_eq!(items.groups[3].completions, None);
        let creators = group_page(&rows, By::Creators, 0);
        assert_eq!(creators.total, 2);
        assert_eq!(
            (
                creators.groups[0].title.as_str(),
                creators.groups[0].played_ms
            ),
            ("Artist", Some(3100))
        );

        let mut legacy = serde_json::to_value(&rows[0]).unwrap();
        legacy.as_object_mut().unwrap().remove("listeningItem");
        let old = serde_json::from_value(legacy).unwrap();
        let mut archive = std::collections::BTreeMap::from([(Category::Listening, vec![old])]);
        super::super::merge(
            &mut archive,
            std::collections::BTreeMap::from([(Category::Listening, vec![rows[0].clone()])]),
        )
        .unwrap();
        assert_eq!(archive[&Category::Listening], [rows[0].clone()]);
    }
}
