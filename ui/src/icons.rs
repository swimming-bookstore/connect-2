//! Lucide (MIT) marks.

use leptos::prelude::*;

pub fn lucide(inner: AnyView) -> AnyView {
    view! {
        <svg
            class="h-5 w-5"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
        >
            {inner}
        </svg>
    }
    .into_any()
}

pub fn sun() -> AnyView {
    lucide(
        view! {
            <circle cx="12" cy="12" r="4"></circle>
            <path d="M12 2v2"></path>
            <path d="M12 20v2"></path>
            <path d="m4.93 4.93 1.41 1.41"></path>
            <path d="m17.66 17.66 1.41 1.41"></path>
            <path d="M2 12h2"></path>
            <path d="M20 12h2"></path>
            <path d="m6.34 17.66-1.41 1.41"></path>
            <path d="m19.07 4.93-1.41 1.41"></path>
        }
        .into_any(),
    )
}

pub fn moon() -> AnyView {
    lucide(
        view! {
            <path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"></path>
        }
        .into_any(),
    )
}

pub fn menu() -> AnyView {
    lucide(
        view! {
            <path d="M4 5h16"></path>
            <path d="M4 12h16"></path>
            <path d="M4 19h16"></path>
        }
        .into_any(),
    )
}

pub fn x() -> AnyView {
    lucide(
        view! {
            <path d="M18 6 6 18"></path>
            <path d="m6 6 12 12"></path>
        }
        .into_any(),
    )
}

pub fn shell() -> AnyView {
    lucide(
        view! {
            <path d="m4 17 6-6-6-6"></path>
            <path d="M12 19h8"></path>
        }
        .into_any(),
    )
}

pub fn browser() -> AnyView {
    lucide(
        view! {
            <circle cx="12" cy="12" r="10"></circle>
            <path d="M12 2a14.5 14.5 0 0 0 0 20 14.5 14.5 0 0 0 0-20"></path>
            <path d="M2 12h20"></path>
        }
        .into_any(),
    )
}

pub fn agent() -> AnyView {
    lucide(
        view! {
            <path d="M12 8V4H8"></path>
            <rect width="16" height="12" x="4" y="8" rx="2"></rect>
            <path d="M2 14h2"></path>
            <path d="M20 14h2"></path>
            <path d="M15 13v2"></path>
            <path d="M9 13v2"></path>
        }
        .into_any(),
    )
}

pub fn new_chat() -> AnyView {
    lucide(
        view! {
            <path d="M22 17a2 2 0 0 1-2 2H6.828a2 2 0 0 0-1.414.586l-2.202 2.202A.71.71 0 0 1 2 21.286V5a2 2 0 0 1 2-2h16a2 2 0 0 1 2 2z"></path>
            <path d="M12 8v6"></path>
            <path d="M9 11h6"></path>
        }
        .into_any(),
    )
}

pub fn send() -> AnyView {
    lucide(
        view! {
            <path d="m5 12 7-7 7 7"></path>
            <path d="M12 19V5"></path>
        }
        .into_any(),
    )
}

pub fn boxes() -> AnyView {
    lucide(
        view! {
            <rect width="7" height="7" x="3" y="3" rx="1"></rect>
            <rect width="7" height="7" x="14" y="3" rx="1"></rect>
            <rect width="7" height="7" x="14" y="14" rx="1"></rect>
            <rect width="7" height="7" x="3" y="14" rx="1"></rect>
        }
        .into_any(),
    )
}

pub fn ai() -> AnyView {
    lucide(
        view! {
            <path d="M12 3v3"></path>
            <path d="M18.5 5.5 16 8"></path>
            <path d="M21 12h-3"></path>
            <path d="M18.5 18.5 16 16"></path>
            <path d="M12 21v-3"></path>
            <path d="M5.5 18.5 8 16"></path>
            <path d="M3 12h3"></path>
            <path d="M5.5 5.5 8 8"></path>
            <circle cx="12" cy="12" r="4"></circle>
        }
        .into_any(),
    )
}

pub fn room() -> AnyView {
    lucide(
        view! {
            <rect width="18" height="18" x="3" y="3" rx="2"></rect>
            <path d="M3 9h18"></path>
            <path d="M9 21V9"></path>
        }
        .into_any(),
    )
}

pub fn square(on: bool) -> &'static str {
    if on {
        "btn btn-ghost btn-square btn-sm on"
    } else {
        "btn btn-ghost btn-square btn-sm"
    }
}
