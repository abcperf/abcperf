use anyhow::Result;
use base64::URL_SAFE;
use serde::{Deserialize, Serialize};
use serde_bytes_repr::{ByteFmtDeserializer, ByteFmtSerializer};
use serde_json::{Deserializer, Serializer};

pub fn to_vec<T>(value: &T) -> Vec<u8>
where
    T: ?Sized + Serialize,
{
    let mut data = Vec::new();

    let mut serializer = Serializer::new(&mut data);
    let serializer = ByteFmtSerializer::base64(&mut serializer, URL_SAFE);
    value
        .serialize(serializer)
        .expect("should always be serializable");

    data
}

pub fn from_slice<'a, T>(data: &'a [u8]) -> Result<T>
where
    T: Deserialize<'a>,
{
    let mut deserializer = Deserializer::from_slice(data);
    let deserializer = ByteFmtDeserializer::new_base64(&mut deserializer, URL_SAFE);
    let value = T::deserialize(deserializer)?;

    Ok(value)
}
