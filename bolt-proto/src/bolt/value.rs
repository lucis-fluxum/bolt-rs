use std::collections::hash_map::RandomState;
use std::collections::HashMap;
use std::convert::{TryFrom, TryInto};
use std::hash::Hash;
use std::panic::catch_unwind;
use std::sync::{Arc, Mutex};

use bytes::{Buf, Bytes};
use failure::Error;

use crate::bolt::structure;
use crate::bolt::structure::get_signature_from_bytes;
use crate::error::DeserializeError;
use crate::native;
use crate::serialize::{Deserialize, Serialize};

pub use self::boolean::Boolean;
pub use self::float::Float;
pub use self::integer::Integer;
pub use self::list::List;
pub use self::map::Map;
pub use self::node::Node;
pub use self::null::Null;
pub use self::path::Path;
pub use self::relationship::Relationship;
pub use self::string::String;
pub use self::unbound_relationship::UnboundRelationship;

mod boolean;
mod float;
mod integer;
mod list;
mod map;
mod node;
mod null;
mod path;
mod relationship;
mod string;
mod unbound_relationship;

pub trait Marker: Serialize + Deserialize {
    fn get_marker(&self) -> Result<u8, Error>;
}

// TODO: Consider manually implementing Hash here and only deriving hash on the values that support it
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub enum Value {
    Boolean(Boolean),
    Integer(Integer),
    Float(Float),
    List(List),
    Map(Map),
    Null(Null),
    String(String),
    Node(Node),
    Relationship(Relationship),
    Path(Path),
    UnboundRelationship(UnboundRelationship),
}

impl From<bool> for Value {
    fn from(value: bool) -> Self {
        Value::Boolean(Boolean::from(value))
    }
}

macro_rules! impl_from_int {
    ($($T:ty),+) => {
        $(
            impl From<$T> for $crate::bolt::value::Value {
                fn from(value: $T) -> Self {
                    Value::Integer(Integer::from(value))
                }
            }
        )*
    };
}
impl_from_int!(i8, i16, i32, i64);

impl From<f64> for Value {
    fn from(value: f64) -> Self {
        Value::Float(Float::from(value))
    }
}

impl<T> From<Vec<T>> for Value
where
    T: Into<Value>,
{
    fn from(value: Vec<T>) -> Self {
        Value::List(List::from(value))
    }
}

impl<K, V> From<HashMap<K, V>> for Value
where
    K: Into<Value>,
    V: Into<Value>,
{
    fn from(value: HashMap<K, V, RandomState>) -> Self {
        Value::Map(Map::from(value))
    }
}

// TODO: No Rust equivalent for Null, consider removing Null struct and just have an empty variant

impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Value::String(String::from(value))
    }
}

impl From<std::string::String> for Value {
    fn from(value: std::string::String) -> Self {
        Value::String(String::from(value))
    }
}

impl From<native::value::Node> for Value {
    fn from(native_node: native::value::Node) -> Self {
        Value::Node(Node::from(native_node))
    }
}

impl From<native::value::Relationship> for Value {
    fn from(value: native::value::Relationship) -> Self {
        Value::Relationship(Relationship::from(value))
    }
}

impl From<native::value::Path> for Value {
    fn from(value: native::value::Path) -> Self {
        Value::Path(Path::from(value))
    }
}

impl From<native::value::UnboundRelationship> for Value {
    fn from(value: native::value::UnboundRelationship) -> Self {
        Value::UnboundRelationship(UnboundRelationship::from(value))
    }
}

impl Marker for Value {
    fn get_marker(&self) -> Result<u8, Error> {
        match self {
            Value::Boolean(boolean) => boolean.get_marker(),
            Value::Integer(integer) => integer.get_marker(),
            Value::Float(float) => float.get_marker(),
            Value::List(list) => list.get_marker(),
            Value::Map(map) => map.get_marker(),
            Value::Null(null) => null.get_marker(),
            Value::String(string) => string.get_marker(),
            Value::Node(node) => node.get_marker(),
            Value::Relationship(rel) => rel.get_marker(),
            Value::Path(path) => path.get_marker(),
            Value::UnboundRelationship(unbound_rel) => unbound_rel.get_marker(),
        }
    }
}

impl Serialize for Value {}

impl TryInto<Bytes> for Value {
    type Error = Error;

    fn try_into(self) -> Result<Bytes, Self::Error> {
        match self {
            Value::Boolean(boolean) => boolean.try_into(),
            Value::Integer(integer) => integer.try_into(),
            Value::Float(float) => float.try_into(),
            Value::List(list) => list.try_into(),
            Value::Map(map) => map.try_into(),
            Value::Null(null) => null.try_into(),
            Value::String(string) => string.try_into(),
            Value::Node(node) => node.try_into(),
            Value::Relationship(rel) => rel.try_into(),
            Value::Path(path) => path.try_into(),
            Value::UnboundRelationship(unbound_rel) => unbound_rel.try_into(),
        }
    }
}

impl Deserialize for Value {}

impl TryFrom<Arc<Mutex<Bytes>>> for Value {
    type Error = Error;

    fn try_from(input_arc: Arc<Mutex<Bytes>>) -> Result<Self, Self::Error> {
        let result: Result<Value, Error> = catch_unwind(move || {
            let marker = input_arc.lock().unwrap().clone().get_u8();

            match marker {
                null::MARKER => {
                    input_arc.lock().unwrap().advance(1);
                    Ok(Value::Null(Null))
                }
                boolean::MARKER_FALSE => {
                    input_arc.lock().unwrap().advance(1);
                    Ok(Value::Boolean(Boolean::from(false)))
                }
                boolean::MARKER_TRUE => {
                    input_arc.lock().unwrap().advance(1);
                    Ok(Value::Boolean(Boolean::from(true)))
                }
                // Tiny int
                marker if (-16..=127).contains(&(marker as i8)) => {
                    input_arc.lock().unwrap().advance(1);
                    Ok(Value::Integer(Integer::from(marker as i8)))
                }
                // Other int types
                integer::MARKER_INT_8
                | integer::MARKER_INT_16
                | integer::MARKER_INT_32
                | integer::MARKER_INT_64 => Ok(Value::Integer(Integer::try_from(input_arc)?)),
                float::MARKER => Ok(Value::Float(Float::try_from(input_arc)?)),
                // Tiny string
                marker
                    if (string::MARKER_TINY..=(string::MARKER_TINY | 0x0F)).contains(&marker) =>
                {
                    Ok(Value::String(String::try_from(input_arc)?))
                }
                string::MARKER_SMALL | string::MARKER_MEDIUM | string::MARKER_LARGE => {
                    Ok(Value::String(String::try_from(input_arc)?))
                }
                // Tiny list
                marker if (list::MARKER_TINY..=(list::MARKER_TINY | 0x0F)).contains(&marker) => {
                    Ok(Value::List(List::try_from(input_arc)?))
                }
                list::MARKER_SMALL | list::MARKER_MEDIUM | list::MARKER_LARGE => {
                    Ok(Value::List(List::try_from(input_arc)?))
                }
                // Tiny map
                marker if (map::MARKER_TINY..=(map::MARKER_TINY | 0x0F)).contains(&marker) => {
                    Ok(Value::Map(Map::try_from(input_arc)?))
                }
                map::MARKER_SMALL | map::MARKER_MEDIUM | map::MARKER_LARGE => {
                    Ok(Value::Map(Map::try_from(input_arc)?))
                }
                // Tiny structure
                marker
                    if (structure::MARKER_TINY..=(list::MARKER_TINY | 0x0F)).contains(&marker) =>
                {
                    deserialize_structure(input_arc)
                }
                structure::MARKER_SMALL | structure::MARKER_MEDIUM => {
                    deserialize_structure(input_arc)
                }
                _ => {
                    return Err(
                        DeserializeError(format!("Invalid marker byte: {:x}", marker)).into(),
                    );
                }
            }
        })
        .map_err(|_| DeserializeError("Panicked during deserialization".to_string()))?;

        Ok(result.map_err(|err: Error| {
            DeserializeError(format!("Error creating Value from Bytes: {}", err))
        })?)
    }
}

// Might panic. Use this inside a catch_unwind block
fn deserialize_structure(input_arc: Arc<Mutex<Bytes>>) -> Result<Value, Error> {
    let signature = get_signature_from_bytes(&mut *input_arc.lock().unwrap())?;
    match signature {
        node::SIGNATURE => Ok(Value::Node(Node::try_from(input_arc)?)),
        relationship::SIGNATURE => Ok(Value::Relationship(Relationship::try_from(input_arc)?)),
        path::SIGNATURE => Ok(Value::Path(Path::try_from(input_arc)?)),
        unbound_relationship::SIGNATURE => Ok(Value::UnboundRelationship(
            UnboundRelationship::try_from(input_arc)?,
        )),
        _ => {
            return Err(
                DeserializeError(format!("Invalid signature byte: {:x}", signature)).into(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::iter::FromIterator;

    use crate::serialize::Serialize;

    use super::*;

    #[test]
    fn null_from_bytes() {
        let null = Null;
        let null_bytes = null.clone().try_into_bytes().unwrap();
        assert_eq!(
            Value::try_from(Arc::new(Mutex::new(null_bytes))).unwrap(),
            Value::Null(null)
        );
    }

    #[test]
    fn boolean_from_bytes() {
        let t = Boolean::from(true);
        let true_bytes = t.clone().try_into_bytes().unwrap();
        let f = Boolean::from(false);
        let false_bytes = f.clone().try_into_bytes().unwrap();
        assert_eq!(
            Value::try_from(Arc::new(Mutex::new(true_bytes))).unwrap(),
            Value::Boolean(t)
        );
        assert_eq!(
            Value::try_from(Arc::new(Mutex::new(false_bytes))).unwrap(),
            Value::Boolean(f)
        );
    }

    #[test]
    fn integer_from_bytes() {
        let tiny = Integer::from(110_i8);
        let tiny_bytes = tiny.clone().try_into_bytes().unwrap();
        let small = Integer::from(-50_i8);
        let small_bytes = small.clone().try_into_bytes().unwrap();
        let medium = Integer::from(8000_i16);
        let medium_bytes = medium.clone().try_into_bytes().unwrap();
        let large = Integer::from(-1_000_000_000_i32);
        let large_bytes = large.clone().try_into_bytes().unwrap();
        let very_large = Integer::from(9_000_000_000_000_000_000_i64);
        let very_large_bytes = very_large.clone().try_into_bytes().unwrap();
        assert_eq!(
            Value::try_from(Arc::new(Mutex::new(tiny_bytes))).unwrap(),
            Value::Integer(tiny)
        );
        assert_eq!(
            Value::try_from(Arc::new(Mutex::new(small_bytes))).unwrap(),
            Value::Integer(small)
        );
        assert_eq!(
            Value::try_from(Arc::new(Mutex::new(medium_bytes))).unwrap(),
            Value::Integer(medium)
        );
        assert_eq!(
            Value::try_from(Arc::new(Mutex::new(large_bytes))).unwrap(),
            Value::Integer(large)
        );
        assert_eq!(
            Value::try_from(Arc::new(Mutex::new(very_large_bytes))).unwrap(),
            Value::Integer(very_large)
        );
    }

    #[test]
    fn float_from_bytes() {
        let min = Float::from(std::f64::MIN_POSITIVE);
        let min_bytes = min.clone().try_into_bytes().unwrap();
        let max = Float::from(std::f64::MAX);
        let max_bytes = max.clone().try_into_bytes().unwrap();
        let e = Float::from(std::f64::consts::E);
        let e_bytes = e.clone().try_into_bytes().unwrap();
        let pi = Float::from(std::f64::consts::PI);
        let pi_bytes = pi.clone().try_into_bytes().unwrap();
        assert_eq!(
            Value::try_from(Arc::new(Mutex::new(min_bytes))).unwrap(),
            Value::Float(min)
        );
        assert_eq!(
            Value::try_from(Arc::new(Mutex::new(max_bytes))).unwrap(),
            Value::Float(max)
        );
        assert_eq!(
            Value::try_from(Arc::new(Mutex::new(e_bytes))).unwrap(),
            Value::Float(e)
        );
        assert_eq!(
            Value::try_from(Arc::new(Mutex::new(pi_bytes))).unwrap(),
            Value::Float(pi)
        );
    }

    #[test]
    fn string_from_bytes() {
        let tiny = String::from("string".repeat(1));
        let tiny_bytes = tiny.clone().try_into_bytes().unwrap();
        let small = String::from("string".repeat(10));
        let small_bytes = small.clone().try_into_bytes().unwrap();
        let medium = String::from("string".repeat(1000));
        let medium_bytes = medium.clone().try_into_bytes().unwrap();
        let large = String::from("string".repeat(100_000));
        let large_bytes = large.clone().try_into_bytes().unwrap();
        assert_eq!(
            Value::try_from(Arc::new(Mutex::new(tiny_bytes))).unwrap(),
            Value::String(tiny)
        );
        assert_eq!(
            Value::try_from(Arc::new(Mutex::new(small_bytes))).unwrap(),
            Value::String(small)
        );
        assert_eq!(
            Value::try_from(Arc::new(Mutex::new(medium_bytes))).unwrap(),
            Value::String(medium)
        );
        assert_eq!(
            Value::try_from(Arc::new(Mutex::new(large_bytes))).unwrap(),
            Value::String(large)
        );
    }

    #[test]
    fn list_from_bytes() {
        let empty_list: List = Vec::<i32>::new().into();
        let empty_list_bytes = empty_list.clone().try_into_bytes().unwrap();
        let tiny_list: List = vec![100_000_000_000_i64; 10].into();
        let tiny_list_bytes = tiny_list.clone().try_into_bytes().unwrap();
        let small_list: List = vec!["item"; 100].into();
        let small_list_bytes = small_list.clone().try_into_bytes().unwrap();
        let medium_list: List = vec![false; 1000].into();
        let medium_list_bytes = medium_list.clone().try_into_bytes().unwrap();
        assert_eq!(
            Value::try_from(Arc::new(Mutex::new(empty_list_bytes))).unwrap(),
            Value::List(empty_list)
        );
        assert_eq!(
            Value::try_from(Arc::new(Mutex::new(tiny_list_bytes))).unwrap(),
            Value::List(tiny_list)
        );
        assert_eq!(
            Value::try_from(Arc::new(Mutex::new(small_list_bytes))).unwrap(),
            Value::List(small_list)
        );
        assert_eq!(
            Value::try_from(Arc::new(Mutex::new(medium_list_bytes))).unwrap(),
            Value::List(medium_list)
        );
    }

    #[test]
    #[ignore]
    fn large_list_from_bytes() {
        let large_list: List = vec![1_i8; 70_000].into();
        let large_list_bytes = large_list.clone().try_into_bytes().unwrap();
        assert_eq!(
            Value::try_from(Arc::new(Mutex::new(large_list_bytes))).unwrap(),
            Value::List(large_list)
        );
    }

    #[test]
    fn map_from_bytes() {
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
            Value::try_from(Arc::new(Mutex::new(empty_map_bytes))).unwrap(),
            Value::Map(empty_map)
        );
        assert_eq!(
            Value::try_from(Arc::new(Mutex::new(tiny_map_bytes))).unwrap(),
            Value::Map(tiny_map)
        );
        assert_eq!(
            Value::try_from(Arc::new(Mutex::new(small_map_bytes))).unwrap(),
            Value::Map(small_map)
        );
    }

    #[test]
    fn node_from_bytes() {
        todo!()
    }

    #[test]
    fn relationship_from_bytes() {
        todo!()
    }

    #[test]
    fn path_from_bytes() {
        todo!()
    }

    #[test]
    fn unbound_relationship_from_bytes() {
        todo!()
    }
}
