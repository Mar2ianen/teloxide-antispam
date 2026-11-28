//! Independent anti-spam core for Telegram and community bots.
//!
//! Pure text processing, profile/message scoring, and optional reputation
//! clients. Snapshot loading, Telegram actions, LLM calls, persistence, and
//! enforcement authorization belong to the consuming application.

#[cfg(feature = "scoring")]
pub mod assessment;
pub mod calibration;
pub mod categories;
#[cfg(feature = "embedding")]
pub mod embedding;
pub mod external;
/// Pure time-decayed primitives used by conversation flood detectors.
#[cfg(feature = "flood")]
pub mod flood;
#[cfg(feature = "classifier")]
pub mod logreg;
pub mod policy;
#[cfg(feature = "unicode")]
pub mod preprocess;
#[cfg(feature = "scoring")]
pub mod scoring;
#[cfg(feature = "scoring")]
pub mod signals;
pub mod text;

/// Optional standalone URL and Telegram reference parser.
#[cfg(feature = "url")]
pub use teloxide_url as url;

/// Advisory URL evidence interpretation; never affects enforcement itself.
#[cfg(feature = "url")]
pub mod url_shadow;
