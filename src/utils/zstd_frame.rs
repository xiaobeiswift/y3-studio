use std::io::Read;

/// Zstandard Magic Number (0xFD2FB528 in Little-Endian)
const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];
/// Zstandard Maximum Block Size (128 KiB)
const MAX_BLOCK_SIZE: usize = 128 * 1024;

/// 纯 Rust 生成符合 RFC 8878 规范的标准 Zstandard Frame：
/// - 包含完整的 Frame Header（`Single_Segment_Flag = 1` + 精确的 `Frame_Content_Size`）
/// - 对全同字节分块自动使用 `RLE_Block` (`Block_Type = 1`) 压缩
/// - 对一般数据分块使用 `Raw_Block` (`Block_Type = 0`) 封装（100% 符合 RFC 8878 标准，所有 Zstd 解码器如 libzstd / Python zstd / ruzstd 均可无损解码）
pub fn compress_zstd(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 32);
    out.extend_from_slice(&ZSTD_MAGIC);

    // Frame_Header_Descriptor:
    // Single_Segment_Flag (bit 5) = 1
    // Frame_Content_Size_Flag (bits 7..6):
    //   0 => 1 byte (0..=255)
    //   1 => 2 bytes (256..=65791, stored as size - 256)
    //   2 => 4 bytes (0..=4GB-1)
    //   3 => 8 bytes
    let len = data.len();
    if len <= 255 {
        let fhd: u8 = 0b0010_0000; // FCS_Field_Size = 1, Single_Segment = 1
        out.push(fhd);
        out.push(len as u8);
    } else if len <= 65535 + 256 {
        let fhd: u8 = 0b0110_0000; // FCS_Field_Size = 2, Single_Segment = 1
        out.push(fhd);
        let fcs = (len - 256) as u16;
        out.extend_from_slice(&fcs.to_le_bytes());
    } else if (len as u64) <= u32::MAX as u64 {
        let fhd: u8 = 0b1010_0000; // FCS_Field_Size = 4, Single_Segment = 1
        out.push(fhd);
        let fcs = len as u32;
        out.extend_from_slice(&fcs.to_le_bytes());
    } else {
        let fhd: u8 = 0b1110_0000; // FCS_Field_Size = 8, Single_Segment = 1
        out.push(fhd);
        let fcs = len as u64;
        out.extend_from_slice(&fcs.to_le_bytes());
    }

    if data.is_empty() {
        // Last_Block = 1, Block_Type = 0 (Raw_Block), Block_Size = 0
        out.extend_from_slice(&[0x01, 0x00, 0x00]);
        return out;
    }

    let chunks: Vec<&[u8]> = data.chunks(MAX_BLOCK_SIZE).collect();
    for (idx, chunk) in chunks.iter().enumerate() {
        let is_last = idx + 1 == chunks.len();
        let last_bit: u32 = if is_last { 1 } else { 0 };
        let chunk_len = chunk.len() as u32;

        // 检查是否全为相同字节（可用 RLE_Block 压缩）
        let first_byte = chunk[0];
        let is_rle = chunk.len() > 4 && chunk.iter().all(|&b| b == first_byte);

        if is_rle {
            // Block_Type = 1 (RLE_Block)
            let header: u32 = last_bit | (1 << 1) | (chunk_len << 3);
            let h_bytes = header.to_le_bytes();
            out.extend_from_slice(&h_bytes[0..3]);
            out.push(first_byte);
        } else {
            // Block_Type = 0 (Raw_Block)
            let header: u32 = last_bit | (chunk_len << 3);
            let h_bytes = header.to_le_bytes();
            out.extend_from_slice(&h_bytes[0..3]);
            out.extend_from_slice(chunk);
        }
    }

    out
}

/// 纯 Rust 解压任意标准 Zstandard 数据（支持所有压缩级别与块类型）
pub fn decompress_zstd(compressed: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut decoder = ruzstd::StreamingDecoder::new(compressed)
        .map_err(|e| anyhow::anyhow!("Zstd decoder init failed: {}", e))?;
    let mut out = Vec::new();
    decoder
        .read_to_end(&mut out)
        .map_err(|e| anyhow::anyhow!("Zstd decompress failed: {}", e))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zstd_compress_decompress_roundtrip() {
        let samples: &[&[u8]] = &[
            b"",
            b"hello world y3-helper rust!",
            &[0xAA; 5000],
            &vec![42u8; 200_000],
        ];
        for sample in samples {
            let compressed = compress_zstd(sample);
            let decompressed = decompress_zstd(&compressed).unwrap();
            assert_eq!(&decompressed, sample);
        }
    }
}
