//! Locks that keep working after a panic.
//!
//! A thread that panics while it holds a `Mutex` marks it poisoned, and every `lock().unwrap()`
//! after that panics too: one mistake in a background loop would take the server monitor, the bug
//! reports and the settings down with it for the rest of the session. The data behind DropLaunch's
//! locks stays usable (settings, the server's state), so the lock is simply taken again.

use std::sync::{Mutex, MutexGuard, PoisonError};

pub trait Locked<T: ?Sized> {
    fn locked(&self) -> MutexGuard<'_, T>;
}

impl<T: ?Sized> Locked<T> for Mutex<T> {
    fn locked(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Run one round of a background loop; a panic in it is logged (by the panic hook) and the loop
/// goes on with the next round. True: the round went fine.
pub fn survive(round: impl FnOnce()) -> bool {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(round)).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn a_poisoned_lock_is_taken_again() {
        let value = Arc::new(Mutex::new(1));
        let inside = value.clone();
        let _ = std::thread::spawn(move || {
            let _guard = inside.locked();
            panic!("on purpose");
        })
        .join();
        assert!(value.lock().is_err(), "poisoned");
        *value.locked() += 1;
        assert_eq!(*value.locked(), 2);
        assert!(!survive(|| panic!("on purpose")));
        assert!(survive(|| {}));
    }
}
