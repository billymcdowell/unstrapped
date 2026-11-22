//! Keep macOS from App-Napping the process while a sync runs (NSProcessInfo beginActivity).

#[cfg(target_os = "macos")]
mod imp {
    use objc2::rc::Retained;
    use objc2::runtime::ProtocolObject;
    use objc2_foundation::{NSActivityOptions, NSObjectProtocol, NSProcessInfo, NSString};

    pub struct NapGuard(Retained<ProtocolObject<dyn NSObjectProtocol>>);

    // The activity token is an opaque, thread-safe Objective-C object; we only hand it back to NSProcessInfo.
    unsafe impl Send for NapGuard {}
    unsafe impl Sync for NapGuard {}

    impl NapGuard {
        pub fn begin(reason: &str) -> Option<Self> {
            let info = NSProcessInfo::processInfo();
            let reason = NSString::from_str(reason);
            let token = info.beginActivityWithOptions_reason(NSActivityOptions::UserInitiatedAllowingIdleSystemSleep, &reason);
            Some(NapGuard(token))
        }
    }

    impl Drop for NapGuard {
        fn drop(&mut self) {
            unsafe { NSProcessInfo::processInfo().endActivity(&self.0) };
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    pub struct NapGuard;
    impl NapGuard {
        pub fn begin(_reason: &str) -> Option<Self> {
            Some(NapGuard)
        }
    }
}

pub use imp::NapGuard;
