//! Types for representing client capabilities sent to LMS.
//!
//! The LMS Protocol requires clients to announce supported capabilities
//! when they connect. This module exposes the `Capability` enum for a single
//! capability item and `CapList` for composing the full capability string.
use std::fmt;

/// A client capability as recognized by the server.
///
/// Variants map to the textual representation expected by LMS when a client
/// announces itself. The `Display` implementation converts each variant into
/// the corresponding protocol token.
#[derive(Clone)]
pub enum Capability {
    Wma,
    Wmap,
    Wmal,
    Ogg,
    Flc,
    Pcm,
    Aif,
    Mp3,
    Alc,
    Aac,
    Maxsamplerate(u32),
    Model(String),
    Modelname(String),
    Rhap,
    Accurateplaypoints,
    Syncgroupid(String),
    Hasdigitalout,
    Haspreamp,
    Hasdisabledac,
    Firmware(String),
    Balance,
    CanHTTPS,
}

/// When sent to the server a capability is sent as text
impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let cap_str = match &self {
            Capability::Wma => "wma",
            Capability::Wmap => "wmap",
            Capability::Wmal => "wmal",
            Capability::Ogg => "ogg",
            Capability::Flc => "flc",
            Capability::Pcm => "pcm",
            Capability::Aif => "aif",
            Capability::Mp3 => "mp3",
            Capability::Alc => "alc",
            Capability::Aac => "aac",
            Capability::Maxsamplerate(v) => return write!(f, "MaxSampleRate={}", v),
            Capability::Model(v) => return write!(f, "Model={}", v),
            Capability::Modelname(v) => return write!(f, "Modelname={}", v),
            Capability::Rhap => "Rhap",
            Capability::Accurateplaypoints => "AccuratePlayPoints=1",
            Capability::Syncgroupid(v) => return write!(f, "SyncgroupID={}", v),
            Capability::Hasdigitalout => "HasDigitalOut=1",
            Capability::Haspreamp => "HasPreAmp=1",
            Capability::Hasdisabledac => "HasDisableDac=1",
            Capability::Firmware(v) => return write!(f, "Firmware={}", v),
            Capability::Balance => "Balance=1",
            Capability::CanHTTPS => "canHTTPS=1",
        };
        write!(f, "{cap_str}")
    }
}

/// A list of capabilities sent to the server when the client announces itself.
///
/// `CapList` formats as a comma-separated list suitable for inclusion in the
/// HELO capability string.
#[derive(Clone, Default)]
pub(crate) struct CapList(pub(crate) Vec<Capability>);

impl CapList {
    pub fn new(caps: Vec<Capability>) -> Self {
        Self(caps)
    }
}

impl fmt::Display for CapList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let capstr = self.0.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        write!(f, "{}", capstr.join(","))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single() {
        let v = vec![Capability::Mp3];
        let c = CapList::new(v);
        assert_eq!(c.to_string(), "mp3");
    }

    #[test]
    fn list_with_values() {
        let v = vec![
            Capability::Mp3,
            Capability::Maxsamplerate(9600),
            Capability::Ogg,
        ];
        let c = CapList::new(v);
        assert_eq!(c.to_string(), "mp3,MaxSampleRate=9600,ogg");
    }
}
