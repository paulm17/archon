//! Codec — VardaDB key encoding (standalone copy from VardaDB/src/storage/codec.rs)

use byteorder::{BigEndian, WriteBytesExt};

pub struct Codec;

impl Codec {
    /// Data key: [0x01][UID:8 BE][field_name]
    pub fn encode_data_key(uid: u64, predicate: &str) -> Vec<u8> {
        let mut buf = Vec::with_capacity(1 + 8 + predicate.len());
        buf.push(0x01);
        buf.write_u64::<BigEndian>(uid).unwrap();
        buf.extend_from_slice(predicate.as_bytes());
        buf
    }

    /// Type index key: [0x03][type_name][0x00][UID:8 BE]
    pub fn encode_type_index_key(type_name: &str, uid: u64) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.push(0x03);
        buf.extend_from_slice(type_name.as_bytes());
        buf.push(0x00);
        buf.write_u64::<BigEndian>(uid).unwrap();
        buf
    }

    /// Unique index key: [0x02][Type.field][0x00][json_value]
    pub fn encode_unique_index_key(predicate: &str, value: &str) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.push(0x02);
        buf.extend_from_slice(predicate.as_bytes());
        buf.push(0x00);
        buf.extend_from_slice(value.as_bytes());
        buf
    }

    /// Edge key (list inverse): [0x07][target_uid:8][field][0x00][source_uid:8]
    pub fn encode_edge_key(target_uid: u64, field: &str, source_uid: u64) -> Vec<u8> {
        let mut buf = Vec::with_capacity(1 + 8 + field.len() + 1 + 8);
        buf.push(0x07);
        buf.write_u64::<BigEndian>(target_uid).unwrap();
        buf.extend_from_slice(field.as_bytes());
        buf.push(0x00);
        buf.write_u64::<BigEndian>(source_uid).unwrap();
        buf
    }

    /// Reverse edge key: [0x08][source_uid:8][target_uid:8][field][0x00]
    pub fn encode_reverse_edge_key(source_uid: u64, target_uid: u64, field: &str) -> Vec<u8> {
        let mut buf = Vec::with_capacity(1 + 8 + 8 + field.len() + 1);
        buf.push(0x08);
        buf.write_u64::<BigEndian>(source_uid).unwrap();
        buf.write_u64::<BigEndian>(target_uid).unwrap();
        buf.extend_from_slice(field.as_bytes());
        buf.push(0x00);
        buf
    }

    /// Encode f64 as 8 sortable bytes (sign bit flipped for correct sort order).
    fn encode_sortable_f64(value: f64) -> [u8; 8] {
        let bits = value.to_bits();
        let sortable = if bits & (1 << 63) != 0 {
            !bits
        } else {
            bits ^ (1 << 63)
        };
        sortable.to_be_bytes()
    }

    /// Encode a JSON value for the order index. Returns None for non-scalar types.
    pub fn encode_order_index_value(
        value: &serde_json::Value,
        descending: bool,
    ) -> Option<Vec<u8>> {
        let mut encoded = match value {
            serde_json::Value::String(s) => s.as_bytes().to_vec(),
            serde_json::Value::Number(n) => {
                let numeric = if let Some(i) = n.as_i64() {
                    i as f64
                } else if let Some(u) = n.as_u64() {
                    u as f64
                } else {
                    n.as_f64()?
                };
                Self::encode_sortable_f64(numeric).to_vec()
            }
            serde_json::Value::Bool(b) => vec![u8::from(*b)],
            _ => return None,
        };

        if descending {
            for byte in &mut encoded {
                *byte = !*byte;
            }
        }

        Some(encoded)
    }

    /// Order index key: [0x09][type][0x00][field][0x00][dir][value][0x00][uid]
    pub fn encode_order_index_key(
        type_name: &str,
        field: &str,
        descending: bool,
        encoded_value: &[u8],
        uid: u64,
    ) -> Vec<u8> {
        let mut buf =
            Vec::with_capacity(1 + type_name.len() + field.len() + encoded_value.len() + 11);
        buf.push(0x09);
        buf.extend_from_slice(type_name.as_bytes());
        buf.push(0x00);
        buf.extend_from_slice(field.as_bytes());
        buf.push(0x00);
        buf.push(if descending { 1 } else { 0 });
        buf.extend_from_slice(encoded_value);
        buf.push(0x00);
        buf.write_u64::<BigEndian>(uid).unwrap();
        buf
    }
}
