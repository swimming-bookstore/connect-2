//! Teleport sign-in.

use crate::hub::cmd;
use crate::state::State;
use leptos::prelude::*;
use serde_json::json;

fn otp_needed(factor: &str) -> bool {
    matches!(
        factor,
        "otp" | "totp" | "on" | "optional" | "true" | "webauthn"
    )
}

fn sso_label(c: &crate::state::Connector) -> String {
    if c.display.is_empty() {
        format!("Sign in with {}", c.name)
    } else {
        format!("Sign in with {}", c.display)
    }
}

#[component]
pub fn ClusterGate(st: RwSignal<State>) -> impl IntoView {
    let user = RwSignal::new(String::new());
    let password = RwSignal::new(String::new());
    let otp = RwSignal::new(String::new());
    view! {
        <div class="hero welcome min-h-0 flex-1" data-testid="cluster-gate">
            <div class="hero-content welcome-inner w-full max-w-sm flex-col text-center">
                <h1 class="text-base-content m-0 text-5xl font-extrabold tracking-tight">"Sign in"</h1>
                <p class="lede text-base-content/70 m-0 text-base">
                    {move || {
                        let s = st.get();
                        if s.cluster.is_empty() {
                            "Teleport cluster".into()
                        } else {
                            format!("Teleport · {}", s.cluster)
                        }
                    }}
                </p>
                <Show when=move || !st.get().login_err.is_empty()>
                    <p class="text-error m-0 text-sm" data-testid="cluster-err">{move || st.get().login_err}</p>
                </Show>
                <Show
                    when=move || st.get().login_wait
                    fallback=move || {
                        view! {
                            <Show when=move || !st.get().connectors.is_empty()>
                                <div class="flex w-full flex-col gap-2" data-testid="cluster-sso">
                                    <For
                                        each=move || st.get().connectors
                                        key=|c| format!("{}:{}", c.kind, c.name)
                                        children=move |c| {
                                            let auth = c.kind.clone();
                                            let connector = c.name.clone();
                                            let label = sso_label(&c);
                                            view! {
                                                <button
                                                    class="btn btn-primary w-full"
                                                    type="button"
                                                    data-testid="cluster-sso-btn"
                                                    on:click=move |_| {
                                                        let auth = auth.clone();
                                                        let connector = connector.clone();
                                                        leptos::task::spawn_local(async move {
                                                            cmd(json!({
                                                                "type": "cluster_login",
                                                                "auth": auth,
                                                                "connector": connector,
                                                            })).await;
                                                        });
                                                    }
                                                >
                                                    {label}
                                                </button>
                                            }
                                        }
                                    />
                                </div>
                            </Show>
                            <form
                                class="flex w-full flex-col gap-3 text-left"
                                data-testid="cluster-login-form"
                                on:submit=move |ev| {
                                    ev.prevent_default();
                                    let user = user.get();
                                    let password = password.get();
                                    let otp = otp.get();
                                    leptos::task::spawn_local(async move {
                                        cmd(json!({
                                            "type": "cluster_login",
                                            "auth": "local",
                                            "user": user,
                                            "password": password,
                                            "otp": otp,
                                        })).await;
                                    });
                                }
                            >
                                <label class="form-control w-full">
                                    <span class="label"><span class="label-text">"User"</span></span>
                                    <input
                                        class="input input-bordered w-full"
                                        type="text"
                                        autocomplete="username"
                                        data-testid="cluster-user"
                                        prop:value=move || user.get()
                                        on:input=move |ev| user.set(event_target_value(&ev))
                                    />
                                </label>
                                <label class="form-control w-full">
                                    <span class="label"><span class="label-text">"Password"</span></span>
                                    <input
                                        class="input input-bordered w-full"
                                        type="password"
                                        autocomplete="current-password"
                                        data-testid="cluster-password"
                                        prop:value=move || password.get()
                                        on:input=move |ev| password.set(event_target_value(&ev))
                                    />
                                </label>
                                <Show when=move || otp_needed(&st.get().second_factor) || st.get().second_factor.is_empty()>
                                    <label class="form-control w-full">
                                        <span class="label"><span class="label-text">"OTP"</span></span>
                                        <input
                                            class="input input-bordered w-full"
                                            type="text"
                                            inputmode="numeric"
                                            autocomplete="one-time-code"
                                            data-testid="cluster-otp"
                                            prop:value=move || otp.get()
                                            on:input=move |ev| otp.set(event_target_value(&ev))
                                        />
                                    </label>
                                </Show>
                                <button class="btn btn-primary" type="submit" data-testid="cluster-login">
                                    "Sign in"
                                </button>
                            </form>
                        }
                    }
                >
                    <p class="text-base-content/70 m-0 max-w-[18rem] text-sm">"Finish SSO in the browser. This app waits for certs."</p>
                    <Show when=move || !st.get().login_url.is_empty()>
                        <a
                            class="btn btn-primary"
                            data-testid="cluster-open"
                            href=move || st.get().login_url.clone()
                            target="_blank"
                            rel="noopener"
                        >
                            "Open SSO"
                        </a>
                    </Show>
                    <p class="text-base-content/55 m-0 text-sm">"Waiting for login…"</p>
                </Show>
            </div>
        </div>
    }
}
