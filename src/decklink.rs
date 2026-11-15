//! DeckLink SDI output: the card's frame clock drives the scheduler.

use crate::op47;
use crate::page::Packet;
use crate::service::Service;
use std::ffi::{CString, c_void};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use tokio::sync::broadcast;

type FillFn = extern "C" fn(*mut c_void, *mut u8, usize) -> usize;

unsafe extern "C" {
    fn ttx_decklink_start(
        device: *const std::ffi::c_char,
        did: u8,
        sdid: u8,
        line: u32,
        fill: FillFn,
        user: *mut c_void,
    ) -> i32;
}

struct Ctx {
    svc: Arc<Mutex<Service>>,
    tx: broadcast::Sender<Vec<Packet>>,
    lines: [u8; 5],
    seq: u16,
}

extern "C" fn fill(user: *mut c_void, buf: *mut u8, cap: usize) -> usize {
    // SAFETY: `user` is the leaked Ctx from `start`, only used from the SDK's callback thread.
    let ctx = unsafe { &mut *(user as *mut Ctx) };
    let packets = match ctx.svc.lock() {
        Ok(mut s) => s.next_frame(SystemTime::now()),
        Err(_) => return 0,
    };
    let sdp = op47::sdp(&packets, &ctx.lines, ctx.seq);
    ctx.seq = ctx.seq.wrapping_add(1);
    let _ = ctx.tx.send(packets);
    let n = sdp.len().min(cap);
    // SAFETY: the shim passes a buffer of `cap` bytes.
    unsafe { std::ptr::copy_nonoverlapping(sdp.as_ptr(), buf, n) };
    n
}

/// Start playback on the named device. Runs until the process exits.
pub fn start(
    device: &str,
    vanc_line: u32,
    lines: [u8; 5],
    svc: Arc<Mutex<Service>>,
    tx: broadcast::Sender<Vec<Packet>>,
) -> anyhow::Result<()> {
    let ctx = Box::into_raw(Box::new(Ctx {
        svc,
        tx,
        lines,
        seq: 0,
    }));
    let name = CString::new(device)?;
    // SAFETY: valid C string and callback; ctx lives for the rest of the process.
    let r = unsafe {
        ttx_decklink_start(
            name.as_ptr(),
            op47::DID,
            op47::SDID,
            vanc_line,
            fill,
            ctx as *mut c_void,
        )
    };
    if r != 0 {
        anyhow::bail!("DeckLink output on {device:?} failed at step {}", -r);
    }
    Ok(())
}
