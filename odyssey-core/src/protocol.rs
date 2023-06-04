/// The protocol version.
pub enum Version {
    V0 = 0,
}

impl Version {
    pub fn as_byte(&self) -> u8 {
        *self as u8
    }
}

pub(crate) const LATEST_VERSION: Version = Version::V0;
