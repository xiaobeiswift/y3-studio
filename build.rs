use std::fs;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=target/installer-payload.zip");
    let payload_path = Path::new("target/installer-payload.zip");
    if !payload_path.exists() {
        if let Some(parent) = payload_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        // 标准空 ZIP 归档头 (End of Central Directory record, 22 bytes)
        let empty_zip: [u8; 22] = [
            0x50, 0x4b, 0x05, 0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        let _ = fs::write(payload_path, empty_zip);
    }
}
