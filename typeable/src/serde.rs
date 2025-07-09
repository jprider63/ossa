
use crate::TypeId;
use serde::ser::{Serialize, Serializer};
use serde::de::{Deserialize, Deserializer};

impl Serialize for TypeId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for TypeId {
    fn deserialize<D>(deserializer: D) -> Result<TypeId, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(TypeId(Deserialize::deserialize(deserializer)?))
    }
}


