//! Settings page.

use crate::grok;
use crate::hub::{cmd, save_video, video_now};
use crate::pane::VideoPref;
use crate::state::State;
use leptos::prelude::*;
use serde_json::json;

#[component]
pub fn Settings(
    st: RwSignal<State>,
    pref: RwSignal<VideoPref>,
    on_close: impl Fn() + Copy + 'static,
) -> impl IntoView {
    let draft = RwSignal::new(video_now());
    view! {
        <div id="desk" class="page settings-page" data-testid="settings">
            <div class="wrap settings-wrap">
                <div class="settings-head">
                    <button class="btn btn-ghost btn-sm" type="button" data-testid="settings-back" on:click=move |_| on_close()>
                        {move || if st.get().dst.is_empty() { "Home" } else { "Back" }}
                    </button>
                    <div class="min-w-0">
                        <h1 class="text-base-content m-0 text-2xl font-bold tracking-tight">"Settings"</h1>
                        <p class="text-base-content/55 m-0 text-sm">"Browser and Grok for this laptop."</p>
                    </div>
                </div>
                <form
                    class="settings-form"
                    data-testid="video-settings"
                    on:submit=move |ev| {
                        ev.prevent_default();
                        let p = draft.get();
                        pref.set(p);
                        save_video(p);
                        on_close();
                    }
                >
                    <div class="card bg-base-100 border-base-300 border shadow-sm">
                        <div class="card-body gap-4 p-5">
                            <div>
                                <h2 class="card-title text-base">"Browser video"</h2>
                                <p class="text-base-content/55 m-0 text-sm">"How the remote tab is sent to this window."</p>
                            </div>
                            <label class="form-control w-full">
                                <span class="label">
                                    <span class="label-text">"Mode"</span>
                                </span>
                                <select
                                    id="video-mode"
                                    class="select select-bordered w-full"
                                    data-testid="video-mode"
                                    on:change=move |ev| {
                                        let v = event_target_value(&ev);
                                        draft.set(VideoPref::parse(&v));
                                    }
                                >
                                    <option value="jpeg" data-testid="video-jpeg" selected=move || draft.get() == VideoPref::Jpeg>"JPEG snapshots"</option>
                                    <option value="turn720" data-testid="video-turn720" selected=move || draft.get() == VideoPref::Turn720>"TURN 720p"</option>
                                    <option value="turn1080" data-testid="video-turn1080" selected=move || draft.get() == VideoPref::Turn1080>"TURN 1080p"</option>
                                </select>
                            </label>
                            <div class="card-actions justify-end">
                                <button class="btn btn-primary" type="submit" data-testid="video-save">"Save"</button>
                            </div>
                        </div>
                    </div>
                </form>
                <Show when=move || st.get().grok.configured>
                    <div class="card bg-base-100 border-base-300 border shadow-sm">
                        <div class="card-body gap-4 p-5">
                            <div>
                                <h2 class="card-title text-base">"Grok"</h2>
                                <p class="text-base-content/55 m-0 text-sm">"Signed in on this laptop."</p>
                            </div>
                            <div class="card-actions">
                                <button
                                    class="btn btn-outline btn-sm"
                                    type="button"
                                    data-testid="ai-logout"
                                    on:click=move |_| {
                                        grok::clear();
                                        st.update(|s| {
                                            s.grok.configured = false;
                                            s.grok.tokens = None;
                                            s.grok.user_code = None;
                                            s.grok.verification_uri = None;
                                        });
                                        leptos::task::spawn_local(async { cmd(json!({"type":"logout"})).await });
                                    }
                                >
                                    "Sign out"
                                </button>
                            </div>
                        </div>
                    </div>
                </Show>
            </div>
        </div>
    }
}

