//! DeckLink SDI output: the card's frame clock calls back for each frame's OP-47 packet.

use crate::op47;
use crate::page::Packet;
use std::ffi::{CString, c_void};

type FillFn = extern "C" fn(*mut c_void, *mut u8, usize) -> usize;

unsafe extern "C" {
    fn ttx_decklink_start(
        device: *const std::ffi::c_char,
        did: u8,
        sdid: u8,
        line: u32,
        packets_per_frame: u32,
        fill: FillFn,
        user: *mut c_void,
    ) -> i32;
}

/// Called once per frame on the SDK's thread; returns that frame's packets (at most 5 per
/// OP-47 packet).
pub type NextFrame = Box<dyn FnMut() -> Vec<Packet> + Send>;

struct Ctx {
    next: NextFrame,
    lines: Vec<u8>,
    seq: u16,
    /// Wrap each SDP in an ANC Multipacket; the first packet's VANC line (packet k is
    /// on the line after k-1, matching the shim) for its line/field word.
    multipacket: Option<u8>,
}

extern "C" fn fill(user: *mut c_void, buf: *mut u8, cap: usize) -> usize {
    // SAFETY: `user` is the leaked Ctx from `start`, only used from the SDK's callback thread.
    let ctx = unsafe { &mut *(user as *mut Ctx) };
    let packets = (ctx.next)();
    // Each ANC packet's user data words, preceded by their count.
    let mut all = vec![];
    for (k, s) in op47::sdps(&packets, &ctx.lines, &mut ctx.seq)
        .into_iter()
        .enumerate()
    {
        let udw = match ctx.multipacket {
            Some(line) => op47::multipacket(&s, line + k as u8),
            None => s,
        };
        all.push(udw.len() as u8);
        all.extend(udw);
    }
    let n = all.len().min(cap);
    // SAFETY: the shim passes a buffer of `cap` bytes.
    unsafe { std::ptr::copy_nonoverlapping(all.as_ptr(), buf, n) };
    n
}

/// Start playback on the named device. Runs until the process exits. `lines` holds 5
/// descriptor lines per OP-47 packet. Packet k goes on VANC line `vanc_line + k`: encoders
/// that check EBU line order flag several OP-47 packets on one line as duplicates. With
/// `multipacket`, each SDP goes out wrapped in an ANC Multipacket (SDID 0x03).
pub fn start(
    device: &str,
    vanc_line: u32,
    lines: Vec<u8>,
    multipacket: bool,
    next: NextFrame,
) -> anyhow::Result<()> {
    let sdps = lines.len() / op47::MAX_PACKETS;
    anyhow::ensure!(
        (1..=crate::link::MAX_SDPS).contains(&sdps),
        "1-{} OP-47 packets per frame",
        crate::link::MAX_SDPS
    );
    let ctx = Box::into_raw(Box::new(Ctx {
        next,
        lines,
        seq: 0,
        multipacket: multipacket.then_some(vanc_line as u8),
    }));
    let name = CString::new(device)?;
    // SAFETY: valid C string and callback; ctx lives for the rest of the process.
    let r = unsafe {
        ttx_decklink_start(
            name.as_ptr(),
            op47::DID,
            if multipacket {
                op47::SDID_MULTIPACKET
            } else {
                op47::SDID
            },
            vanc_line,
            sdps as u32,
            fill,
            ctx as *mut c_void,
        )
    };
    if r != 0 {
        anyhow::bail!("DeckLink output on {device:?} failed at step {}", -r);
    }
    Ok(())
}
