//! The command interface (PRM 7.14, 8.2): one command queue entry, used
//! synchronously, with chains of mailbox blocks for the input and output
//! beyond the 16 bytes each carried inline. Completion is polled through the
//! entry's ownership bit (PRM 7.14.1); no EQ is mapped to the command queue.
//!
//! Nothing here allocates once `Cmdq::new` has returned, so the
//! ExitBootServices handler can still send commands.

use core::sync::atomic::{fence, Ordering};

use uefi::boot;

use crate::dma::{DmaBuf, Mem};
use crate::layout::{self, mbox, sig};
use crate::pci::PciIo;

/// The HCA BAR: the only BAR (PRM 8.1.2), init segment at offset 0.
pub const BAR: u8 = 0;

/// Initialization segment (PRM 4.3, Table 7/8), byte offsets in BAR 0.
pub mod init {
    pub const FW_REV: u64 = 0x00;
    pub const CMDIF_REV: u64 = 0x04;
    pub const CMDQ_HI: u64 = 0x10;
    pub const CMDQ_LO: u64 = 0x14;
    pub const DOORBELL: u64 = 0x18;
    pub const INITIALIZING: u64 = 0x1fc;
    pub const HEALTH: u64 = 0x1010;
}

/// Mailbox blocks kept for each direction: 4 KiB inline-plus-mailbox data
/// covers every command sent (QUERY_HCA_CAP's output, 0x1010 bytes, is the
/// largest; MANAGE_PAGES is sent in batches that fit).
const BLOCKS: usize = 16;
/// Block stride: 1 KiB, the alignment `next_pointer[31:10]` needs (Table 98).
const BLOCK_STRIDE: usize = 1024;

/// How long a command may take. INIT_HCA and TEARDOWN_HCA take seconds on
/// some firmware; the PRM gives no bound.
const TIMEOUT_MS: u32 = 60_000;

/// Largest command input and output this queue carries.
pub const MAX_IN: usize = 16 + BLOCKS * mbox::DATA;
pub const MAX_OUT: usize = MAX_IN;

/// Why a command failed.
#[derive(Clone, Copy, Debug)]
pub enum Error {
    /// The ownership bit stayed set (`TIMEOUT_MS`).
    Timeout,
    /// Delivery status in the entry (Table 96): the command never ran.
    Delivery(u8),
    /// Command status and syndrome in the output (Table 102, Table 128).
    Status(u8, u32),
    /// Too long for the mailboxes kept.
    TooLong,
    /// A PCI I/O access to the BAR failed.
    Pci,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Error::Timeout => write!(f, "timed out after {} s", TIMEOUT_MS / 1000),
            Error::Delivery(s) => write!(f, "delivery status {s:#x} ({})", layout::delivery_name(s)),
            Error::Status(s, syn) => write!(f, "status {s:#x} {} syndrome {syn:#010x}", layout::status_name(s)),
            Error::TooLong => write!(f, "too long for the mailboxes"),
            Error::Pci => write!(f, "PCI I/O access failed"),
        }
    }
}

pub struct Cmdq {
    /// The command queue page (4 KiB, aligned on its size, PRM 7.14.1).
    queue: DmaBuf,
    /// Input and output mailbox blocks, `BLOCKS` each.
    inbox: DmaBuf,
    outbox: DmaBuf,
    token: u8,
    /// Set once a command has timed out: the entry may still be the
    /// device's, so nothing more is posted.
    stuck: bool,
}

/// A big-endian dword of the init segment (PciIo reads little-endian).
pub fn read_be(pci: &mut PciIo, off: u64) -> Result<u32, Error> {
    pci.mem_read32(BAR, off).map(u32::from_be).map_err(|_| Error::Pci)
}

pub fn write_be(pci: &mut PciIo, off: u64, v: u32) -> Result<(), Error> {
    pci.mem_write32(BAR, off, v.to_be()).map_err(|_| Error::Pci)
}

impl Cmdq {
    /// Allocate the queue and mailboxes and give the queue's address to the
    /// device (PRM 8.2 step 2): high dword first, then the low dword with
    /// nic_interface 0 (full driver).
    pub fn new(pci: &mut PciIo) -> uefi::Result<Cmdq> {
        let queue = DmaBuf::new(pci, 4096)?;
        let inbox = DmaBuf::new(pci, BLOCKS * BLOCK_STRIDE)?;
        let outbox = DmaBuf::new(pci, BLOCKS * BLOCK_STRIDE)?;
        Ok(Cmdq { queue, inbox, outbox, token: 0, stuck: false })
    }

    pub fn queue_addr(&self) -> u64 {
        self.queue.dev
    }

    /// Unmap and free the queue and mailboxes.
    ///
    /// # Safety
    /// The device must no longer use the queue (DISABLE_HCA done, or reset).
    pub unsafe fn free(self, pci: &mut PciIo) {
        unsafe {
            self.queue.free(pci);
            self.inbox.free(pci);
            self.outbox.free(pci);
        }
    }

    pub fn stuck(&self) -> bool {
        self.stuck
    }

    fn block(m: &Mem, i: usize) -> Mem {
        m.slice(i * BLOCK_STRIDE, mbox::SIZE)
    }

    /// Lay `len` bytes out as a chain of blocks in `m`: next pointers,
    /// block numbers, token and signatures (PRM Table 97/98, 7.14.1.1.1).
    /// `data` fills them (None for an output chain, which is zeroed).
    fn chain(m: &Mem, blocks: usize, token: u8, data: Option<&[u8]>) {
        for i in 0..blocks {
            let b = Self::block(m, i);
            b.zero();
            if let Some(d) = data {
                let from = i * mbox::DATA;
                let n = (d.len() - from).min(mbox::DATA);
                b.write(0, &d[from..from + n]);
            }
            let next = if i + 1 < blocks { Self::block(m, i + 1).dev } else { 0 };
            b.set_be64(mbox::NEXT, next);
            b.set_be32(mbox::BLOCK_NUM, i as u32);
            b.set_u8(mbox::TOKEN, token);
            let mut raw = [0u8; mbox::SIZE];
            b.read(0, &mut raw);
            let (ctrl, all) = sig::mailbox(&raw);
            b.set_u8(mbox::CTRL_SIG, ctrl);
            if data.is_some() {
                b.set_u8(mbox::SIG, all);
            }
        }
    }

    /// Run one command: `input` is the whole command input (header first,
    /// PRM Table 99); `out` receives the whole output (Table 101), and its
    /// length is the output length asked for. Errors carry the delivery or
    /// command status; a non-zero command status is an error.
    pub fn exec(&mut self, pci: &mut PciIo, input: &[u8], out: &mut [u8]) -> Result<(), Error> {
        if self.stuck {
            return Err(Error::Timeout);
        }
        if input.len() > MAX_IN || out.len() > MAX_OUT || input.len() < 8 || out.len() < 8 {
            return Err(Error::TooLong);
        }
        self.token = self.token.wrapping_add(1).max(1);
        let token = self.token;
        let in_blocks = input.len().saturating_sub(16).div_ceil(mbox::DATA);
        let out_blocks = out.len().saturating_sub(16).div_ceil(mbox::DATA);
        if in_blocks > 0 {
            Self::chain(&self.inbox, in_blocks, token, Some(&input[16..]));
        }
        if out_blocks > 0 {
            Self::chain(&self.outbox, out_blocks, token, None);
        }

        // The entry (PRM Table 95/96).
        let e = self.queue.slice(0, layout::entry::SIZE);
        e.zero();
        e.set_u8(layout::entry::TYPE, layout::entry::TYPE_PCIE);
        e.set_be32(layout::entry::IN_LEN, input.len() as u32);
        e.set_be32(layout::entry::OUT_LEN, out.len() as u32);
        if in_blocks > 0 {
            e.set_be64(layout::entry::IN_PTR, self.inbox.dev);
        }
        if out_blocks > 0 {
            e.set_be64(layout::entry::OUT_PTR, self.outbox.dev);
        }
        e.write(layout::entry::IN_INLINE, &input[..input.len().min(16)]);
        e.set_u8(layout::entry::TOKEN, token);
        let mut raw = [0u8; layout::entry::SIZE];
        e.read(0, &mut raw);
        e.set_u8(layout::entry::SIG, sig::entry(&raw));
        e.set_u8(layout::entry::STATUS_OWN, 1);
        fence(Ordering::SeqCst);
        // Doorbell vector bit 0: entry 0 (Table 8, offset 18h).
        write_be(pci, init::DOORBELL, 1)?;

        let mut waited = 0;
        while e.u8(layout::entry::STATUS_OWN) & 1 != 0 {
            if waited >= TIMEOUT_MS * 10 {
                self.stuck = true;
                return Err(Error::Timeout);
            }
            boot::stall(100);
            waited += 1;
        }
        fence(Ordering::SeqCst);
        let delivery = e.u8(layout::entry::STATUS_OWN) >> 1;
        if delivery != 0 {
            return Err(Error::Delivery(delivery));
        }
        let first = out.len().min(16);
        e.read(layout::entry::OUT_INLINE, &mut out[..first]);
        for i in 0..out_blocks {
            let from = 16 + i * mbox::DATA;
            let n = (out.len() - from).min(mbox::DATA);
            Self::block(&self.outbox, i).read(0, &mut out[from..from + n]);
        }
        let status = out[0];
        if status != 0 {
            let syndrome = u32::from_be_bytes(out[4..8].try_into().unwrap());
            return Err(Error::Status(status, syndrome));
        }
        Ok(())
    }
}
