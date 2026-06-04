/// Provides the types needed to send capability data to the server.
use std::fmt;

/// A client capability as recognized by by the server. Sent as a list of capabilities
/// when the client announces itself to the server
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
        match &self {
            Capability::Wma => write!(f, "wma"),
            Capability::Wmap => write!(f, "wmap"),
            Capability::Wmal => write!(f, "wmal"),
            Capability::Ogg => write!(f, "ogg"),
            Capability::Flc => write!(f, "flc"),
            Capability::Pcm => write!(f, "pcm"),
            Capability::Aif => write!(f, "aif"),
            Capability::Mp3 => write!(f, "mp3"),
            Capability::Alc => write!(f, "alc"),
            Capability::Aac => write!(f, "aac"),
            Capability::Maxsamplerate(v) => write!(f, "MaxSampleRate={}", v),
            Capability::Model(v) => write!(f, "Model={}", v),
            Capability::Modelname(v) => write!(f, "Modelname={}", v),
            Capability::Rhap => write!(f, "Rhap"),
            Capability::Accurateplaypoints => write!(f, "AccuratePlayPoints=1"),
            Capability::Syncgroupid(v) => write!(f, "SyncgroupID={}", v),
            Capability::Hasdigitalout => write!(f, "HasDigitalOut=1"),
            Capability::Haspreamp => write!(f, "HasPreAmp=1"),
            Capability::Hasdisabledac => write!(f, "HasDisableDac=1"),
            Capability::Firmware(v) => write!(f, "Firmware={}", v),
            Capability::Balance => write!(f, "Balance=1"),
            Capability::CanHTTPS => write!(f, "canHTTPS=1"),
        }
    }
}

/// A list of capabilities which is sent to the server when the client announces itself.
/// See [SlimpProto](crate::proto::SlimProto) for more details.
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
