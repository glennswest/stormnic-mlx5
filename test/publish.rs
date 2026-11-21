//! `src/publish.rs`: a child is published and withdrawn all or nothing, and
//! never freed while the firmware holds its interfaces (#19, findings
//! aa46b24c4f4f and 5d95b0b4f0e3). Run by `scripts/test-host.sh`.
#[path = "../src/publish.rs"]
mod publish;
use publish::{Failed, Firmware, State};

/// A firmware whose steps fail on request, recording what it holds.
#[derive(Default)]
struct Fw {
    fail_event: bool,
    fail_install: bool,
    fail_uninstall: bool,
    fail_by_child: bool,
    event: bool,
    installed: bool,
    by_child: bool,
}

impl Firmware for Fw {
    type Status = &'static str;
    fn create_event(&mut self) -> Result<(), &'static str> {
        if self.fail_event {
            return Err("event");
        }
        self.event = true;
        Ok(())
    }
    fn close_event(&mut self) {
        assert!(!self.installed, "WaitForPacket closed while the SNP naming it is installed");
        self.event = false;
    }
    fn install(&mut self) -> Result<(), &'static str> {
        if self.fail_install {
            return Err("install");
        }
        self.installed = true;
        Ok(())
    }
    fn uninstall(&mut self) -> Result<(), &'static str> {
        if self.fail_uninstall {
            return Err("in use");
        }
        self.installed = false;
        Ok(())
    }
    fn open_by_child(&mut self) -> Result<(), &'static str> {
        if self.fail_by_child {
            return Err("by child");
        }
        self.by_child = true;
        Ok(())
    }
    fn close_by_child(&mut self) {
        self.by_child = false;
    }
}

fn held(f: &Fw) -> State {
    State { event: f.event, installed: f.installed, by_child: f.by_child }
}

#[test]
fn install_then_uninstall() {
    let (mut f, mut s) = (Fw::default(), State::default());
    assert_eq!(publish::install(&mut f, &mut s), Ok(()));
    assert_eq!(s, State { event: true, installed: true, by_child: true });
    assert_eq!(s, held(&f));
    assert_eq!(publish::uninstall(&mut f, &mut s), Ok(()));
    assert!(s.free());
    assert_eq!(s, held(&f));
}

#[test]
fn a_failed_by_child_open_fails_the_install() {
    // 5d95b0b4f0e3: no Ok with a child outside the driver model; what was
    // installed is removed again and the child may be freed.
    let (mut f, mut s) = (Fw { fail_by_child: true, ..Fw::default() }, State::default());
    assert_eq!(publish::install(&mut f, &mut s), Err(Failed { status: "by child", kept: false }));
    assert!(s.free());
    assert_eq!(s, held(&f));
}

#[test]
fn an_install_that_cannot_be_undone_keeps_the_child() {
    let mut f = Fw { fail_by_child: true, fail_uninstall: true, ..Fw::default() };
    let mut s = State::default();
    assert_eq!(publish::install(&mut f, &mut s), Err(Failed { status: "by child", kept: true }));
    assert!(!s.free());
    assert_eq!(s, held(&f));
}

#[test]
fn early_failures_leave_nothing() {
    for f in [Fw { fail_event: true, ..Fw::default() }, Fw { fail_install: true, ..Fw::default() }] {
        let (mut f, mut s) = (f, State::default());
        let r = publish::install(&mut f, &mut s);
        assert!(matches!(r, Err(Failed { kept: false, .. })));
        assert!(s.free());
        assert_eq!(s, held(&f));
    }
}

#[test]
fn a_refused_uninstall_keeps_the_child_as_it_was() {
    // aa46b24c4f4f: the interfaces stay, so the child is not freed, its
    // event stays open and BY_CHILD_CONTROLLER is opened again.
    let (mut f, mut s) = (Fw::default(), State::default());
    publish::install(&mut f, &mut s).unwrap();
    f.fail_uninstall = true;
    assert_eq!(publish::uninstall(&mut f, &mut s), Err("in use"));
    assert_eq!(s, State { event: true, installed: true, by_child: true });
    assert_eq!(s, held(&f));
    assert!(!s.free());
    // Once the consumer lets go, Stop withdraws it.
    f.fail_uninstall = false;
    assert_eq!(publish::uninstall(&mut f, &mut s), Ok(()));
    assert!(s.free());
}

#[test]
fn a_kept_child_is_withdrawn_later() {
    // Kept by a failed install (no BY_CHILD open): Stop removes it without
    // closing what was never opened.
    let mut f = Fw { fail_by_child: true, fail_uninstall: true, ..Fw::default() };
    let mut s = State::default();
    let _ = publish::install(&mut f, &mut s);
    f.fail_uninstall = false;
    assert_eq!(publish::uninstall(&mut f, &mut s), Ok(()));
    assert!(s.free());
    assert_eq!(s, held(&f));
}
