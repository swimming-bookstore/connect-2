//! Open box: browser, shell, agent.

use crate::hub::{self, cmd, term_fit};
use crate::icons;
use crate::pane::{self, VideoPref, Work};
use crate::state::State;
use leptos::prelude::*;
use serde_json::json;
use wasm_bindgen::JsCast;

#[component]
pub fn Session(
    st: RwSignal<State>,
    work: RwSignal<Work>,
    pref: RwSignal<VideoPref>,
    url_edit: RwSignal<String>,
    url_typing: RwSignal<bool>,
    shot: RwSignal<String>,
    agent_draft: RwSignal<String>,
    hop: RwSignal<bool>,
) -> impl IntoView {
    let go = move |_| {
        let u = url_edit.get();
        url_typing.set(false);
        if u.trim().is_empty() {
            return;
        }
        leptos::task::spawn_local(async move {
            cmd(json!({"type":"navigate","url": u})).await;
        });
    };

    view! {
        <div class="session relative">
            <div id="stage-wrap" class="relative">
                <div id="chrome" class=move || if work.get() == Work::Browser { "" } else { "off" }>
                    <div id="tabs" class="tabs tabs-border px-3 pt-2">
                        <For
                            each=move || st.get().tabs
                            key=|t| format!("{}:{}:{}:{}", t.id, t.title, t.url, t.active)
                            children=move |t| {
                                let id = t.id.clone();
                                let id2 = t.id.clone();
                                let active = t.active;
                                let label = pane::tab_label(&t.title, &t.url);
                                let tip = label.clone();
                                view! {
                                    <button
                                        class=if active { "tab tab-active btab on" } else { "tab btab" }
                                        data-testid="tab"
                                        title=tip
                                        on:mousedown=move |_| {
                                            let id = id.clone();
                                            leptos::task::spawn_local(async move {
                                                cmd(json!({"type":"focus","id": id})).await;
                                            });
                                        }
                                    >
                                        <span class="tab-label" data-testid="tab-label">{label}</span>
                                        <span
                                            class="x"
                                            on:mousedown=move |ev| {
                                                ev.stop_propagation();
                                                let id = id2.clone();
                                                leptos::task::spawn_local(async move {
                                                    cmd(json!({"type":"close_tab","id": id})).await;
                                                });
                                            }
                                        >
                                            "×"
                                        </span>
                                    </button>
                                }
                            }
                        />
                        <button
                            class="tab btab"
                            data-testid="tab-new"
                            type="button"
                            on:click=move |_| {
                                leptos::task::spawn_local(async {
                                    cmd(json!({"type":"new_tab","url":"about:blank"})).await;
                                });
                            }
                        >
                            "+"
                        </button>
                    </div>
                    <div id="bar" class="join w-full min-w-0 items-stretch px-3 pb-3 pt-2">
                        <button
                            id="back"
                            class="btn btn-sm join-item"
                            data-testid="nav-back"
                            type="button"
                            on:click=move |_| {
                                leptos::task::spawn_local(async { cmd(json!({"type":"back"})).await });
                            }
                        >
                            "←"
                        </button>
                        <button
                            id="fwd"
                            class="btn btn-sm join-item"
                            data-testid="nav-fwd"
                            type="button"
                            on:click=move |_| {
                                leptos::task::spawn_local(async { cmd(json!({"type":"forward"})).await });
                            }
                        >
                            "→"
                        </button>
                        <input
                            id="url"
                            class="input input-sm join-item min-w-0 flex-1 font-mono"
                            type="text"
                            prop:value=move || url_edit.get()
                            on:focus=move |_| url_typing.set(true)
                            on:blur=move |_| url_typing.set(false)
                            on:input=move |ev| {
                                url_typing.set(true);
                                url_edit.set(event_target_value(&ev));
                            }
                            on:keydown=move |ev| {
                                if ev.key() == "Enter" {
                                    let u = url_edit.get();
                                    url_typing.set(false);
                                    if !u.trim().is_empty() {
                                        leptos::task::spawn_local(async move {
                                            cmd(json!({"type":"navigate","url": u})).await;
                                        });
                                    }
                                }
                            }
                        />
                        <button id="go" class="btn btn-sm join-item" type="button" on:click=go>"Go"</button>
                    </div>
                </div>
                <div
                    class=move || {
                        if work.get() == Work::Browser && !pref.get().is_turn() {
                            "pane"
                        } else {
                            "pane off"
                        }
                    }
                    id="stage"
                >
                    <img id="view" tabindex="0" src=move || shot.get() alt="" />
                    <img
                        id="view-next"
                        src=move || {
                            if work.get() == Work::Browser && !pref.get().is_turn() {
                                pane::hub_api(&hub::hub(), &format!("/shot?{}", st.get().jpeg_n))
                            } else {
                                String::new()
                            }
                        }
                        alt=""
                        on:load=move |ev| {
                            if let Some(el) = ev.target().and_then(|t| t.dyn_into::<web_sys::HtmlImageElement>().ok())
                            {
                                if el.natural_width() >= 32 {
                                    let u = el.src();
                                    if !u.is_empty() {
                                        shot.set(u);
                                    }
                                }
                            }
                        }
                    />
                </div>
                <div
                    class=move || {
                        if work.get() == Work::Browser && pref.get().is_turn() {
                            "pane"
                        } else {
                            "pane off"
                        }
                    }
                    id="live"
                ></div>
                <div
                    class=move || if work.get() == Work::Shell { "pane" } else { "pane off" }
                    id="term"
                    tabindex="0"
                    on:mousedown=move |_| term_fit()
                ></div>
                <div
                    class=move || if work.get() == Work::Agent { "pane" } else { "pane off" }
                    id="agent"
                >
                    <div id="chat" class="flex min-h-0 flex-1 flex-col gap-3 overflow-auto px-5 py-3">
                        <For
                            each=move || {
                                st.get()
                                    .log
                                    .into_iter()
                                    .enumerate()
                                    .filter_map(|(i, line)| pane::chat_row(&line).map(|(k, t)| (i, k, t)))
                                    .collect::<Vec<_>>()
                            }
                            key=|row| row.0
                            children=move |(_, kind, t)| {
                                view! {
                                    <div class=if kind == "me" { "chat chat-end" } else { "chat chat-start" }>
                                        <div class=if kind == "me" { "chat-bubble chat-bubble-primary" } else if kind == "tool" { "chat-bubble font-mono text-xs" } else { "chat-bubble" }>{t}</div>
                                    </div>
                                }
                            }
                        />
                    </div>
                    <form
                        id="ask"
                        class="flex w-full min-w-0 items-stretch gap-2 bg-base-100 px-4 py-3"
                        on:submit=move |ev| {
                            ev.prevent_default();
                            let t = agent_draft.get();
                            agent_draft.set(String::new());
                            if !t.trim().is_empty() {
                                leptos::task::spawn_local(async move {
                                    cmd(json!({"type":"agent_ask","text": t})).await;
                                });
                            }
                        }
                    >
                        <input
                            id="q"
                            class="input min-w-0 flex-1"
                            autocomplete="off"
                            spellcheck="false"
                            prop:value=move || agent_draft.get()
                            on:input=move |ev| agent_draft.set(event_target_value(&ev))
                        />
                        <button class="btn btn-primary btn-square shrink-0" type="submit" aria-label="Send">{icons::send()}</button>
                    </form>
                </div>
                <Show when=move || hop.get() || st.get().busy>
                    <div
                        class="bg-base-100/80 absolute inset-0 z-10 flex flex-col items-center justify-center gap-3"
                        data-testid="box-loading"
                    >
                        <span class="loading loading-spinner loading-lg text-base-content"></span>
                        <p class="m-0 text-sm font-medium">"Connecting…"</p>
                    </div>
                </Show>
            </div>
        </div>
    }
}

