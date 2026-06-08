//! Item component hashing for the 1.21+ HashedSlot format. Re-serializes a
//! component's data with the protocol codec, then computes Java's
//! `Arrays.hashCode(byte[])` over the bytes.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde_json::Value;

use super::codec::{write_seeded, TypeRegistry};
use super::value::PValue;

struct ComponentCodec {
    registry: TypeRegistry,
    serializers: HashMap<String, Value>,
}

fn component_codec() -> &'static ComponentCodec {
    static CODEC: OnceLock<ComponentCodec> = OnceLock::new();
    CODEC.get_or_init(|| {
        let registry = TypeRegistry::new(super::shared_types_map());
        let mut serializers = HashMap::new();

        // SlotComponent = ["container", [ {name:"type",...},
        //   {name:"data", type:["switch", {compareTo:"type", fields:{...}}]} ]]
        if let Some(schema) = super::shared_type("SlotComponent") {
            if let Some(fields) = schema.get(1).and_then(Value::as_array) {
                if let Some(data_field) = fields
                    .iter()
                    .find(|f| f["name"] == Value::String("data".into()))
                {
                    if let Some(map) = data_field["type"][1]["fields"].as_object() {
                        for (name, ty) in map {
                            serializers.insert(name.clone(), ty.clone());
                        }
                    }
                }
            }
        }

        ComponentCodec {
            registry,
            serializers,
        }
    })
}

/// Java's `Arrays.hashCode(byte[])` — bytes treated as signed.
pub fn java_arrays_hashcode(buf: &[u8]) -> i32 {
    let mut result: i32 = 1;
    for &b in buf {
        result = result.wrapping_mul(31).wrapping_add(b as i8 as i32);
    }
    result
}

/// Serialize a component's data to bytes using the protocol codec.
pub fn serialize_component_data(component_type: &str, data: &PValue) -> Vec<u8> {
    let codec = component_codec();
    let Some(schema) = codec.serializers.get(component_type) else {
        return Vec::new(); // unknown component type → empty (hash = 1)
    };
    let seed = [("type".to_string(), PValue::str(component_type))];
    let mut out = Vec::new();
    let _ = write_seeded(&codec.registry, schema, data, &mut out, &seed);
    out
}

/// Java-compatible hash for a single component.
pub fn hash_component_data(component_type: &str, data: &PValue) -> i32 {
    java_arrays_hashcode(&serialize_component_data(component_type, data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_buffer_hashes_to_one() {
        assert_eq!(java_arrays_hashcode(&[]), 1);
    }

    #[test]
    fn known_hashcode() {
        // Java Arrays.hashCode([1,2,3]) = 30817
        assert_eq!(java_arrays_hashcode(&[1, 2, 3]), 30817);
        // signed bytes: [0xff] => -1 => 31*1 + (-1) = 30
        assert_eq!(java_arrays_hashcode(&[0xff]), 30);
    }

    #[test]
    fn unknown_component_is_empty() {
        assert_eq!(
            serialize_component_data("not_a_component", &PValue::Void),
            Vec::<u8>::new()
        );
        assert_eq!(hash_component_data("not_a_component", &PValue::Void), 1);
    }
}
