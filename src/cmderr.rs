//! Why a command failed, and whether the firmware may still act on it.
//! Kept apart from `cmd.rs` (no UEFI) so the host tests can check it.

use crate::layout;

/// How long a command may take. INIT_HCA and TEARDOWN_HCA take seconds on
/// some firmware; the PRM gives no bound.
pub const TIMEOUT_MS: u32 = 60_000;

/// Why a command failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

impl Error {
    /// The command is over and was not carried out: the firmware gave a
    /// delivery or command status (PRM 7.14.1: it handed the entry back,
    /// Table 96/102), or it was never posted. False when that is unknown:
    /// a timeout leaves the entry hardware-owned, and a failed doorbell
    /// write may still have reached the device, so the firmware may yet
    /// act on the command (and DMA to what it names).
    pub fn refused(&self) -> bool {
        match self {
            Error::Delivery(_) | Error::Status(..) | Error::TooLong => true,
            Error::Timeout | Error::Pci => false,
        }
    }
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
