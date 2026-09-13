//! Box-side agent. Joins a Teleport cluster and serves Chromium, a PTY, or a coding agent.

pub mod ai;
pub mod chrome;
pub mod code;
pub mod jpeg;
pub mod protocol;
pub mod rtc;
pub mod tools;
pub mod video;

pub use connect2_teleport::{identity, join, plane, shell, teleport, user};

#[cfg(feature = "web")]
pub mod hub;
