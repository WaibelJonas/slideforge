//! Writes tile records to the `.tfrecord` format used by TensorFlow
//!
//! A `.tfrecord` file is a sequence of byte records (see [`TfRecordWriter`]).
//! Each record's payload is itself a serialized protobuf message
//! (inspired by the `tensorflow.train.Example` proto).
//!
//! A `.tfrecord` contains the following fields:
//! - The length of the byte payload
//! - The masked CRC32C hash of the length
//! - The byte data payload
//! - The masked CRC32C hash of the byte data payload
//!
//! For each tile, `tile_record` assembles the four features Slideflow's own
//! writes (`slide`, `image_raw`, `loc_x`, `loc_y`) to the [`TfRecordWriter`]
//!
//! # Sources
//! - <https://www.tensorflow.org/tutorials/load_data/tfrecord#tfrecords_format_details>
//! - <https://en.wikipedia.org/wiki/Cyclic_redundancy_check#CRC-32_algorithm>
//! - <https://protobuf.dev/programming-guides/encoding/>
use crc32c::crc32c;
use std::io::Write;

use crate::error::WsiError;

/// Computes the masked CRC32C for `bytes`, as required by the `.tfrecord`
/// format.
///
/// The masked CRC32C is computed as follows (where crc is the unmasked CRC32C): \
/// `((crc >> 15) | (crc << 17)) + 0xa282ead8`
///
/// Source: <https://www.tensorflow.org/tutorials/load_data/tfrecord#tfrecords_format_details>
///
/// # Arguments
/// * `bytes` - The raw byte data to compute the masked CRC32C for.
///
/// # Returns
/// The masked CRC32C of `bytes` as a [`u32`].
fn masked_crc32c(bytes: &[u8]) -> u32 {
    let crc = crc32c(bytes);
    crc.rotate_right(15).wrapping_add(0xa282ead8)
}

/// Writes a stream of TFRecord-framed records to `writer`.
///
/// Wraps any [`Write`] destination (ideally this should be a [`std::fs::File`]).
pub struct TfRecordWriter<W: Write> {
    writer: W,
}

impl<W: Write> TfRecordWriter<W> {
    pub fn new(writer: W) -> Self {
        Self { writer }
    }

    /// Writes one record, framed as the `.tfrecord` format requires:
    ///
    /// ```text
    /// u64    length
    /// u32    masked_crc32c(length)
    /// byte   data[length]
    /// u32    masked_crc32c(data)   
    /// ```
    pub fn write_record(&mut self, data: &[u8]) -> Result<(), WsiError> {
        let length = (data.len() as u64).to_le_bytes();
        let masked_crc_length = masked_crc32c(&length).to_le_bytes();
        let masked_crc_data = masked_crc32c(data).to_le_bytes();

        self.writer.write_all(&length)?;
        self.writer.write_all(&masked_crc_length)?;
        self.writer.write_all(data)?;
        self.writer.write_all(&masked_crc_data)?;
        Ok(())
    }
}

/// Appends `value` to `buf`- the value is split into 7-bit groups.
fn write_varint(buf: &mut Vec<u8>, mut value: u64) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            buf.push(byte);
            break;
        }
        buf.push(byte | 0x80);
    }
}

/// Appends a length-delimited field to `buf`: a tag identifying `field` as
/// wire type 2, followed by `payload`'s length and then `payload` itself.
///
/// As per the Protobuf standard, length-delimited data (like raw bytes,
/// embedded messages, strings) only ever has the wire type 2. The tag is
/// computed from `field` and wire type as follows: \
/// `(field << 3) | wire_type = (field << 3) | 2`
///
/// # Sources
/// - <https://protobuf.dev/programming-guides/encoding/>
fn write_length_delimited(buf: &mut Vec<u8>, field: u32, payload: &[u8]) {
    write_varint(buf, ((field as u64) << 3) | 2); // wire type 2 = length-delimited
    write_varint(buf, payload.len() as u64);
    buf.extend_from_slice(payload);
}

/// Encodes a `BytesList` message. `value` is assigned to field number 1
/// and encoded as a length-delimited message
fn encode_bytes_list(value: &[u8]) -> Vec<u8> {
    let mut buf = Vec::new();
    write_length_delimited(&mut buf, 1, value);
    buf
}

/// Encodes a `Int64List` message. `value` is assigned to field number 1
/// and encoded as a length-delimited message
fn encode_int64_list(values: &[i64]) -> Vec<u8> {
    let mut packed = Vec::new();
    for &v in values {
        write_varint(&mut packed, v as u64); // plain varint of the bit pattern
    }
    let mut buf = Vec::new();
    write_length_delimited(&mut buf, 1, &packed); // Int64List.value = 1, packed
    buf
}

/// The two branches of `Feature`'s `oneof kind` this crate actually needs.
/// `feature.proto` also declares `float_list = 2`, deliberately unused
/// here -- nothing in [`tile_record`]'s schema needs it.
pub(crate) enum FeatureValue {
    Bytes(Vec<u8>),
    Int64s(Vec<i64>),
}

/// Encodes a `Feature` message from the given `oneof` variant.
fn encode_feature(value: &FeatureValue) -> Vec<u8> {
    let mut buf = Vec::new();
    match value {
        FeatureValue::Bytes(b) => {
            let encoded = encode_bytes_list(b);
            write_length_delimited(&mut buf, 1, &encoded) // Feature.bytes_list = 1
        }
        FeatureValue::Int64s(i) => {
            let encoded = encode_int64_list(i);
            write_length_delimited(&mut buf, 3, &encoded); // Feature.int64_list = 3
        }
    };
    buf
}

/// Encodes one `Features.feature` map entry for `(key, value)`.
fn encode_feature_map_entry(key: &str, value: &FeatureValue) -> Vec<u8> {
    let mut entry = Vec::new();
    write_length_delimited(&mut entry, 1, key.as_bytes()); // MapEntry.key = 1
    write_length_delimited(&mut entry, 2, &encode_feature(value)); // MapEntry.value = 2

    let mut buf = Vec::new();
    write_length_delimited(&mut buf, 1, &entry);
    buf
}

/// Encodes a full `Example { features: Features { feature: {...} } }`
/// message from a list of `(name, value)` feature pairs.
pub(crate) fn encode_example(fields: &[(&str, FeatureValue)]) -> Vec<u8> {
    let mut features = Vec::new();
    for (key, value) in fields {
        features.extend_from_slice(&encode_feature_map_entry(key, value));
    }
    let mut example = Vec::new();
    write_length_delimited(&mut example, 1, &features); // Example.features = 1
    example
}

/// Builds the serialized `tf.train.Example` proto for one extracted tile.
///
/// encodes four features (`slide`, `image_raw`, `loc_x`, `loc_y`) for each tile.
pub(crate) fn tile_record(slide_name: &str, bytes: Vec<u8>, loc_x: i64, loc_y: i64) -> Vec<u8> {
    encode_example(&[
        ("slide", FeatureValue::Bytes(slide_name.as_bytes().to_vec())),
        ("image_raw", FeatureValue::Bytes(bytes)),
        ("loc_x", FeatureValue::Int64s(vec![loc_x])),
        ("loc_y", FeatureValue::Int64s(vec![loc_y])),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Inverse of the masking step, transcribed from the same authoritative
    /// source as `masked_crc32c` itself (`Unmask` in TensorFlow's
    /// `crc32c.h`): `rot = masked - kMaskDelta; (rot >> 17) | (rot << 15)`.
    /// Exists only so `masked_crc32c` can be checked against it below --
    /// there's no public need for it beyond this test.
    fn unmask(masked: u32) -> u32 {
        let rot = masked.wrapping_sub(0xa282ead8);
        rot.rotate_left(15)
    }

    #[test]
    fn crc32c_matches_standard_check_value() {
        // The official CRC32C (Castagnoli) check value for "123456789",
        // cited by essentially every independent implementation.
        assert_eq!(crc32c(b"123456789"), 0xE3069283);
    }

    #[test]
    fn masked_crc32c_round_trips_through_unmask() {
        let inputs: &[&[u8]] = &[
            b"",
            b"123456789",
            // Chosen because its crc happens to make `shift_or + kMaskDelta`
            // exceed u32::MAX: a regression test for a real bug where this
            // function used a plain `+` instead of `wrapping_add`, which
            // panicked here in debug builds (masking is defined mod 2^32,
            // matching C++'s `uint32_t` wraparound).
            b"hello world",
            b"a longer string that will produce some arbitrary crc value",
        ];

        for data in inputs {
            let crc = crc32c(data);
            let masked = masked_crc32c(data);
            assert_eq!(unmask(masked), crc, "round-trip failed for {data:?}");
        }
    }
}
