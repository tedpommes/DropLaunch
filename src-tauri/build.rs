fn main() {
    // The Windows manifest comes from the linker (see `manifest`), not from tauri-build, which
    // only gives it to DropLaunch.exe.
    let windows = tauri_build::WindowsAttributes::new_without_app_manifest();
    if let Err(e) = tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows)) {
        panic!("tauri-build: {e:#}");
    }
    english_texts();
    manifest();
}

/// The Windows manifest (Common Controls 6, which Tauri's dialogs need) in every exe: DropLaunch
/// and the test exes. A test exe without it doesn't even load once a test reaches the server
/// manager (STATUS_ENTRYPOINT_NOT_FOUND: TaskDialogIndirect is only in version 6), and
/// `rustc-link-arg-tests` doesn't reach the library's own tests, so it's for all of them here.
fn manifest() {
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
        return;
    }
    const MANIFEST: &str = r#"<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*" />
    </dependentAssembly>
  </dependency>
</assembly>
"#;
    let file = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("droplaunch.manifest");
    std::fs::write(&file, MANIFEST).expect("writing droplaunch.manifest");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", file.display());
    // Only that: no UAC part (as before, it runs as the user who started it).
    println!("cargo:rustc-link-arg=/MANIFESTUAC:NO");
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
