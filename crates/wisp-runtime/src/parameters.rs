use wisp_format::ParameterValue;

/// ABI 2: one aligned 16-byte slot per parameter; unused lanes are zero.
pub(crate) fn encode(value: &ParameterValue) -> [u8; 16] {
    use ParameterValue::*;
    let mut bytes = [0; 16];
    let floats: &[f32] = match value {
        Float(v) => std::slice::from_ref(v),
        Color(v) | Vector4(v) => v,
        Vector2(v) => v,
        Vector3(v) => v,
        Integer(v) => {
            bytes[..4].copy_from_slice(&v.to_le_bytes());
            return bytes;
        }
        Boolean(v) => {
            bytes[..4].copy_from_slice(&u32::from(*v).to_le_bytes());
            return bytes;
        }
    };
    for (lane, value) in bytes.chunks_exact_mut(4).zip(floats) {
        lane.copy_from_slice(&value.to_le_bytes());
    }
    bytes
}
