//! Independent anti-spam core for Telegram and community bots.
//!
//! Pure text processing, profile/message scoring, and optional reputation
//! clients. Snapshot loading, Telegram actions, LLM calls, persistence, and
//! enforcement authorization belong to the consuming application.

#[cfg(feature = "scoring")]
pub mod assessment;
pub mod calibration;
pub mod external;
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
