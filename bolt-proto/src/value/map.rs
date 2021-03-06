use std::collections::HashMap;
use std::convert::{TryFrom, TryInto};
use std::hash::BuildHasher;
use std::mem;
use std::panic::catch_unwind;
use std::sync::{Arc, Mutex};

use bytes::{Buf, BufMut, Bytes, BytesMut};

use crate::error::*;
use crate::serialization::*;
use crate::value::String;
use crate::Value;

pub(crate) const MARKER_TINY: u8 = 0xA0;
pub(crate) const MARKER_SMALL: u8 = 0xD8;
pub(crate) const MARKER_MEDIUM: u8 = 0xD9;
pub(crate) const MARKER_LARGE: u8 = 0xDA;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Map {
    pub(crate) value: HashMap<String, Value>,
}

impl Marker for Map {
    fn get_marker(&self) -> Result<u8> {
        match self.value.len() {
            0..=15 => Ok(MARKER_TINY | self.value.len() as u8),
            16..=255 => Ok(MARKER_SMALL),
            256..=65_535 => Ok(MARKER_MEDIUM),
            65_536..=4_294_967_295 => Ok(MARKER_LARGE),
            _ => Err(Error::ValueTooLarge(self.value.len())),
        }
    }
}

impl Serialize for Map {}

impl TryInto<Bytes> for Map {
    type Error = Error;

    fn try_into(self) -> Result<Bytes> {
        let marker = self.get_marker()?;
        let length = self.value.len();

        let mut total_value_bytes: usize = 0;
        let mut value_bytes_vec: Vec<Bytes> = Vec::with_capacity(length);
        for (key, val) in self.value {
            let key_bytes: Bytes = key.try_into()?;
            let val_bytes: Bytes = val.try_into()?;
            total_value_bytes += key_bytes.len() + val_bytes.len();
            value_bytes_vec.push(key_bytes);
            value_bytes_vec.push(val_bytes);
        }
        // Worst case is a large Map, with marker byte, 32-bit size value, and all the
        // Value bytes
        let mut bytes = BytesMut::with_capacity(
            mem::size_of::<u8>() + mem::size_of::<u32>() + total_value_bytes,
        );
        bytes.put_u8(marker);
        match length {
            0..=15 => {}
            16..=255 => bytes.put_u8(length as u8),
            256..=65_535 => bytes.put_u16(length as u16),
            65_536..=4_294_967_295 => bytes.put_u32(length as u32),
            _ => return Err(Error::ValueTooLarge(length)),
        }
        for value_bytes in value_bytes_vec {
            bytes.put(value_bytes);
        }
        Ok(bytes.freeze())
    }
}

impl Deserialize for Map {}

impl TryFrom<Arc<Mutex<Bytes>>> for Map {
    type Error = Error;

    fn try_from(input_arc: Arc<Mutex<Bytes>>) -> Result<Self> {
        catch_unwind(move || {
            let marker = input_arc.lock().unwrap().get_u8();
            let size = match marker {
                marker if (MARKER_TINY..=(MARKER_TINY | 0x0F)).contains(&marker) => {
                    0x0F & marker as usize
                }
                MARKER_SMALL => input_arc.lock().unwrap().get_u8() as usize,
                MARKER_MEDIUM => input_arc.lock().unwrap().get_u16() as usize,
                MARKER_LARGE => input_arc.lock().unwrap().get_u32() as usize,
                _ => {
                    return Err(DeserializationError::InvalidMarkerByte(marker).into());
                }
            };
            let mut hash_map: HashMap<String, Value> = HashMap::with_capacity(size);
            for _ in 0..size {
                let key = String::try_from(Arc::clone(&input_arc))?;
                let value = Value::try_from(Arc::clone(&input_arc))?;
                hash_map.insert(key, value);
            }
            Ok(Map::from(hash_map))
        })
        .map_err(|_| DeserializationError::Panicked)?
    }
}

impl<K, V, S> From<HashMap<K, V, S>> for Map
where
    K: Into<String>,
    V: Into<Value>,
    S: BuildHasher,
{
    fn from(value: HashMap<K, V, S>) -> Self {
        Self {
            value: value
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::clone::Clone;
    use std::collections::HashMap;
    use std::convert::TryFrom;
    use std::iter::FromIterator;
    use std::sync::{Arc, Mutex};

    use bytes::Bytes;

    use super::*;

    #[test]
    fn get_marker() {
        let empty_map: Map = HashMap::<&str, i8>::new().into();
        assert_eq!(empty_map.get_marker().unwrap(), MARKER_TINY);
        let tiny_map: Map =
            HashMap::<&str, i8>::from_iter(vec![("a", 1_i8), ("b", 2_i8), ("c", 3_i8)]).into();
        assert_eq!(
            tiny_map.get_marker().unwrap(),
            MARKER_TINY | tiny_map.value.len() as u8
        );
    }

    #[test]
    fn try_into_bytes() {
        let empty_map: Map = HashMap::<&str, i8>::new().into();
        assert_eq!(
            empty_map.try_into_bytes().unwrap(),
            Bytes::from_static(&[MARKER_TINY])
        );
        let tiny_map: Map = HashMap::<&str, i8>::from_iter(vec![("a", 1_i8)]).into();
        assert_eq!(
            tiny_map.try_into_bytes().unwrap(),
            Bytes::from_static(&[MARKER_TINY | 1, 0x81, 0x61, 0x01])
        );

        let small_map: Map = HashMap::<&str, i8>::from_iter(vec![
            ("a", 1_i8),
            ("b", 1_i8),
            ("c", 3_i8),
            ("d", 4_i8),
            ("e", 5_i8),
            ("f", 6_i8),
            ("g", 7_i8),
            ("h", 8_i8),
            ("i", 9_i8),
            ("j", 0_i8),
            ("k", 1_i8),
            ("l", 2_i8),
            ("m", 3_i8),
            ("n", 4_i8),
            ("o", 5_i8),
            ("p", 6_i8),
        ])
        .into();
        let small_len = small_map.value.len();
        let small_bytes = small_map.try_into_bytes().unwrap();
        // Can't check the whole map since the bytes are in no particular order, check
        // marker/length instead
        assert_eq!(small_bytes[0], MARKER_SMALL);
        // Marker byte, size (u8), then list of 2-byte String (marker, value) + 1-byte
        // tiny ints
        assert_eq!(small_bytes.len(), 2 + small_len * 3);
    }

    #[test]
    fn try_from_bytes() {
        let empty_map: Map = HashMap::<&str, i8>::new().into();
        let empty_map_bytes = empty_map.clone().try_into_bytes().unwrap();
        let tiny_map: Map = HashMap::<&str, i8>::from_iter(vec![("a", 1_i8)]).into();
        let tiny_map_bytes = tiny_map.clone().try_into_bytes().unwrap();
        let small_map: Map = HashMap::<&str, i8>::from_iter(vec![
            ("a", 1_i8),
            ("b", 1_i8),
            ("c", 3_i8),
            ("d", 4_i8),
            ("e", 5_i8),
            ("f", 6_i8),
            ("g", 7_i8),
            ("h", 8_i8),
            ("i", 9_i8),
            ("j", 0_i8),
            ("k", 1_i8),
            ("l", 2_i8),
            ("m", 3_i8),
            ("n", 4_i8),
            ("o", 5_i8),
            ("p", 6_i8),
        ])
        .into();
        let small_map_bytes = small_map.clone().try_into_bytes().unwrap();

        assert_eq!(
            Map::try_from(Arc::new(Mutex::new(empty_map_bytes))).unwrap(),
            empty_map
        );
        assert_eq!(
            Map::try_from(Arc::new(Mutex::new(tiny_map_bytes))).unwrap(),
            tiny_map
        );
        assert_eq!(
            Map::try_from(Arc::new(Mutex::new(small_map_bytes))).unwrap(),
            small_map
        );
    }

    #[test]
    fn deep_nested_map_is_ok() {
        let bytes = Bytes::from_static(&[
            // From https://boltprotocol.org/v1/#accessing_notifications
            0xA4, 0x84, 0x74, 0x79, 0x70, 0x65, 0x81, 0x72, 0xD0, 0x15, 0x72, 0x65, 0x73, 0x75,
            0x6C, 0x74, 0x5F, 0x63, 0x6F, 0x6E, 0x73, 0x75, 0x6D, 0x65, 0x64, 0x5F, 0x61, 0x66,
            0x74, 0x65, 0x72, 0x0C, 0x84, 0x70, 0x6C, 0x61, 0x6E, 0xA4, 0x84, 0x61, 0x72, 0x67,
            0x73, 0xA7, 0x8C, 0x72, 0x75, 0x6E, 0x74, 0x69, 0x6D, 0x65, 0x2D, 0x69, 0x6D, 0x70,
            0x6C, 0x8B, 0x49, 0x4E, 0x54, 0x45, 0x52, 0x50, 0x52, 0x45, 0x54, 0x45, 0x44, 0x8C,
            0x70, 0x6C, 0x61, 0x6E, 0x6E, 0x65, 0x72, 0x2D, 0x69, 0x6D, 0x70, 0x6C, 0x83, 0x49,
            0x44, 0x50, 0x87, 0x76, 0x65, 0x72, 0x73, 0x69, 0x6F, 0x6E, 0x8A, 0x43, 0x59, 0x50,
            0x48, 0x45, 0x52, 0x20, 0x33, 0x2E, 0x31, 0x88, 0x4B, 0x65, 0x79, 0x4E, 0x61, 0x6D,
            0x65, 0x73, 0x84, 0x6E, 0x2C, 0x20, 0x6D, 0x8D, 0x45, 0x73, 0x74, 0x69, 0x6D, 0x61,
            0x74, 0x65, 0x64, 0x52, 0x6F, 0x77, 0x73, 0xC1, 0x3F, 0xF0, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x87, 0x70, 0x6C, 0x61, 0x6E, 0x6E, 0x65, 0x72, 0x84, 0x43, 0x4F, 0x53,
            0x54, 0x87, 0x72, 0x75, 0x6E, 0x74, 0x69, 0x6D, 0x65, 0x8B, 0x49, 0x4E, 0x54, 0x45,
            0x52, 0x50, 0x52, 0x45, 0x54, 0x45, 0x44, 0x88, 0x63, 0x68, 0x69, 0x6C, 0x64, 0x72,
            0x65, 0x6E, 0x91, 0xA4, 0x84, 0x61, 0x72, 0x67, 0x73, 0xA1, 0x8D, 0x45, 0x73, 0x74,
            0x69, 0x6D, 0x61, 0x74, 0x65, 0x64, 0x52, 0x6F, 0x77, 0x73, 0xC1, 0x3F, 0xF0, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x88, 0x63, 0x68, 0x69, 0x6C, 0x64, 0x72, 0x65, 0x6E,
            0x92, 0xA4, 0x84, 0x61, 0x72, 0x67, 0x73, 0xA1, 0x8D, 0x45, 0x73, 0x74, 0x69, 0x6D,
            0x61, 0x74, 0x65, 0x64, 0x52, 0x6F, 0x77, 0x73, 0xC1, 0x3F, 0xF0, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x88, 0x63, 0x68, 0x69, 0x6C, 0x64, 0x72, 0x65, 0x6E, 0x90, 0x8B,
            0x69, 0x64, 0x65, 0x6E, 0x74, 0x69, 0x66, 0x69, 0x65, 0x72, 0x73, 0x91, 0x81, 0x6E,
            0x8C, 0x6F, 0x70, 0x65, 0x72, 0x61, 0x74, 0x6F, 0x72, 0x54, 0x79, 0x70, 0x65, 0x8C,
            0x41, 0x6C, 0x6C, 0x4E, 0x6F, 0x64, 0x65, 0x73, 0x53, 0x63, 0x61, 0x6E, 0xA4, 0x84,
            0x61, 0x72, 0x67, 0x73, 0xA1, 0x8D, 0x45, 0x73, 0x74, 0x69, 0x6D, 0x61, 0x74, 0x65,
            0x64, 0x52, 0x6F, 0x77, 0x73, 0xC1, 0x3F, 0xF0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x88, 0x63, 0x68, 0x69, 0x6C, 0x64, 0x72, 0x65, 0x6E, 0x90, 0x8B, 0x69, 0x64, 0x65,
            0x6E, 0x74, 0x69, 0x66, 0x69, 0x65, 0x72, 0x73, 0x91, 0x81, 0x6D, 0x8C, 0x6F, 0x70,
            0x65, 0x72, 0x61, 0x74, 0x6F, 0x72, 0x54, 0x79, 0x70, 0x65, 0x8C, 0x41, 0x6C, 0x6C,
            0x4E, 0x6F, 0x64, 0x65, 0x73, 0x53, 0x63, 0x61, 0x6E, 0x8B, 0x69, 0x64, 0x65, 0x6E,
            0x74, 0x69, 0x66, 0x69, 0x65, 0x72, 0x73, 0x92, 0x81, 0x6D, 0x81, 0x6E, 0x8C, 0x6F,
            0x70, 0x65, 0x72, 0x61, 0x74, 0x6F, 0x72, 0x54, 0x79, 0x70, 0x65, 0xD0, 0x10, 0x43,
            0x61, 0x72, 0x74, 0x65, 0x73, 0x69, 0x61, 0x6E, 0x50, 0x72, 0x6F, 0x64, 0x75, 0x63,
            0x74, 0x8B, 0x69, 0x64, 0x65, 0x6E, 0x74, 0x69, 0x66, 0x69, 0x65, 0x72, 0x73, 0x92,
            0x81, 0x6D, 0x81, 0x6E, 0x8C, 0x6F, 0x70, 0x65, 0x72, 0x61, 0x74, 0x6F, 0x72, 0x54,
            0x79, 0x70, 0x65, 0x8E, 0x50, 0x72, 0x6F, 0x64, 0x75, 0x63, 0x65, 0x52, 0x65, 0x73,
            0x75, 0x6C, 0x74, 0x73, 0x8D, 0x6E, 0x6F, 0x74, 0x69, 0x66, 0x69, 0x63, 0x61, 0x74,
            0x69, 0x6F, 0x6E, 0x73, 0x91, 0xA5, 0x88, 0x73, 0x65, 0x76, 0x65, 0x72, 0x69, 0x74,
            0x79, 0x87, 0x57, 0x41, 0x52, 0x4E, 0x49, 0x4E, 0x47, 0x85, 0x74, 0x69, 0x74, 0x6C,
            0x65, 0xD0, 0x44, 0x54, 0x68, 0x69, 0x73, 0x20, 0x71, 0x75, 0x65, 0x72, 0x79, 0x20,
            0x62, 0x75, 0x69, 0x6C, 0x64, 0x73, 0x20, 0x61, 0x20, 0x63, 0x61, 0x72, 0x74, 0x65,
            0x73, 0x69, 0x61, 0x6E, 0x20, 0x70, 0x72, 0x6F, 0x64, 0x75, 0x63, 0x74, 0x20, 0x62,
            0x65, 0x74, 0x77, 0x65, 0x65, 0x6E, 0x20, 0x64, 0x69, 0x73, 0x63, 0x6F, 0x6E, 0x6E,
            0x65, 0x63, 0x74, 0x65, 0x64, 0x20, 0x70, 0x61, 0x74, 0x74, 0x65, 0x72, 0x6E, 0x73,
            0x2E, 0x84, 0x63, 0x6F, 0x64, 0x65, 0xD0, 0x38, 0x4E, 0x65, 0x6F, 0x2E, 0x43, 0x6C,
            0x69, 0x65, 0x6E, 0x74, 0x4E, 0x6F, 0x74, 0x69, 0x66, 0x69, 0x63, 0x61, 0x74, 0x69,
            0x6F, 0x6E, 0x2E, 0x53, 0x74, 0x61, 0x74, 0x65, 0x6D, 0x65, 0x6E, 0x74, 0x2E, 0x43,
            0x61, 0x72, 0x74, 0x65, 0x73, 0x69, 0x61, 0x6E, 0x50, 0x72, 0x6F, 0x64, 0x75, 0x63,
            0x74, 0x57, 0x61, 0x72, 0x6E, 0x69, 0x6E, 0x67, 0x8B, 0x64, 0x65, 0x73, 0x63, 0x72,
            0x69, 0x70, 0x74, 0x69, 0x6F, 0x6E, 0xD1, 0x01, 0xA9, 0x49, 0x66, 0x20, 0x61, 0x20,
            0x70, 0x61, 0x72, 0x74, 0x20, 0x6F, 0x66, 0x20, 0x61, 0x20, 0x71, 0x75, 0x65, 0x72,
            0x79, 0x20, 0x63, 0x6F, 0x6E, 0x74, 0x61, 0x69, 0x6E, 0x73, 0x20, 0x6D, 0x75, 0x6C,
            0x74, 0x69, 0x70, 0x6C, 0x65, 0x20, 0x64, 0x69, 0x73, 0x63, 0x6F, 0x6E, 0x6E, 0x65,
            0x63, 0x74, 0x65, 0x64, 0x20, 0x70, 0x61, 0x74, 0x74, 0x65, 0x72, 0x6E, 0x73, 0x2C,
            0x20, 0x74, 0x68, 0x69, 0x73, 0x20, 0x77, 0x69, 0x6C, 0x6C, 0x20, 0x62, 0x75, 0x69,
            0x6C, 0x64, 0x20, 0x61, 0x20, 0x63, 0x61, 0x72, 0x74, 0x65, 0x73, 0x69, 0x61, 0x6E,
            0x20, 0x70, 0x72, 0x6F, 0x64, 0x75, 0x63, 0x74, 0x20, 0x62, 0x65, 0x74, 0x77, 0x65,
            0x65, 0x6E, 0x20, 0x61, 0x6C, 0x6C, 0x20, 0x74, 0x68, 0x6F, 0x73, 0x65, 0x20, 0x70,
            0x61, 0x72, 0x74, 0x73, 0x2E, 0x20, 0x54, 0x68, 0x69, 0x73, 0x20, 0x6D, 0x61, 0x79,
            0x20, 0x70, 0x72, 0x6F, 0x64, 0x75, 0x63, 0x65, 0x20, 0x61, 0x20, 0x6C, 0x61, 0x72,
            0x67, 0x65, 0x20, 0x61, 0x6D, 0x6F, 0x75, 0x6E, 0x74, 0x20, 0x6F, 0x66, 0x20, 0x64,
            0x61, 0x74, 0x61, 0x20, 0x61, 0x6E, 0x64, 0x20, 0x73, 0x6C, 0x6F, 0x77, 0x20, 0x64,
            0x6F, 0x77, 0x6E, 0x20, 0x71, 0x75, 0x65, 0x72, 0x79, 0x20, 0x70, 0x72, 0x6F, 0x63,
            0x65, 0x73, 0x73, 0x69, 0x6E, 0x67, 0x2E, 0x20, 0x57, 0x68, 0x69, 0x6C, 0x65, 0x20,
            0x6F, 0x63, 0x63, 0x61, 0x73, 0x69, 0x6F, 0x6E, 0x61, 0x6C, 0x6C, 0x79, 0x20, 0x69,
            0x6E, 0x74, 0x65, 0x6E, 0x64, 0x65, 0x64, 0x2C, 0x20, 0x69, 0x74, 0x20, 0x6D, 0x61,
            0x79, 0x20, 0x6F, 0x66, 0x74, 0x65, 0x6E, 0x20, 0x62, 0x65, 0x20, 0x70, 0x6F, 0x73,
            0x73, 0x69, 0x62, 0x6C, 0x65, 0x20, 0x74, 0x6F, 0x20, 0x72, 0x65, 0x66, 0x6F, 0x72,
            0x6D, 0x75, 0x6C, 0x61, 0x74, 0x65, 0x20, 0x74, 0x68, 0x65, 0x20, 0x71, 0x75, 0x65,
            0x72, 0x79, 0x20, 0x74, 0x68, 0x61, 0x74, 0x20, 0x61, 0x76, 0x6F, 0x69, 0x64, 0x73,
            0x20, 0x74, 0x68, 0x65, 0x20, 0x75, 0x73, 0x65, 0x20, 0x6F, 0x66, 0x20, 0x74, 0x68,
            0x69, 0x73, 0x20, 0x63, 0x72, 0x6F, 0x73, 0x73, 0x20, 0x70, 0x72, 0x6F, 0x64, 0x75,
            0x63, 0x74, 0x2C, 0x20, 0x70, 0x65, 0x72, 0x68, 0x61, 0x70, 0x73, 0x20, 0x62, 0x79,
            0x20, 0x61, 0x64, 0x64, 0x69, 0x6E, 0x67, 0x20, 0x61, 0x20, 0x72, 0x65, 0x6C, 0x61,
            0x74, 0x69, 0x6F, 0x6E, 0x73, 0x68, 0x69, 0x70, 0x20, 0x62, 0x65, 0x74, 0x77, 0x65,
            0x65, 0x6E, 0x20, 0x74, 0x68, 0x65, 0x20, 0x64, 0x69, 0x66, 0x66, 0x65, 0x72, 0x65,
            0x6E, 0x74, 0x20, 0x70, 0x61, 0x72, 0x74, 0x73, 0x20, 0x6F, 0x72, 0x20, 0x62, 0x79,
            0x20, 0x75, 0x73, 0x69, 0x6E, 0x67, 0x20, 0x4F, 0x50, 0x54, 0x49, 0x4F, 0x4E, 0x41,
            0x4C, 0x20, 0x4D, 0x41, 0x54, 0x43, 0x48, 0x20, 0x28, 0x69, 0x64, 0x65, 0x6E, 0x74,
            0x69, 0x66, 0x69, 0x65, 0x72, 0x20, 0x69, 0x73, 0x3A, 0x20, 0x28, 0x6D, 0x29, 0x29,
            0x88, 0x70, 0x6F, 0x73, 0x69, 0x74, 0x69, 0x6F, 0x6E, 0xA3, 0x86, 0x6F, 0x66, 0x66,
            0x73, 0x65, 0x74, 0x00, 0x86, 0x63, 0x6F, 0x6C, 0x75, 0x6D, 0x6E, 0x01, 0x84, 0x6C,
            0x69, 0x6E, 0x65, 0x01,
        ]);
        assert!(Map::try_from(Arc::new(Mutex::new(bytes))).is_ok());
    }
}
