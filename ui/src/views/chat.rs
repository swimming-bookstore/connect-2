//! Laptop Grok pane.

use crate::hub::{cmd, js_call};
use crate::icons;
use crate::pane;
use crate::state::State;
use leptos::prelude::*;
use serde_json::json;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;

fn send_ai(st: RwSignal<State>, draft: RwSignal<String>) {
    let t = draft.get().trim().to_string();
    draft.set(String::new());
    if let Some(el) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id("ai-q"))
        .and_then(|e| e.dyn_into::<web_sys::HtmlInputElement>().ok())
    {
        el.set_value("");
        let _ = el.focus();
    }
    if t.is_empty() {
        return;
    }
    st.update(|s| s.desk.push(format!("you {t}")));
    leptos::task::spawn_local(async move { cmd(json!({"type":"ask","text": t})).await });
}

#[component]
pub fn Chat(st: RwSignal<State>, draft: RwSignal<String>) -> impl IntoView {
    view! {
        <aside
            class="ai-pane"
            data-testid="ai-chat"
            on:mousedown=move |ev| {
                ev.stop_propagation();
                js_call("connectTermBlur", &JsValue::UNDEFINED);
            }
            on:pointerdown=move |ev| {
                ev.stop_propagation();
                js_call("connectTermBlur", &JsValue::UNDEFINED);
            }
            on:keydown=move |ev| ev.stop_propagation()
        >
            <Show
                when=move || st.get().grok.configured
                fallback=move || {
                    view! {
                        <Show
                            when=move || st.get().grok.user_code.is_some()
                            fallback=move || {
                                view! {
                                    <div class="card-body gate flex flex-1 flex-col items-center justify-center gap-4 text-center" data-testid="ai-gate">
                                        <button
                                            class="btn btn-primary"
                                            type="button"
                                            data-testid="ai-login"
                                            on:click=move |_| {
                                                leptos::task::spawn_local(async {
                                                    cmd(json!({"type":"grok_login"})).await;
                                                });
                                            }
                                        >
                                            "Login Grok"
                                        </button>
                                    </div>
                                }
                            }
                        >
                            <div class="card-body gate flex flex-1 flex-col items-center justify-center gap-4 text-center" data-testid="ai-gate">
                                <p class="text-base-content/70 m-0 max-w-[18rem] text-sm">"Enter this code in the xAI window. SuperGrok or X Premium."</p>
                                <div class="text-base-content font-mono text-4xl font-black tracking-widest" data-testid="ai-code">
                                    {move || st.get().grok.user_code.clone().unwrap_or_default()}
                                </div>
                                <a
                                    class="btn btn-outline btn-sm"
                                    data-testid="ai-open-xai"
                                    href=move || st.get().grok.verification_uri.clone().unwrap_or_else(|| "#".into())
                                    target="_blank"
                                    rel="noopener"
                                >
                                    "Open xAI"
                                </a>
                                <p class="text-base-content/55 m-0 px-3 py-2 text-center text-sm">"Waiting for login…"</p>
                            </div>
                        </Show>
                    }
                }
            >
                <div class="chat-log flex min-h-0 flex-1 flex-col gap-2 overflow-auto px-4 py-4" data-testid="ai-log">
                    <For
                        each=move || {
                            st.get()
                                .desk
                                .into_iter()
                                .enumerate()
                                .filter_map(|(i, line)| pane::chat_row(&line).map(|(k, t)| (i, k, t)))
                                .collect::<Vec<_>>()
                        }
                        key=|row| row.0
                        children=move |(_, kind, t)| {
                            if kind == "tool" {
                                view! { <pre class="bg-base-200 max-h-36 overflow-auto rounded-box p-2 font-mono text-xs whitespace-pre-wrap">{t}</pre> }.into_any()
                            } else {
                                let end = kind == "me";
                                view! {
                                    <div class=if end { "chat chat-end" } else { "chat chat-start" }>
                                        <div class=if end { "chat-bubble chat-bubble-primary" } else { "chat-bubble" }>{t}</div>
                                    </div>
                                }.into_any()
                            }
                        }
                    />
                </div>
                <form
                    class="flex w-full min-w-0 items-stretch gap-2 bg-base-100 px-4 py-3"
                    on:submit=move |ev| {
                        ev.prevent_default();
                        send_ai(st, draft);
                    }
                >
                    <input
                        id="ai-q"
                        class="input min-w-0 flex-1"
                        data-testid="ai-q"
                        autocomplete="off"
                        spellcheck="false"
                        on:mousedown=move |ev| {
                            ev.stop_propagation();
                            js_call("connectTermBlur", &JsValue::UNDEFINED);
                            if let Some(el) = ev
                                .target()
                                .and_then(|t| t.dyn_into::<web_sys::HtmlElement>().ok())
                            {
                                let _ = el.focus();
                            }
                        }
                        on:keydown=move |ev| ev.stop_propagation()
                        on:input=move |ev| draft.set(event_target_value(&ev))
                    />
                    <button class="btn btn-primary btn-square shrink-0" type="submit" data-testid="ai-send" aria-label="Send">{icons::send()}</button>
                </form>
            </Show>
        </aside>
    }
}
