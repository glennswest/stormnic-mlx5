//! `src/layout.rs`: bit fields, command headers and signatures (PRM 7.14).
//! Run by `scripts/test-host.sh`.
#[allow(dead_code)]
#[path = "../src/layout.rs"]
mod layout;
use layout::*;

fn xor(b: &[u8]) -> u8 {
    b.iter().fold(0, |a, &x| a ^ x)
}

#[test]
fn fields_are_big_endian_dword_bits() {
    let mut b = [0u8; 8];
    put(&mut b, 0, 31, 16, 0x0102);
    put(&mut b, 4, 15, 0, 0xabcd);
    assert_eq!(b, [0x01, 0x02, 0, 0, 0, 0, 0xab, 0xcd]);
    assert_eq!(get(&b, 0, 31, 16), 0x0102);
    assert_eq!(get(&b, 4, 7, 0), 0xcd);
    // A field leaves its neighbours alone and is truncated to its width.
    put(&mut b, 0, 3, 0, 0xff);
    assert_eq!(b[3], 0x0f);
    assert_eq!(get(&b, 0, 31, 16), 0x0102);
    put(&mut b, 0, 31, 0, 0xdead_beef);
    assert_eq!(get(&b, 0, 31, 0), 0xdead_beef);
}

#[test]
fn sixty_four_bit_values_are_high_dword_first() {
    let mut b = [0u8; 8];
    put64(&mut b, 0, 0x1122_3344_5566_7788);
    assert_eq!(b, [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88]);
    assert_eq!(get64(&b, 0), 0x1122_3344_5566_7788);
}

#[test]
fn command_header() {
    let mut b = [0u8; 16];
    header(&mut b, op::QUERY_HCA_CAP, 1);
    assert_eq!(&b[..8], &[0x01, 0x00, 0, 0, 0, 0, 0, 1]);
}

#[test]
fn entry_signature_makes_the_xor_ff() {
    let mut e = [0u8; entry::SIZE];
    e[entry::TYPE] = entry::TYPE_PCIE;
    e[entry::IN_LEN + 3] = 0x10;
    e[entry::IN_INLINE] = 0x01;
    e[entry::TOKEN] = 0x5a;
    e[entry::SIG] = sig::entry(&e);
    assert_eq!(xor(&e), 0xff);
}

#[test]
fn mailbox_signatures_make_both_xors_ff() {
    let mut m = [0u8; mbox::SIZE];
    for (i, b) in m[..mbox::DATA].iter_mut().enumerate() {
        *b = i as u8;
    }
    m[mbox::NEXT + 6] = 0x04;
    m[mbox::BLOCK_NUM + 3] = 2;
    m[mbox::TOKEN] = 0x33;
    let (ctrl, all) = sig::mailbox(&m);
    m[mbox::CTRL_SIG] = ctrl;
    m[mbox::SIG] = all;
    // The control part with the signature byte taken as 0, and the block.
    let mut c = m[mbox::DATA..].to_vec();
    *c.last_mut().unwrap() = 0;
    assert_eq!(xor(&c), 0xff);
    assert_eq!(xor(&m), 0xff);
}

#[test]
fn names() {
    assert_eq!(op_name(op::INIT_HCA), "INIT_HCA");
    assert_eq!(status_name(0x03), "BAD_PARAM");
    assert_eq!(delivery_name(0x1), "signature error");
    assert_eq!(health_name(0x7), "DEAD_IRISC");
}
