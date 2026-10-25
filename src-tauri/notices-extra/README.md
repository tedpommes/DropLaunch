License files for libraries whose published package leaves them out, taken from the project's
own repository (`examples/notices.rs` adds them to the package's own files in `ui/licenses.json`):

- `selectors`: MPL-2.0, the standard text (servo's packages share it; this copy is cssparser's).
- `webview2-com`, `webview2-com-sys`: wravery/webview2-rs, `LICENSE`.
- `webview2-com-sys` also links Microsoft's WebView2 loader (`WebView2LoaderStatic.lib` from the
  WebView2 SDK) into DropLaunch: `LICENSE-WebView2-SDK` is the SDK's license (NuGet package
  Microsoft.Web.WebView2 1.0.3650.58).

When `cargo run --example notices` lists a library "without license files of their own", or the
`every_library_comes_with_its_license_text` test names one, add a folder named after it here
with the files from its repository.
