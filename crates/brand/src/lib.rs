//! The product's identity, in one place.
//!
//! Qanloom is a fork of [WordCraft](https://github.com/storytold/wordcraft) by the ArtCraft team
//! and community. Code keeps WordCraft's internal names (crates, commands, environment variables)
//! so upstream changes merge cleanly; everything a user or the operating system sees comes from
//! here. WordCraft's own name stays only where it states the origin and in copyright notices.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::borrow::Cow;

/// The product name shown to people.
pub const NAME: &str = "Tavru Docs";
/// Reverse-DNS application id: desktop file, icon name, window class, Flatpak id.
pub const APP_ID: &str = "io.github.tomblanchard312.Nisaba";
/// The executable name packages install.
pub const EXECUTABLE: &str = "nisaba";
/// Settings folder name on Linux and the BSDs (`$XDG_CONFIG_HOME/<this>`).
pub const SETTINGS_DIR_XDG: &str = "nisaba";
/// Settings folder name on macOS (`~/Library/Application Support/<this>`) and Windows (`%APPDATA%\<this>`).
pub const SETTINGS_DIR: &str = "Nisaba";
/// Where people find the project.
pub const HOMEPAGE: &str = "https://github.com/tomblanchard312/Qanloom";
/// The project Qanloom is based on, stated wherever its origin is.
/// The writing assistant's functional action label, as a macro so `concat!` can build command labels from it.
/// Users only ever see this name: the local model service behind it (SharedAI) stays out of the
/// interface.
#[macro_export]
macro_rules! assistant_name {
    () => {
        "Ask"
    };
}
/// The writing assistant's name (see [`assistant_name!`]).
pub const ASSISTANT_NAME: &str = assistant_name!();

pub const UPSTREAM_NAME: &str = "WordCraft";
pub const UPSTREAM_HOMEPAGE: &str = "https://github.com/storytold/wordcraft";
/// Licence and copyright line. The upstream notice must stay (MIT/Apache-2.0).
pub const COPYRIGHT: &str = "MIT OR Apache-2.0. Copyright (c) 2026 ArtCraft Team and the WordCraft contributors; Qanloom (formerly DraftHearth/Nisaba) changes copyright (c) 2026 Tom Blanchard.";

/// Text inherited from upstream with WordCraft's name replaced by [`NAME`]. Applied to every
/// interface string (all languages keep the product name untranslated), so upstream strings
/// added later are branded too.
pub fn branded(text: &str) -> Cow<'_, str> {
    if text.contains(UPSTREAM_NAME) { Cow::Owned(text.replace(UPSTREAM_NAME, NAME)) } else { Cow::Borrowed(text) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branding_replaces_only_the_product_name() {
        assert_eq!(branded("Show the Start screen when WordCraft starts"), "Show the Start screen when Tavru Docs starts");
        assert_eq!(branded("WordCraft User"), "Tavru Docs User");
        assert!(matches!(branded("Save As"), Cow::Borrowed("Save As")));
        assert_eq!(ASSISTANT_NAME, "Ask");
        assert!(APP_ID.split('.').count() >= 3 && !APP_ID.contains("storyteller"));
        assert!(COPYRIGHT.contains("ArtCraft Team and the WordCraft contributors"), "the upstream notice stays");
    }
}
