#![recursion_limit = "512"]
//! cracamole 桌面介面（GPUI 版）原型：連線庫畫面。
//! 資料來自 `cracamole-store`（`CRACAMOLE_DESKTOP_DATA_DIR`），未設定時顯示範例連線。

use cracamole_store::PublicConnection;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::test::TestSupportExt;
use gpui_kit::*;
use std::collections::BTreeMap;

const BG: u32 = 0x0f1117;
const PANEL: u32 = 0x151821;
const CARD: u32 = 0x1a1e29;
const BORDER: u32 = 0x262b38;
const TEXT: u32 = 0xf4f4f5;
const MUTED: u32 = 0x9aa1b1;
const ACCENT: u32 = 0x7c5cff;
const OK: u32 = 0x22c55e;

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    All,
    Pinned,
}

struct Library {
    connections: Vec<PublicConnection>,
    pinned: Vec<String>,
    selected: Option<String>,
    tab: Tab,
    search: Entity<InputState>,
    editing: bool,
    protocol: &'static str,
    error: Option<String>,
    name: Entity<InputState>,
    host: Entity<InputState>,
    port: Entity<InputState>,
    user: Entity<InputState>,
}

const PROTOCOLS: [(&str, &str); 4] = [("ssh", "22"), ("rdp", "3389"), ("vnc", "5900"), ("telnet", "23")];

fn sample(id: &str, name: &str, protocol: &str, host: &str, user: &str) -> PublicConnection {
    let mut params = BTreeMap::new();
    params.insert("hostname".to_string(), host.to_string());
    params.insert("username".to_string(), user.to_string());
    PublicConnection {
        id: id.into(),
        name: name.into(),
        protocol: protocol.into(),
        params,
        configured_secrets: vec![],
    }
}

fn load() -> Vec<PublicConnection> {
    if let Some(dir) = std::env::var_os("CRACAMOLE_DESKTOP_DATA_DIR") {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        if let Ok(list) = runtime.block_on(cracamole_store::Store::list_existing(dir.into())) {
            return list;
        }
    }
    vec![
        sample("1", "prod-web-01", "ssh", "10.0.1.11", "deploy"),
        sample("2", "prod-db-01", "ssh", "10.0.1.21", "dba"),
        sample("3", "win-rdp-01", "rdp", "10.0.2.15", "alice"),
        sample("4", "mac-studio-01", "vnc", "10.0.2.30", "studio"),
        sample("5", "edge-router-01", "telnet", "10.0.3.1", "admin"),
    ]
}

impl Library {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("搜尋名稱、主機或協定"));
        cx.subscribe(&search, |_, _, _: &InputEvent, cx| cx.notify()).detach();
        let connections = load();
        let pinned = connections.iter().take(2).map(|c| c.id.clone()).collect();
        let field = |window: &mut Window, cx: &mut Context<Self>, hint: &'static str| {
            cx.new(|cx| InputState::new(window, cx).placeholder(hint))
        };
        let name = field(window, cx, "連線名稱");
        let host = field(window, cx, "主機名稱或位址");
        let port = field(window, cx, "連接埠（留空使用預設）");
        let user = field(window, cx, "帳號");
        Self {
            connections,
            pinned,
            selected: None,
            tab: Tab::All,
            search,
            editing: false,
            protocol: "ssh",
            error: None,
            name,
            host,
            port,
            user,
        }
    }

    /// 以欄位內容建立草稿並以共用規則驗證；成功時加入清單並關閉編輯器。
    fn save(&mut self, cx: &mut Context<Self>) {
        let text = |state: &Entity<InputState>| state.read(cx).value().trim().to_string();
        let default_port = PROTOCOLS
            .iter()
            .find(|(name, _)| *name == self.protocol)
            .map_or("", |(_, port)| *port);
        let mut params = BTreeMap::new();
        params.insert("hostname".to_string(), text(&self.host));
        let port = text(&self.port);
        params.insert(
            "port".to_string(),
            if port.is_empty() { default_port.to_string() } else { port },
        );
        let user = text(&self.user);
        if !user.is_empty() {
            params.insert("username".to_string(), user);
        }
        let draft = cracamole_store::Draft {
            name: text(&self.name),
            protocol: self.protocol.to_string(),
            params,
        };
        match cracamole_store::validate(&draft) {
            Err(invalid) => self.error = Some(invalid.message.to_string()),
            Ok(()) => {
                self.connections.push(PublicConnection {
                    id: format!("new-{}", self.connections.len() + 1),
                    name: draft.name,
                    protocol: draft.protocol,
                    params: draft.params,
                    configured_secrets: vec![],
                });
                self.editing = false;
                self.error = None;
            }
        }
        cx.notify();
    }

    fn visible(&self, cx: &App) -> Vec<PublicConnection> {
        let query = self.search.read(cx).value().to_lowercase();
        self.connections
            .iter()
            .filter(|c| self.tab == Tab::All || self.pinned.contains(&c.id))
            .filter(|c| {
                query.is_empty()
                    || c.name.to_lowercase().contains(&query)
                    || c.protocol.contains(&query)
                    || c.params.get("hostname").is_some_and(|h| h.contains(&query))
            })
            .map(|c| PublicConnection {
                id: c.id.clone(),
                name: c.name.clone(),
                protocol: c.protocol.clone(),
                params: c.params.clone(),
                configured_secrets: c.configured_secrets.clone(),
            })
            .collect()
    }
}

impl Library {
    fn editor_card(&self, cx: &mut Context<Self>) -> Div {
        let label = |text: &'static str| div().text_size(px(13.)).text_color(rgb(MUTED)).child(text);
        let mut protocols = div().flex().gap_2();
        for (name, _) in PROTOCOLS {
            let id: &'static str = match name {
                "ssh" => "protocol-ssh",
                "rdp" => "protocol-rdp",
                "vnc" => "protocol-vnc",
                _ => "protocol-telnet",
            };
            protocols = protocols.child(
                pill(id, name.to_uppercase(), self.protocol == name)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.protocol = name;
                        cx.notify();
                    }))
                    .test_support()
                    .aria_label(name),
            );
        }
        let button = |id: &'static str, text: &'static str, primary: bool| {
            div()
                .id(id)
                .px_5()
                .h(px(38.))
                .flex()
                .items_center()
                .rounded(px(10.))
                .border_1()
                .border_color(rgb(if primary { ACCENT } else { BORDER }))
                .bg(rgb(if primary { ACCENT } else { PANEL }))
                .text_color(rgb(if primary { 0xffffff } else { TEXT }))
                .cursor_pointer()
                .child(text)
        };
        div()
            .w(px(520.))
            .p_6()
            .rounded(px(16.))
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(CARD))
            .flex()
            .flex_col()
            .gap_3()
            .child(div().text_size(px(18.)).text_color(rgb(TEXT)).child("新增連線"))
            .child(label("協定"))
            .child(protocols)
            .child(label("名稱"))
            .child(Input::new(&self.name))
            .child(label("主機"))
            .child(Input::new(&self.host))
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(label("連接埠"))
                            .child(Input::new(&self.port)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(label("帳號"))
                            .child(Input::new(&self.user)),
                    ),
            )
            .children(self.error.clone().map(|message| {
                div().text_size(px(13.)).text_color(rgb(0xf87171)).child(message)
            }))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        button("cancel", "取消", false)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.editing = false;
                                this.error = None;
                                cx.notify();
                            }))
                            .test_support()
                            .aria_label("取消"),
                    )
                    .child(
                        button("save", "儲存", true)
                            .on_click(cx.listener(|this, _, _, cx| this.save(cx)))
                            .test_support()
                            .aria_label("儲存"),
                    ),
            )
    }
}

fn protocol_color(protocol: &str) -> u32 {
    match protocol {
        "ssh" => 0x60a5fa,
        "rdp" => 0x22d3ee,
        "vnc" => 0xfbbf24,
        _ => 0xc4b5fd,
    }
}

fn pill(id: &'static str, label: String, active: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex_1()
        .h(px(40.))
        .flex()
        .items_center()
        .justify_center()
        .gap_2()
        .rounded(px(10.))
        .border_1()
        .border_color(rgb(if active { ACCENT } else { BORDER }))
        .bg(rgb(if active { 0x24204a } else { PANEL }))
        .text_color(rgb(if active { TEXT } else { MUTED }))
        .cursor_pointer()
        .hover(|s| s.bg(rgb(0x20243a)))
        .child(label)
}

impl Render for Library {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.visible(cx);
        let total = self.connections.len();
        let pinned = self.pinned.len();
        let selected = self.selected.clone();
        let selected_name = rows
            .iter()
            .find(|c| Some(&c.id) == selected.as_ref())
            .map(|c| c.name.clone());

        let mut list = div()
            .id("list")
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .overflow_y_scroll()
            .flex_1();
        for c in rows {
            let id = c.id.clone();
            let is_selected = selected.as_deref() == Some(c.id.as_str());
            let is_pinned = self.pinned.contains(&c.id);
            let host = c.params.get("hostname").cloned().unwrap_or_default();
            let user = c.params.get("username").cloned().unwrap_or_default();
            list = list.child(
                div()
                    .id(SharedString::from(format!("row-{}", c.id)))
                    .test_support()
                    .aria_label(c.name.clone())
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .rounded(px(12.))
                    .border_1()
                    .border_color(rgb(if is_selected { ACCENT } else { PANEL }))
                    .bg(rgb(if is_selected { 0x1e1b3a } else { PANEL }))
                    .hover(|s| s.bg(rgb(CARD)))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected = Some(id.clone());
                        cx.notify();
                    }))
                    .child(div().size(px(8.)).rounded_full().bg(rgb(MUTED)))
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .rounded(px(6.))
                            .bg(rgb(0x20243a))
                            .text_color(rgb(protocol_color(&c.protocol)))
                            .text_size(px(12.))
                            .child(c.protocol.to_uppercase()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .child(div().text_color(rgb(TEXT)).child(c.name.clone()))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .text_color(rgb(MUTED))
                                    .child(format!("{user}@{host}")),
                            ),
                    )
                    .child(
                        div()
                            .text_color(rgb(if is_pinned { 0xfbbf24 } else { MUTED }))
                            .child(if is_pinned { "★" } else { "☆" }),
                    ),
            );
        }

        let sidebar = div()
            .w(px(380.))
            .h_full()
            .flex()
            .flex_col()
            .bg(rgb(PANEL))
            .border_r_1()
            .border_color(rgb(BORDER))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .p_4()
                    .child(div().size(px(10.)).rounded_full().bg(rgb(ACCENT)))
                    .child(div().text_size(px(20.)).text_color(rgb(TEXT)).child("cracamole")),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .px_4()
                    .pb_4()
                    .child(div().flex_1().child(Input::new(&self.search)))
                    .child(
                        div()
                            .id("add")
                            .px_4()
                            .h(px(36.))
                            .flex()
                            .items_center()
                            .rounded(px(10.))
                            .bg(rgb(ACCENT))
                            .text_color(rgb(0xffffff))
                            .cursor_pointer()
                            .hover(|s| s.opacity(0.9))
                            .child("新增")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.editing = true;
                                this.error = None;
                                cx.notify();
                            }))
                            .test_support()
                            .aria_label("新增連線"),
                    ),
            )
            .child(
                div()
                    .px_4()
                    .pb_2()
                    .text_size(px(13.))
                    .text_color(rgb(MUTED))
                    .child("連線庫"),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .px_4()
                    .pb_3()
                    .child(
                        pill("tab-all", format!("全部  {total}"), self.tab == Tab::All).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.tab = Tab::All;
                                cx.notify();
                            }),
                        )
                        .test_support()
                        .aria_label("全部"),
                    )
                    .child(
                        pill("tab-pinned", format!("已釘選  {pinned}"), self.tab == Tab::Pinned)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.tab = Tab::Pinned;
                                cx.notify();
                            }))
                            .test_support()
                            .aria_label("已釘選"),
                    ),
            )
            .child(div().h(px(1.)).bg(rgb(BORDER)))
            .child(list);

        let editor = self.editor_card(cx);
        let subtitle = if selected_name.is_some() {
            "按「連線」開始工作階段"
        } else {
            "從左側連線庫選擇主機開始工作"
        };
        let stage = div()
            .flex_1()
            .h_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .px_5()
                    .py_4()
                    .border_b_1()
                    .border_color(rgb(BORDER))
                    .child(
                        div()
                            .text_size(px(18.))
                            .text_color(rgb(TEXT))
                            .child(selected_name.unwrap_or_else(|| "尚未選擇連線".into())),
                    )
                    .child(div().text_size(px(13.)).text_color(rgb(MUTED)).child(subtitle)),
            )
            .child(
                div().flex_1().flex().items_center().justify_center().child(if self.editing {
                    editor
                } else {
                    div()
                        .w(px(520.))
                        .p_6()
                        .rounded(px(16.))
                        .border_1()
                        .border_color(rgb(BORDER))
                        .bg(rgb(CARD))
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_2()
                        .child(div().size(px(10.)).rounded_full().bg(rgb(OK)))
                        .child(
                            div()
                                .text_size(px(20.))
                                .text_color(rgb(TEXT))
                                .child("歡迎使用 cracamole"),
                        )
                        .child(
                            div()
                                .text_color(rgb(MUTED))
                                .child("選擇左側連線，或搜尋主機與輸入連線位址即可開始。"),
                        )
                }),
            );

        div()
            .size_full()
            .flex()
            .bg(rgb(BG))
            .text_color(rgb(TEXT))
            .child(sidebar)
            .child(stage)
    }
}

fn main() {
    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            let view = cx.new(|cx| Library::new(window, cx));
            cx.new(|cx| gpui_kit::component::Root::new(view, window, cx))
        })
        .expect("window");
    });
}

#[cfg(test)]
mod tests {
    use super::Library;
    use gpui_kit::test::{TestAppContextExt, TestWindowExt};
    use gpui_kit::{
        px, size, AppContext, Bounds, Entity, Point, TestAppContext, WindowBounds, WindowOptions,
    };

    fn open(cx: &mut TestAppContext) -> (gpui_kit::AnyWindowHandle, Entity<Library>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            let (window, content) = gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: Point::default(),
                        size: size(px(1200.), px(760.)),
                    })),
                    ..Default::default()
                },
                cx,
                |window, cx| cx.new(|cx| Library::new(window, cx)),
            )
            .expect("open test window");
            (window.into(), content)
        })
    }

    #[gpui_kit::test]
    fn library_lists_sample_connections_and_selects_on_click(cx: &mut TestAppContext) {
        let (window, content) = open(cx);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(content.read(cx).connections.len(), 5);
            window.click("row-2", cx);
            window.render_frame(cx);
            assert_eq!(content.read(cx).selected.as_deref(), Some("2"));
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn pinned_tab_filters_the_list(cx: &mut TestAppContext) {
        let (window, content) = open(cx);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(content.read(cx).visible(cx).len(), 5);
            window.click("tab-pinned", cx);
            window.render_frame(cx);
            assert_eq!(content.read(cx).visible(cx).len(), 2);
            assert!(window.try_find("row-5").is_none());
            window.click("tab-all", cx);
            window.render_frame(cx);
            assert_eq!(content.read(cx).visible(cx).len(), 5);
        })
        .unwrap();
    }

    fn fill(
        content: &Entity<Library>,
        window: &mut gpui_kit::Window,
        cx: &mut gpui_kit::App,
        values: [&str; 4],
    ) {
        let (name, host, port, user) = {
            let library = content.read(cx);
            (
                library.name.clone(),
                library.host.clone(),
                library.port.clone(),
                library.user.clone(),
            )
        };
        for (state, value) in [name, host, port, user].into_iter().zip(values) {
            state.update(cx, |state, cx| state.set_value(value, window, cx));
        }
    }

    #[gpui_kit::test]
    fn editor_rejects_invalid_input_and_saves_a_valid_connection(cx: &mut TestAppContext) {
        let (window, content) = open(cx);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("add", cx);
            window.render_frame(cx);
            assert!(content.read(cx).editing);

            // 缺少主機與名稱：驗證失敗且不新增。
            window.click("save", cx);
            window.render_frame(cx);
            assert!(content.read(cx).error.is_some());
            assert_eq!(content.read(cx).connections.len(), 5);

            // 無效連接埠同樣被共用驗證拒絕。
            fill(&content, window, cx, ["lab-rdp", "10.9.9.9", "99999", ""]);
            window.click("protocol-rdp", cx);
            window.click("save", cx);
            window.render_frame(cx);
            assert!(content.read(cx).error.is_some());
            assert_eq!(content.read(cx).connections.len(), 5);

            // 有效輸入：使用 RDP 預設連接埠並關閉編輯器。
            fill(&content, window, cx, ["lab-rdp", "10.9.9.9", "", "bob"]);
            window.click("save", cx);
            window.render_frame(cx);
            let library = content.read(cx);
            assert!(!library.editing && library.error.is_none());
            let saved = library.connections.last().unwrap();
            assert_eq!(saved.protocol, "rdp");
            assert_eq!(saved.params.get("port").map(String::as_str), Some("3389"));
            assert_eq!(saved.params.get("username").map(String::as_str), Some("bob"));
            assert_eq!(library.connections.len(), 6);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn cancel_closes_the_editor_without_saving(cx: &mut TestAppContext) {
        let (window, content) = open(cx);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("add", cx);
            window.render_frame(cx);
            window.click("cancel", cx);
            window.render_frame(cx);
            assert!(!content.read(cx).editing);
            assert_eq!(content.read(cx).connections.len(), 5);
        })
        .unwrap();
    }
}
