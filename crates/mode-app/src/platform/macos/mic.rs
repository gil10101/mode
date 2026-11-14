//! Microphone authorization (08 §6.2, XC-37): TCC's answer for this process
//! before cpal opens the device, and the system prompt while it is
//! undecided. CoreAudio hands a refused process silence rather than an
//! error, so AVFoundation is asked first. No Tauri types; any thread.
//!
//! A dev run is an unbundled binary: TCC files it under the responsible
//! process, the terminal or editor that launched `cargo run`.

use std::sync::mpsc;
use std::time::Duration;

use block2::RcBlock;
use objc2::runtime::Bool;
use objc2::{class, msg_send};
use objc2_foundation::NSString;

use super::super::MicAccess;

#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {
    /// `AVMediaTypeAudio`.
    static AVMediaTypeAudio: &'static NSString;
}

/// `AVCaptureDevice.authorizationStatus(for: .audio)`.
pub fn access() -> MicAccess {
    // SAFETY: a class method of the linked framework, given its own constant;
    // the result is the `AVAuthorizationStatus` integer.
    let status: isize = unsafe {
        msg_send![class!(AVCaptureDevice), authorizationStatusForMediaType: AVMediaTypeAudio]
    };
    match status {
        1 => MicAccess::Restricted,
        2 => MicAccess::Denied,
        3 => MicAccess::Allowed,
        _ => MicAccess::Undecided,
    }
}

/// `AVCaptureDevice.requestAccess(for: .audio)`: the system prompt while the
/// status is undecided (the stored answer at once otherwise), waited for up
/// to `wait`. The completion handler runs on a system queue and hands its
/// answer back over the channel.
pub fn request(wait: Duration) -> Option<bool> {
    let (tx, rx) = mpsc::channel::<bool>();
    let handler = RcBlock::new(move |granted: Bool| {
        let _ = tx.send(granted.as_bool());
    });
    // SAFETY: AVFoundation retains the block until it has called it once;
    // the handler owns its sender, and a receiver dropped by the timeout
    // only makes that send fail.
    unsafe {
        let _: () = msg_send![
            class!(AVCaptureDevice),
            requestAccessForMediaType: AVMediaTypeAudio,
            completionHandler: &*handler
        ];
    }
    rx.recv_timeout(wait).ok()
}
