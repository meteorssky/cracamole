fn main() {
    // GetWindowSubclass requires comctl32 v6 before Windows resolves imports.
    if std::env::var("TARGET").is_ok_and(|target| target.ends_with("windows-msvc")) {
        println!("cargo:rustc-link-arg-bin=cracamole-desktop=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg-bin=cracamole-desktop=/MANIFESTDEPENDENCY:type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'");
    }
}
