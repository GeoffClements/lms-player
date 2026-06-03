// src/notify.rs
// Desktop notification implementation
//
// Required system dependencies:
// - dbus development libraries (libdbus-1-dev on Debian-based systems)
// - package configuration tool (pkg-config on Debian-based systems)

use std::{collections::HashMap, ops::Deref, thread};

use notify_rust::Notification;
use symphonia::core::meta::{MetadataRevision, StandardTag};

pub fn notify(metadata: MetadataRevision) {
    thread::spawn(move || {
        let notify_tags = metadata
            .media
            .tags
            .iter()
            .fold(HashMap::new(), |mut tags, tag| {
                match tag.std {
                    Some(StandardTag::AlbumArtist(ref album_artist)) => {
                        tags.insert("artist", album_artist.deref().clone());
                    }

                    Some(StandardTag::Artist(ref artist)) => {
                        tags.entry("artist").or_insert(artist.deref().clone());
                    }

                    Some(StandardTag::Album(ref album)) => {
                        tags.insert("album", album.deref().clone());
                    }

                    Some(StandardTag::TrackTitle(ref track_title)) => {
                        tags.insert("track", track_title.deref().clone());
                    }

                    Some(StandardTag::RecordingDate(ref date)) => {
                        let year: String = date
                            .split('-')
                            .map(|s| s.trim())
                            .filter(|p| p.len() == 4)
                            .take(1)
                            .collect();
                        if !year.is_empty() {
                            tags.insert("year", year);
                        }
                    }

                    Some(StandardTag::ReleaseYear(ref year))
                    | Some(StandardTag::OriginalReleaseYear(ref year))
                    | Some(StandardTag::RecordingYear(ref year))
                    | Some(StandardTag::OriginalRecordingYear(ref year)) => {
                        tags.entry("year").or_insert(year.to_string());
                    }

                    _ => {}
                }
                tags
            });

        let mut notification = String::new();
        if let Some(track) = notify_tags.get("track") {
            notification.push_str(format!("<b>{}</b>", track).as_str());

            if let Some(artist) = notify_tags.get("artist") {
                notification.push_str(format!(" by <b>{}</b>", artist).as_str());
            }

            if let Some(album) = notify_tags.get("album") {
                notification.push_str(format!(" from <b>{}</b>", album).as_str());
            }

            if let Some(date) = notify_tags.get("year") {
                notification.push_str(format!(" ({})", date).as_str());
            }

            _ = Notification::new()
                .summary("Now playing")
                .body(&notification)
                .icon("emblem-music-symbolic")
                .timeout(6000)
                .show();
        }
    });
}
