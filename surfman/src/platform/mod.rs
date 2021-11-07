// surfman/surfman/src/platform/mod.rs
//
//! Platform-specific backends.

pub mod generic;

#[cfg(android)]
pub mod android;
#[cfg(android)]
pub use android as default;

#[cfg(any(macos, ios))]
pub mod macos;
#[cfg(any(macos, ios))]
pub use macos::cgl as default;
#[cfg(any(macos, ios))]
pub use macos::system;

#[cfg(linux)]
pub mod unix;
#[cfg(linux)]
pub use unix::default;

#[cfg(windows)]
pub mod windows;
#[cfg(angle_default)]
pub use windows::angle as default;
#[cfg(all(windows, not(angle_default)))]
pub use windows::wgl as default;
