//! `src/cmderr.rs`: which command failures prove the firmware did not act
//! (#18, finding 21b5b4e705e4). Run by `scripts/test-host.sh`.
#[allow(dead_code)]
#[path = "../src/layout.rs"]
mod layout;
#[path = "../src/cmderr.rs"]
mod cmderr;
use cmderr::Error;

#[test]
fn a_completed_or_unsent_command_is_a_refusal() {
    // The firmware handed the entry back with a delivery or command status,
    // or the command was never posted: what it named may be freed.
    assert!(Error::Delivery(0x01).refused());
    assert!(Error::Status(0x03, 0x1234_5678).refused());
    assert!(Error::TooLong.refused());
}

#[test]
fn a_timeout_or_failed_doorbell_is_not() {
    // The entry may still be the firmware's: MANAGE_PAGES' pages must stay
    // allocated (give_pages keeps them as lent).
    assert!(!Error::Timeout.refused());
    assert!(!Error::Pci.refused());
}

#[test]
fn errors_still_say_what_happened() {
    assert_eq!(format!("{}", Error::Timeout), "timed out after 60 s");
    assert_eq!(format!("{}", Error::Pci), "PCI I/O access failed");
    assert!(format!("{}", Error::Delivery(0x01)).starts_with("delivery status 0x1"));
}
