use crate::error::{EngineError, Result};

pub fn encode_u32_le(val: u32, buf: &mut Vec<u8>) {
    let bytes: [u8; 4] = val.to_le_bytes();
    buf.extend_from_slice(&bytes);
}

pub fn decode_u32_le(src: &[u8]) -> Result<(u32, &[u8])> {
    if src.len() < 4 {
        return Err(EngineError::UnexpectedEof);
    }

    let bytes: [u8; 4] = src[0..4].try_into().unwrap();
    let val = u32::from_le_bytes(bytes);

    Ok((val, &src[4..]))
}

pub fn encode_bytes(data: &[u8], buf: &mut Vec<u8>) {
    let len = data.len() as u32;
    encode_u32_le(len, buf);
    buf.extend_from_slice(data);
}

pub fn decode_bytes(src: &[u8]) -> Result<(&[u8], &[u8])> {
    let (len, rest) = decode_u32_le(src)?;
    let len = len as usize;

    if rest.len() < len {
        return Err(EngineError::UnexpectedEof);
    }

    Ok((&rest[..len], &rest[len..]))

}



#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_u32_roundtrip() {
        let mut buf = Vec::new();
        encode_u32_le(1337, &mut buf);
        assert_eq!(buf.len(), 4);

        let (val, remaining) = decode_u32_le(&buf).unwrap();
        assert_eq!(val, 1337);
        assert_eq!(remaining.len(), 0);
    }

    #[test]
    fn test_u32_incomplete() {
        let truncated = [0x01, 0x02, 0x03]; // only 3 bytes
        let result = decode_u32_le(&truncated);
        assert!(matches!(result, Err(EngineError::UnexpectedEof)));
    }

    #[test]
    fn test_bytes_roundtrip() {
        let mut buf = Vec::new();
        encode_bytes(b"hello world", &mut buf);

        let (payload, remaining) = decode_bytes(&buf).unwrap();
        assert_eq!(payload, b"hello world");
        assert_eq!(remaining.len(), 0);
    }
}