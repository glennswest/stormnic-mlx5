//! The order a child handle is published and withdrawn in, all or nothing
//! (#19, findings aa46b24c4f4f and 5d95b0b4f0e3). No UEFI here, so the host
//! tests can drive it with a firmware of their own (`test/publish.rs`);
//! `snp.rs` supplies the boot services.
//!
//! A child is three things: its `WaitForPacket` event, its interfaces (device
//! path and SNP, installed and uninstalled together by
//! Install/UninstallMultipleProtocolInterfaces, which put back what they took
//! when one fails), and the parent's PCI I/O opened `BY_CHILD_CONTROLLER`.
//! While its interfaces are installed, the firmware and every consumer hold
//! pointers into the child, so it is never freed then: an install that cannot
//! be undone, or an uninstall the firmware refuses (UEFI 2.10 7.3:
//! UninstallProtocolInterface fails while a driver keeps the interface open),
//! leaves the child kept and says so.

/// The boot services a child's publication needs.
pub trait Firmware {
    type Status: Copy;
    fn create_event(&mut self) -> Result<(), Self::Status>;
    fn close_event(&mut self);
    /// Device path and SNP together: both installed, or neither.
    fn install(&mut self) -> Result<(), Self::Status>;
    /// Device path and SNP together: both removed, or neither.
    fn uninstall(&mut self) -> Result<(), Self::Status>;
    fn open_by_child(&mut self) -> Result<(), Self::Status>;
    fn close_by_child(&mut self);
}

/// What of a child is in place.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub event: bool,
    pub installed: bool,
    pub by_child: bool,
}

impl State {
    /// Nothing the firmware holds points into the child: it may be freed.
    pub fn free(&self) -> bool {
        !self.event && !self.installed
    }
}

/// A failed install: the status that failed it, and whether the child must
/// be kept (its interfaces stay installed because removing them failed too).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Failed<S> {
    pub status: S,
    pub kept: bool,
}

/// Publish a child: event, interfaces, `BY_CHILD_CONTROLLER`. A failed step
/// undoes the ones before it; Ok only when all three are in place.
pub fn install<F: Firmware>(f: &mut F, s: &mut State) -> Result<(), Failed<F::Status>> {
    let fail = |status, s: &State| Err(Failed { status, kept: !s.free() });
    if let Err(st) = f.create_event() {
        return fail(st, s);
    }
    s.event = true;
    if let Err(st) = f.install() {
        f.close_event();
        s.event = false;
        return fail(st, s);
    }
    s.installed = true;
    if let Err(st) = f.open_by_child() {
        // Without it the child is not the controller's in the driver model:
        // the install fails (5d95b0b4f0e3).
        let _ = withdraw(f, s);
        return fail(st, s);
    }
    s.by_child = true;
    Ok(())
}

/// Withdraw a child: `BY_CHILD_CONTROLLER` closed, interfaces removed, event
/// closed. On a refusal the child is left published as it was (`BY_CHILD`
/// opened again if it had been) and the status returned: it must be kept.
pub fn uninstall<F: Firmware>(f: &mut F, s: &mut State) -> Result<(), F::Status> {
    let had = s.by_child;
    if had {
        f.close_by_child();
        s.by_child = false;
    }
    let r = withdraw(f, s);
    if r.is_err() && had && f.open_by_child().is_ok() {
        s.by_child = true;
    }
    r
}

/// Interfaces, then the event; the event stays while the interfaces do,
/// since the SNP names it.
fn withdraw<F: Firmware>(f: &mut F, s: &mut State) -> Result<(), F::Status> {
    if s.installed {
        f.uninstall()?;
        s.installed = false;
    }
    if s.event {
        f.close_event();
        s.event = false;
    }
    Ok(())
}
