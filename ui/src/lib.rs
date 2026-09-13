mod grok;
mod hub;
mod icons;
pub mod pane;
mod state;
mod views;

use crate::hub::{
    apply_theme, close_nav_menu, cmd, go_root, live, path_now, push_path, rtc_apply, rtc_close,
    set_hop_want, term_fit, term_reset, term_write, theme_now, video_now,
};
use crate::pane::{VideoPref, Work};
use crate::state::State;
use crate::views::{Chat, ClusterGate, Session, Settings};
use leptos::prelude::*;
use serde_json::json;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phone {
    Chats,
    Room,
    Ai,
}

#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
    let Some(el) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id("main"))
    else {
        web_sys::console::error_1(&"#main missing".into());
        return;
    };
    leptos::mount::mount_to(el.unchecked_into(), App).forget();
    if let Some(boot) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id("boot"))
    {
        boot.remove();
    }
}

#[component]
fn App() -> impl IntoView {
    let st = RwSignal::new(State::default());
    let settings = RwSignal::new(false);
    let work = RwSignal::new(Work::Shell);
    let phone = RwSignal::new(Phone::Chats);
    let pref = RwSignal::new(video_now());
    let theme = RwSignal::new(theme_now());
    let draft = RwSignal::new(String::new());
    let agent_draft = RwSignal::new(String::new());
    let url_edit = RwSignal::new(String::new());
    let url_typing = RwSignal::new(false);
    let shot = RwSignal::new(String::new());
    let hop = RwSignal::new(false);
    let hop_want = RwSignal::new(String::new());
    let peers = Memo::new(move |_| st.with(|s| s.peers.clone()));

    Effect::new(move |_| apply_theme(&theme.get()));
    settings.set(pane::is_settings_path(&path_now()));
    {
        let cb = Closure::<dyn FnMut()>::new(move || {
            settings.set(pane::is_settings_path(&path_now()));
        });
        if let Some(w) = web_sys::window() {
            let _ = w.add_event_listener_with_callback("popstate", cb.as_ref().unchecked_ref());
        }
        cb.forget();
    }
    Effect::new(move |_| {
        let s = st.get();
        if !hop.get_untracked() {
            return;
        }
        let w = if s.kind.is_empty() {
            work.get_untracked()
        } else {
            pane::work_from_kind(&s.kind)
        };
        if pane::hop_ready(
            w,
            pref.get_untracked().is_turn(),
            s.busy,
            &s.stdout,
            s.jpeg_n,
            s.width,
            &s.answer,
            &s.kind,
            &s.last,
            &s.dst,
            &hop_want.get_untracked(),
        ) {
            hop.set(false);
        }
    });

    Effect::new(move |_| {
        let s = st.get();
        if !s.dst.is_empty() && !s.kind.is_empty() {
            let w = pane::work_from_kind(&s.kind);
            if work.get_untracked() != w {
                work.set(w);
                if w == Work::Shell {
                    term_fit();
                }
            }
        }
        if !url_typing.get_untracked() {
            let u = s
                .tabs
                .iter()
                .find(|t| t.active)
                .map(|t| t.url.clone())
                .filter(|u| !u.is_empty())
                .unwrap_or_else(|| s.url.clone());
            url_edit.set(u);
        }
        let w = work.get_untracked();
        let p = pref.get_untracked();
        if w == Work::Browser && p.is_turn() && s.kind == "browser" && s.video == "webrtc" && s.width != 0
        {
            rtc_apply(&s);
        }
    });

    leptos::task::spawn_local(async move {
        live(st).await;
    });

    let leave_settings = move || {
        settings.set(false);
        let s = st.get_untracked();
        if s.dst.is_empty() {
            phone.set(Phone::Chats);
            push_path("/");
            return;
        }
        phone.set(Phone::Room);
        let slug = s
            .peers
            .iter()
            .find(|x| x.id == s.dst)
            .map(|x| x.name.clone())
            .unwrap_or_else(|| s.dst.clone());
        push_path(&pane::route_path(&slug, work.get_untracked(), pref.get_untracked()));
    };

    let open_settings = move || {
        close_nav_menu();
        settings.set(true);
        phone.set(Phone::Room);
        push_path("/settings");
    };

    let home = move |_| {
        phone.set(Phone::Chats);
        push_path("/");
        rtc_close();
        term_reset();
        st.update(|s| {
            s.dst.clear();
            s.kind.clear();
            s.video.clear();
            s.stdout.clear();
            s.answer.clear();
            s.ice.clear();
        });
        hop.set(false);
        hop_want.set(String::new());
        set_hop_want(Some(String::new()));
        settings.set(false);
        leptos::task::spawn_local(async { cmd(json!({"type":"home"})).await });
    };

    let open_box = move |id: String, name: String| {
        let cur = st.get_untracked();
        if cur.dst == id && cur.kind == "shell" && !cur.stdout.is_empty() {
            work.set(Work::Shell);
            term_reset();
            term_write(&cur.stdout);
            term_fit();
            hop.set(false);
            hop_want.set(id);
            set_hop_want(None);
            return;
        }
        rtc_close();
        term_reset();
        hop.set(true);
        hop_want.set(id.clone());
        set_hop_want(Some(id.clone()));
        push_path(&pane::route_path(&name, Work::Shell, VideoPref::Jpeg));
        st.update(|s| {
            s.dst = id.clone();
            s.kind = "shell".into();
            s.video = "none".into();
            s.stdout.clear();
            s.answer.clear();
            s.ice.clear();
            s.tabs.clear();
            s.url.clear();
            s.last.clear();
        });
        work.set(Work::Shell);
        settings.set(false);
        phone.set(Phone::Room);
        shot.set(String::new());
        leptos::task::spawn_local(async move {
            cmd(json!({"type":"open","dst": id, "kind":"shell", "video":"none"})).await;
        });
    };

    let open_work = move |w: Work| {
        if w == Work::Browser {
            pref.set(video_now());
        }
        work.set(w);
        if w == Work::Shell {
            term_fit();
        }
        let s = st.get_untracked();
        let p = pref.get_untracked();
        let slug = s
            .peers
            .iter()
            .find(|x| x.id == s.dst)
            .map(|x| x.name.clone())
            .unwrap_or_else(|| s.dst.clone());
        push_path(&pane::route_path(&slug, w, p));
        let (kind, video, height) = pane::work_open(w, p);
        let same = s.kind == kind && (kind != "browser" || s.video == video);
        if same {
            if w == Work::Shell {
                term_fit();
            }
            return;
        }
        hop.set(true);
        hop_want.set(s.dst.clone());
        set_hop_want(Some(s.dst.clone()));
        rtc_close();
        if w == Work::Shell {
            term_reset();
            term_fit();
        }
        let dst = s.dst.clone();
        leptos::task::spawn_local(async move {
            cmd(json!({"type":"open","dst": dst, "kind": kind, "video": video, "height": height}))
                .await;
        });
    };

    view! {
        <div class=move || match phone.get() {
            Phone::Chats => "shell h-full w-full phone-chats",
            Phone::Room => "shell h-full w-full phone-room",
            Phone::Ai => "shell h-full w-full phone-ai",
        }>
            <nav class="navbar top-bar bg-base-100 shadow-sm" data-testid="chats-nav">
                <div class="navbar-start nav-brand" data-testid="nav-col-chats">
                    <button class="brand" type="button" id="back-boxes" data-testid="nav-boxes" on:click=home>
                        <span class="text-lg font-extrabold tracking-tight">"Connect 2"</span>
                    </button>
                    <div class="nav-actions">
                    <button
                        class="btn btn-ghost btn-square btn-sm"
                        type="button"
                        data-testid="theme"
                        aria-label=move || if theme.get() == "dark" { "Light" } else { "Dark" }
                        on:click=move |_| {
                            theme.update(|t| *t = if t == "dark" { "light" } else { "dark" }.into());
                        }
                    >
                        {move || if theme.get() == "dark" { icons::sun() } else { icons::moon() }}
                    </button>
                    <div class="dropdown dropdown-end" id="nav-menu">
                        <div tabindex="0" role="button" class="btn btn-ghost btn-square btn-sm" data-testid="nav-more" aria-label="Menu">
                            {icons::menu()}
                        </div>
                        <ul tabindex="-1" class="menu menu-sm dropdown-content bg-base-100 rounded-box z-20 mt-3 w-52 p-2 shadow">
                            <li>
                                <button type="button" data-testid="nav-settings" on:click=move |_| open_settings()>"Settings"</button>
                            </li>
                            <Show when=move || st.get().authed>
                                <li>
                                    <button
                                        type="button"
                                        data-testid="plane-logout"
                                        on:click=move |_| {
                                            close_nav_menu();
                                            go_root();
                                            rtc_close();
                                            term_reset();
                                            settings.set(false);
                                            st.update(|s| {
                                                s.authed = false;
                                                s.me.clear();
                                                s.peers.clear();
                                                s.dst.clear();
                                                s.kind.clear();
                                            });
                                            phone.set(Phone::Chats);
                                            leptos::task::spawn_local(async {
                                                cmd(json!({"type":"cluster_logout"})).await;
                                            });
                                        }
                                    >
                                        "Sign out"
                                    </button>
                                </li>
                            </Show>
                        </ul>
                    </div>
                    </div>
                </div>
                <div class="navbar-center nav-room" data-testid="nav-col-room">
                    <Show when=move || !st.get().dst.is_empty() && !settings.get()>
                        <Show when=move || work.get() == Work::Agent>
                            <button
                                class="btn btn-ghost btn-square btn-sm"
                                type="button"
                                id="newchat"
                                aria-label="New chat"
                                on:click=move |_| {
                                    leptos::task::spawn_local(async { cmd(json!({"type":"new_chat"})).await });
                                }
                            >
                                {icons::new_chat()}
                            </button>
                        </Show>
                        <p class="box-name m-0 truncate" data-testid="box-name">{move || {
                            let s = st.get();
                            s.peers
                                .iter()
                                .find(|p| p.id == s.dst)
                                .map(|p| p.name.clone())
                                .filter(|n| !n.is_empty())
                                .unwrap_or_else(|| s.dst.clone())
                        }}</p>
                        <div id="nav" class="nav-work ml-auto">
                            <button
                                class=move || icons::square(work.get() == Work::Shell)
                                type="button"
                                data-testid="nav-shell"
                                aria-label="Shell"
                                on:click=move |_| open_work(Work::Shell)
                            >
                                {icons::shell()}
                            </button>
                            <button
                                class=move || icons::square(work.get() == Work::Browser)
                                type="button"
                                data-testid="nav-browser"
                                aria-label="Browser"
                                on:click=move |_| open_work(Work::Browser)
                            >
                                {icons::browser()}
                            </button>
                            <button
                                class=move || icons::square(work.get() == Work::Agent)
                                type="button"
                                data-testid="nav-agent"
                                aria-label="Agent"
                                on:click=move |_| open_work(Work::Agent)
                            >
                                {icons::agent()}
                            </button>
                        </div>
                        <button
                            class="btn btn-ghost btn-square btn-sm"
                            type="button"
                            data-testid="box-close"
                            aria-label="Close"
                            on:click=home
                        >
                            {icons::x()}
                        </button>
                    </Show>
                </div>
                <div class="navbar-end nav-ai" data-testid="nav-col-ai">
                    <Show when=move || st.get().grok.configured>
                        <button
                            class="btn btn-ghost btn-square btn-sm"
                            type="button"
                            data-testid="ai-new"
                            aria-label="New chat"
                            on:click=move |_| {
                                st.update(|s| s.desk.clear());
                                leptos::task::spawn_local(async { cmd(json!({"type":"new_desk"})).await });
                            }
                        >
                            {icons::new_chat()}
                        </button>
                    </Show>
                    <p class="ai-name m-0 truncate" data-testid="ai-title">"Ask AI"</p>
                </div>
            </nav>
            <div class="desk">
            <aside class="chats" data-testid="dir-title">
                <ul class="menu menu-lg bg-base-200 chats-list min-h-0 w-full flex-1 overflow-auto p-1">
                    <Show
                        when=move || !st.get().authed
                        fallback=move || {
                            view! {
                                <Show
                                    when=move || st.get().peers.is_empty()
                                    fallback=move || {
                                        view! {
                                            <For
                                                each=move || peers.get()
                                                key=|p| p.id.clone()
                                                children=move |p| {
                                                    let id = p.id.clone();
                                                    let name = p.name.clone();
                                                    let id2 = id.clone();
                                                    let name2 = name.clone();
                                                    view! {
                                                        <li>
                                                            <button
                                                                class=move || {
                                                                    let s = st.get();
                                                                    if s.dst == id && !s.kind.is_empty() {
                                                                        "menu-active min-h-10 transition-none"
                                                                    } else {
                                                                        "min-h-10 transition-none"
                                                                    }
                                                                }
                                                                data-testid="box-card"
                                                                type="button"
                                                                on:click=move |_| open_box(id2.clone(), name2.clone())
                                                            >
                                                                <span class="min-w-0 flex-1 truncate text-left" data-testid="box-card-name">{name.clone()}</span>
                                                            </button>
                                                        </li>
                                                    }
                                                }
                                            />
                                        }
                                    }
                                >
                                    <p class="text-base-content/55 px-3 py-6 text-center text-sm" data-testid="empty-boxes">"No boxes online."</p>
                                </Show>
                            }
                        }
                    >
                        <p class="text-base-content/55 px-3 py-6 text-center text-sm" data-testid="empty-boxes">"Sign in to list boxes."</p>
                    </Show>
                </ul>
            </aside>
            <div class="room">
                <Show
                    when=move || settings.get()
                    fallback=move || {
                        view! {
                            <Show
                                when=move || st.get().dst.is_empty()
                                fallback=move || {
                                    view! {
                                        <Session
                                            st=st
                                            work=work
                                            pref=pref
                                            url_edit=url_edit
                                            url_typing=url_typing
                                            shot=shot
                                            agent_draft=agent_draft
                                            hop=hop
                                        />
                                    }
                                }
                            >
                                <Show
                                    when=move || !st.get().authed
                                    fallback=move || {
                                        view! {
                                            <div class="welcome min-h-0 flex-1"></div>
                                        }
                                    }
                                >
                                    <ClusterGate st=st />
                                </Show>
                            </Show>
                        }
                    }
                >
                    <Settings st=st pref=pref on_close=leave_settings />
                </Show>
            </div>
            <Chat st=st draft=draft />
            </div>
            <nav class="phone-dock bt-nav border-t border-base-300 bg-base-100" data-testid="phone-dock">
                <button
                    class=move || icons::square(phone.get() == Phone::Chats)
                    type="button"
                    data-testid="phone-chats"
                    aria-label="Boxes"
                    on:click=move |_| phone.set(Phone::Chats)
                >
                    {icons::boxes()}
                </button>
                <button
                    class=move || icons::square(phone.get() == Phone::Room)
                    type="button"
                    data-testid="phone-room"
                    aria-label="Room"
                    on:click=move |_| phone.set(Phone::Room)
                >
                    {icons::room()}
                </button>
                <button
                    class=move || icons::square(phone.get() == Phone::Ai)
                    type="button"
                    data-testid="phone-ai"
                    aria-label="Ask AI"
                    on:click=move |_| phone.set(Phone::Ai)
                >
                    {icons::ai()}
                </button>
            </nav>
        </div>
    }
}
