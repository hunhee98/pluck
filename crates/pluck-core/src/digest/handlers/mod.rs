//! Per-tool digest handlers.
//!
//! Each handler exposes `pub fn digest(input: &str) -> String` and
//! is dispatched from [`super::digest`] by [`super::Format`]. Handlers
//! never touch global state, never panic on malformed input, and
//! must preserve every diagnostic an agent would act on
//! (file:line:col, traceback, panic stack, failed-step body).

pub mod cargo;
pub mod gha;
pub mod npm;
pub mod pytest;
