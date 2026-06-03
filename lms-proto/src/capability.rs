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

impl PartialEq for Capability {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Capability::Wma, Capability::Wma) => true,
            (Capability::Wmap, Capability::Wmap) => true,
            (Capability::Wmal, Capability::Wmal) => true,
            (Capability::Ogg, Capability::Ogg) => true,
            (Capability::Flc, Capability::Flc) => true,
            (Capability::Pcm, Capability::Pcm) => true,
            (Capability::Aif, Capability::Aif) => true,
            (Capability::Mp3, Capability::Mp3) => true,
            (Capability::Alc, Capability::Alc) => true,
            (Capability::Aac, Capability::Aac) => true,
            (Capability::Maxsamplerate(_), Capability::Maxsamplerate(_)) => true,
            (Capability::Model(_), Capability::Model(_)) => true,
            (Capability::Modelname(_), Capability::Modelname(_)) => true,
            (Capability::Rhap, Capability::Rhap) => true,
            (Capability::Accurateplaypoints, Capability::Accurateplaypoints) => true,
            (Capability::Syncgroupid(_), Capability::Syncgroupid(_)) => true,
            (Capability::Hasdigitalout, Capability::Hasdigitalout) => true,
            (Capability::Haspreamp, Capability::Haspreamp) => true,
            (Capability::Hasdisabledac, Capability::Hasdisabledac) => true,
            (Capability::Firmware(_), Capability::Firmware(_)) => true,
            (Capability::Balance, Capability::Balance) => true,
            (Capability::CanHTTPS, Self::CanHTTPS) => true,
            _ => false,
        }
    }
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
            Capability::Maxsamplerate(v) => write!(f, "MaxSampleRate={}", v.to_string()),
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
#[derive(Clone)]
pub struct Capabilities(pub(crate) Vec<Capability>);

impl Capabilities {
    /// Add a new capability to the list. Note that capabilities are sent to the server
    /// in the order that they are added to the list.
    ///
    /// Normally you will not need to use this method as capabilities are usually added
    /// using the [add_capability](crate::proto::SlimProto::add_capability) method.
    pub fn add(&mut self, newcap: Capability) {
        if let Some(index) = self.0.iter().position(|c| c == &newcap) {
            // If the capability already exists, remove it first
            self.0.remove(index);
        }
        self.0.push(newcap);
    }

    pub fn add_name(&mut self, name: &str) {
        self.add(Capability::Modelname(name.to_owned()));
    }
}

/// Default to most likely capabilities for a Squeezelite client.
impl Default for Capabilities {
    fn default() -> Self {
        let mut caps = Vec::new();
        caps.push(Capability::Model("squeezelite".to_owned()));
        caps.push(Capability::Modelname("SqueezeLite".to_owned()));
        caps.push(Capability::Accurateplaypoints);
        caps.push(Capability::Hasdigitalout);
        caps.push(Capability::Haspreamp);
        caps.push(Capability::Hasdisabledac);
        Self(caps)
    }
}

impl fmt::Display for Capabilities {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self(ref caps) = self;
        let capstr = caps.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        write!(f, "{}", capstr.join(","))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single() {
        let mut c = Capabilities::default();
        c.add(Capability::Mp3);
        assert_eq!(c.to_string(), "Model=squeezelite,Modelname=SqueezeLite,AccuratePlayPoints=1,HasDigitalOut=1,HasPreAmp=1,HasDisableDac=1,mp3");
    }

    #[test]
    fn list_with_values() {
        let mut c = Capabilities::default();
        c.add(Capability::Mp3);
        c.add(Capability::Maxsamplerate(9600));
        c.add(Capability::Ogg);
        assert_eq!(c.to_string(), "Model=squeezelite,Modelname=SqueezeLite,AccuratePlayPoints=1,HasDigitalOut=1,HasPreAmp=1,HasDisableDac=1,mp3,MaxSampleRate=9600,ogg");
    }

    #[test]
    fn name() {
        let mut c = Capabilities::default();
        c.add_name("Testing");
        assert_eq!(c.to_string(), "Model=squeezelite,AccuratePlayPoints=1,HasDigitalOut=1,HasPreAmp=1,HasDisableDac=1,Modelname=Testing");
    }
}
