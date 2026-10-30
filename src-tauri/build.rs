fn main() {
    tauri_build::build();
    english_texts();
}

/// ui/lang/en.json is in the exe as one of the UI's files, and the Rust side needs it as well (its
/// own texts, the fallback for other languages): that second copy goes in compressed, unpacked once
/// when DropLaunch starts (lang::builtin_text).
fn english_texts() {
    const SOURCE: &str = "../ui/lang/en.json";
    println!("cargo:rerun-if-changed={SOURCE}");
    let text = std::fs::read(SOURCE).expect("ui/lang/en.json");
    let params = brotli::enc::BrotliEncoderParams { quality: 11, lgwin: 22, ..Default::default() };
    let mut packed = Vec::new();
    brotli::BrotliCompress(&mut &text[..], &mut packed, &params).expect("compressing en.json");
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("en.json.br");
    std::fs::write(out, packed).expect("writing en.json.br");
}
