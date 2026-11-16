//! 離屏渲染截圖：以 GPUI 的 wgpu 無頭渲染器把畫面畫成 PNG，不需要桌面或實體視窗。
//!
//! 只在設定 `CRACAMOLE_SCREENSHOT_DIR` 時執行：
//! `CRACAMOLE_SCREENSHOT_DIR=out cargo test screenshots -- --nocapture`

use super::{Library, Pending};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    px, size, AnyWindowHandle, AppContext as _, HeadlessAppContext, PlatformHeadlessRenderer,
};
use std::path::PathBuf;
use std::sync::Arc;

fn context() -> HeadlessAppContext {
    let text_system = Arc::new(gpui_wgpu::CosmicTextSystem::new("Segoe UI"));
    let cx = HeadlessAppContext::with_platform(text_system, Arc::new(()), || {
        gpui_wgpu::WgpuHeadlessRenderer::new()
            .map(|renderer| Some(Box::new(renderer) as Box<dyn PlatformHeadlessRenderer>))
    });
    cx.allow_parking();
    cx
}

fn save(cx: &mut HeadlessAppContext, window: AnyWindowHandle, dir: &std::path::Path, name: &str) {
    cx.update_window(window, |_, window, cx| window.render_frame(cx)).unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| window.render_frame(cx)).unwrap();
    let image = cx.capture_screenshot(window).expect("離屏渲染失敗");
    let path: PathBuf = dir.join(format!("{name}.png"));
    image.save(&path).unwrap();
    // 簡單的健全檢查：畫面不是單一顏色。
    let first = *image.get_pixel(0, 0);
    let varied = image.pixels().step_by(997).any(|pixel| *pixel != first);
    println!("{} {}x{} varied={varied}", path.display(), image.width(), image.height());
    assert!(varied, "{name} 渲染結果是單一顏色");
}

#[test]
fn screenshots() {
    let Some(dir) = std::env::var_os("CRACAMOLE_SCREENSHOT_DIR").map(PathBuf::from) else {
        return;
    };
    std::fs::create_dir_all(&dir).unwrap();
    let mut cx = context();
    cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::component::Theme::change(gpui_kit::component::ThemeMode::Dark, None, cx);
    });

    fn open(
        cx: &mut HeadlessAppContext,
        build: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::Context<Library>) -> Library + 'static,
    ) -> gpui_kit::WindowHandle<gpui_kit::component::Root> {
        cx.open_window(size(px(1280.), px(800.)), move |window, cx| {
            let view = cx.new(|cx| build(window, cx));
            cx.new(|cx| gpui_kit::component::Root::new(view, window, cx))
        })
        .unwrap()
    }

    // 1. 登入頁
    let credentials = Arc::new(
        super::remote::block_on(super::Credentials::new("admin".into(), "correct-horse-battery".into())).unwrap(),
    );
    let login = open(&mut cx, move |window, cx| {
        Library::locked(window, cx, Ok(credentials), None, std::time::Duration::from_secs(3600))
    });
    save(&mut cx, login.into(), &dir, "01-login");

    // 範例資料：含群組與釘選。
    fn populated(window: &mut gpui_kit::Window, cx: &mut gpui_kit::Context<Library>) -> Library {
        let mut library = Library::new(window, cx);
        library.connections.clear();
        for (id, name, protocol, host, user) in [
            ("1", "核心機房 / prod-db-01", "ssh", "10.0.1.21", "dba"),
            ("2", "核心機房 / prod-web-01", "ssh", "10.0.1.11", "deploy"),
            ("3", "辦公網段 / win-rdp-01", "rdp", "10.0.2.15", "alice"),
            ("4", "辦公網段 / mac-studio-01", "vnc", "10.0.2.30", "studio"),
            ("5", "edge-router-01", "telnet", "10.0.3.1", "admin"),
        ] {
            library.connections.push(super::sample(id, name, protocol, host, user));
        }
        library.pinned = vec!["2".into(), "3".into()];
        library
    }

    // 2. 連線庫（未選取）
    let library = open(&mut cx, |window, cx| populated(window, cx));
    save(&mut cx, library.into(), &dir, "02-library");

    // 3. 連線庫（選取一筆、已記住指紋）
    let selected = open(&mut cx, |window, cx| {
        let mut library = populated(window, cx);
        library.selected = Some("2".into());
        library.saved_trust = Some(
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIDTq3Yk0nQw8rF9m7pXcVbL2sHj4uE6gN1oZtKyAaBcD".into(),
        );
        library
    });
    save(&mut cx, selected.into(), &dir, "03-selected");

    // 4. 新增連線編輯器
    let editor = open(&mut cx, |window, cx| {
        let mut library = populated(window, cx);
        library.editing = true;
        library
    });
    save(&mut cx, editor.into(), &dir, "04-editor");

    // 5. 指紋核對
    let trust = open(&mut cx, |window, cx| {
        let mut library = populated(window, cx);
        library.selected = Some("2".into());
        library.pending = Some(Pending {
            connection: cracamole_store::Connection {
                id: "2".into(),
                name: "prod-web-01".into(),
                protocol: "ssh".into(),
                params: [("hostname".to_string(), "10.0.1.11".to_string())].into(),
            },
            trust_param: "host-key",
            trust_value: "ssh-ed25519 AAAA".into(),
            fingerprint: "SHA256:Vq3w9Zr1nT8kP2mXbL5cHj7uE4gN0oYtKsAaBcDeFgH".into(),
            detail: None,
        });
        library
    });
    save(&mut cx, trust.into(), &dir, "05-trust");

    // 6. 匯入預覽
    let import = open(&mut cx, |window, cx| {
        let mut library = populated(window, cx);
        library.import_preview = cracamole_store::portable::import(
            "ssh://ops@10.8.0.1\nvnc://10.8.0.2:5901\nrdp://CORP%5Cbob@10.8.0.3\n",
        )
        .ok();
        library
    });
    save(&mut cx, import.into(), &dir, "06-import");
}
