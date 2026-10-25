//! Everything that talks to SteamCMD.

pub mod appinfo;
pub mod install;
pub mod manifest;
pub mod parser;
pub mod runner;
pub mod vdf;
pub mod vt;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub const RUST_SERVER_APP_ID: &str = "258550";

/// Shared "stop" flag for a running job; the runner polls it every 100 ms.
#[derive(Clone, Default, Debug)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}
