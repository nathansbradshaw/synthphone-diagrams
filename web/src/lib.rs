//! Shared Rust DSP, with a narrow AudioWorklet bridge. UI and plotting are Rust too.
mod engine;
#[cfg(any(test, all(target_arch = "wasm32", feature = "browser")))]
mod key;
mod midi;
pub use engine::Engine;
#[cfg(all(target_arch = "wasm32", feature = "browser"))]
mod browser;
#[cfg(all(target_arch = "wasm32", feature = "browser"))]
pub use browser::mount;
#[cfg(any(test, all(target_arch = "wasm32", feature = "browser")))]
mod scope_math;
#[cfg(all(target_arch = "wasm32", not(feature = "browser")))]
mod worklet;
