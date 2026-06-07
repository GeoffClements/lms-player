// src/notify.rs
//
// Desktop notification implementation.
//
// Previously consumed a `symphonia::core::meta::MetadataRevision`; now
// receives a plain `HashMap<String, String>` whose keys are lower-cased
// FFmpeg metadata key names (e.g. "title", "artist", "album_artist",
// "album", "date").
//
// Required system dependencies:
// - dbus development libraries (libdbus-1-dev on Debian-based systems)
// - package configuration tool (pkg-config on Debian-based systems)

use std::{collections::HashMap, thread};

use notify_rust::Notification;

/// Spawn a background thread that raises a desktop notification for the
/// currently playing track described by `metadata`.
pub fn notify(metadata: HashMap<String, String>) {
    thread::spawn(move || {
        // Resolve artist: prefer album artist, fall back to track artist.
        let artist = metadata
            .get("album_artist")
            .or_else(|| metadata.get("artist"))
            .cloned();

        let album = metadata.get("album").cloned();
        let title = metadata.get("title").cloned();

        // Extract a 4-digit year from the "date" key which FFmpeg encodes as
        // an ISO 8601 string ("2003-07-15", "2003", etc.).
        let year = metadata.get("date").and_then(|date| {
            date.split('-')
                .map(str::trim)
                .find(|part| part.len() == 4 && part.chars().all(|c| c.is_ascii_digit()))
                .map(|y| y.to_owned())
        });

        if let Some(track) = title {
            let mut body = format!("<b>{}</b>", track);

            if let Some(a) = artist {
                body.push_str(&format!(" by <b>{}</b>", a));
            }
            if let Some(al) = album {
                body.push_str(&format!(" from <b>{}</b>", al));
            }
            if let Some(y) = year {
                body.push_str(&format!(" ({})", y));
            }

            _ = Notification::new()
                .summary("Now playing")
                .body(&body)
                .icon("emblem-music-symbolic")
                .timeout(6000)
                .show();
        }
    });
}
