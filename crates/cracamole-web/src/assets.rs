//! Static front-end files compiled into the binary.
//!
//! One table instead of one route per file. `every_web_file_is_served` fails when a file is added
//! under `web/` without being listed here.

pub struct Asset {
    /// Path below `/assets/`.
    pub path: &'static str,
    pub mime: &'static str,
    pub body: &'static [u8],
}

macro_rules! asset {
    ($path:literal, $mime:expr, $file:literal) => {
        Asset {
            path: $path,
            mime: $mime,
            body: include_bytes!(concat!("../../../web/", $file)),
        }
    };
}

pub const INDEX: &[u8] = include_bytes!("../../../web/index.html");

const CSS: &str = "text/css; charset=utf-8";
const JS: &str = "text/javascript; charset=utf-8";
const TEXT: &str = "text/plain; charset=utf-8";

pub static ASSETS: &[Asset] = &[
    asset!("css/tokens.css", CSS, "css/tokens.css"),
    asset!("css/base.css", CSS, "css/base.css"),
    asset!("css/layout.css", CSS, "css/layout.css"),
    asset!(
        "css/components/button.css",
        CSS,
        "css/components/button.css"
    ),
    asset!("css/components/form.css", CSS, "css/components/form.css"),
    asset!("css/components/badge.css", CSS, "css/components/badge.css"),
    asset!("css/components/card.css", CSS, "css/components/card.css"),
    asset!(
        "css/components/dialog.css",
        CSS,
        "css/components/dialog.css"
    ),
    asset!(
        "css/components/overlay.css",
        CSS,
        "css/components/overlay.css"
    ),
    asset!("css/components/toast.css", CSS, "css/components/toast.css"),
    asset!(
        "css/components/keybar.css",
        CSS,
        "css/components/keybar.css"
    ),
    asset!(
        "css/components/banner.css",
        CSS,
        "css/components/banner.css"
    ),
    asset!("js/main.js", JS, "js/main.js"),
    asset!("js/api.js", JS, "js/api.js"),
    asset!("js/state.js", JS, "js/state.js"),
    asset!("js/util/dom.js", JS, "js/util/dom.js"),
    asset!("js/util/format.js", JS, "js/util/format.js"),
    asset!("js/util/protocols.js", JS, "js/util/protocols.js"),
    asset!("js/util/theme.js", JS, "js/util/theme.js"),
    asset!("js/ui/toast.js", JS, "js/ui/toast.js"),
    asset!("js/ui/dialog.js", JS, "js/ui/dialog.js"),
    asset!("js/session/guac.js", JS, "js/session/guac.js"),
    asset!("js/session/keys.js", JS, "js/session/keys.js"),
    asset!("js/session/reasons.js", JS, "js/session/reasons.js"),
    asset!("js/views/drawer.js", JS, "js/views/drawer.js"),
    asset!("js/views/login.js", JS, "js/views/login.js"),
    asset!("js/views/library.js", JS, "js/views/library.js"),
    asset!("js/views/editor.js", JS, "js/views/editor.js"),
    asset!("js/views/options.js", JS, "js/views/options.js"),
    asset!("js/views/stage.js", JS, "js/views/stage.js"),
    asset!("js/views/trust.js", JS, "js/views/trust.js"),
    asset!("vendor/guacamole.js", JS, "vendor/guacamole.js"),
    asset!("NOTICE", TEXT, "vendor/NOTICE"),
    asset!("LICENSE", TEXT, "vendor/LICENSE"),
];

pub fn find(path: &str) -> Option<&'static Asset> {
    ASSETS.iter().find(|asset| asset.path == path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn files(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                files(&path, out);
            } else {
                out.push(path);
            }
        }
    }

    #[test]
    fn every_web_file_is_served() {
        let web = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web");
        let mut found = vec![];
        files(&web, &mut found);
        let ignored = ["index.html", "GUIDELINES.md", "vendor/README.md"];
        let mut missing = vec![];
        for file in found {
            let relative = file
                .strip_prefix(&web)
                .unwrap()
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/");
            if ignored.contains(&relative.as_str()) {
                continue;
            }
            // NOTICE and LICENSE are served from the vendor directory at the top level.
            let served = relative
                .strip_prefix("vendor/")
                .filter(|name| matches!(*name, "NOTICE" | "LICENSE"))
                .unwrap_or(&relative);
            if find(served).is_none() {
                missing.push(relative);
            }
        }
        assert!(
            missing.is_empty(),
            "web files not in the asset table: {missing:?}"
        );
    }

    #[test]
    fn mime_types_match_extensions() {
        for asset in ASSETS {
            let expected = match asset.path.rsplit('.').next() {
                Some("css") => "text/css",
                Some("js") => "text/javascript",
                _ => "text/plain",
            };
            assert!(asset.mime.starts_with(expected), "{}", asset.path);
        }
    }
}
