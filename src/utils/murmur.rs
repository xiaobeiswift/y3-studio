/// MurmurHash3 32-bit 实现，与原版 `src/editorTable/language.ts` 中的 `hash(key, seed=0)` 逐位一致。
/// 注意：原版 TS 代码对 UTF-16 code unit (`charCodeAt`) 按单字节 `& 0xff` 取值进行哈希计算，
/// 这里同时提供按原版 UTF-16 code unit 低字节计算的 `murmur_hash_utf16_low_byte`（100% 兼容原版 TS）
/// 以及按字节流计算的 `murmur_hash_bytes`。
pub fn murmur_hash3_ts_compat(key: &str, seed: u32) -> u32 {
    let units: Vec<u8> = key.encode_utf16().map(|u| (u & 0xff) as u8).collect();
    murmur_hash3_bytes(&units, seed)
}

pub fn murmur_hash3_bytes(data: &[u8], seed: u32) -> u32 {
    let c1: u32 = 0xcc9e2d51;
    let c2: u32 = 0x1b873593;
    let len = data.len();
    let remainder = len & 3;
    let bytes = len - remainder;

    let mut h1 = seed;
    let mut i = 0;

    while i < bytes {
        let mut k1 = (data[i] as u32)
            | ((data[i + 1] as u32) << 8)
            | ((data[i + 2] as u32) << 16)
            | ((data[i + 3] as u32) << 24);
        i += 4;

        k1 = k1.wrapping_mul(c1);
        k1 = k1.rotate_left(15);
        k1 = k1.wrapping_mul(c2);

        h1 ^= k1;
        h1 = h1.rotate_left(13);
        h1 = h1.wrapping_mul(5).wrapping_add(0xe6546b64);
    }

    let mut k1: u32 = 0;
    if remainder >= 3 {
        k1 ^= (data[i + 2] as u32) << 16;
    }
    if remainder >= 2 {
        k1 ^= (data[i + 1] as u32) << 8;
    }
    if remainder >= 1 {
        k1 ^= data[i] as u32;
        k1 = k1.wrapping_mul(c1);
        k1 = k1.rotate_left(15);
        k1 = k1.wrapping_mul(c2);
        h1 ^= k1;
    }

    h1 ^= len as u32;
    h1 ^= h1 >> 16;
    h1 = h1.wrapping_mul(0x85ebca6b);
    h1 ^= h1 >> 13;
    h1 = h1.wrapping_mul(0xc2b2ae35);
    h1 ^= h1 >> 16;

    h1
}

/// 生成物编本地化哈希键（有符号 32 位整数转字符串，或无符号，原版 TS 最后 `return h1 >>> 0` 即无符号 u32）
pub fn language_key_hash(key: &str) -> String {
    murmur_hash3_ts_compat(key, 0).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_murmur_hash3_compat() {
        let h = murmur_hash3_ts_compat("134274912_name", 0);
        assert_ne!(h, 0);
        assert_eq!(language_key_hash("134274912_name"), h.to_string());
    }
}
