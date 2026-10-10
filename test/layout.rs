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

#[test]
fn minimum_frame_is_all_inline_in_two_wqebbs() {
    let frame: Vec<u8> = (0..60u8).collect();
    let mut w = [0u8; wqe::MAX];
    let bbs = wqe::send(&mut w, 0x1234, 0x56, &frame, 0x100, 0x1000);
    // ctrl + eth (1 + ceil(58/16) = 5) = 6 octowords → 2 WQEBBs.
    assert_eq!(get(&w, 0x04, 5, 0), 6);
    assert_eq!(bbs, 2);
    assert_eq!(get(&w, 0x00, 23, 8), 0x1234);
    assert_eq!(get(&w, 0x00, 7, 0), 0x0a);
    assert_eq!(get(&w, 0x04, 31, 8), 0x56);
    assert_eq!(w[0x0b], 0x08);
    assert_eq!(get(&w, 0x1c, 25, 16), 60);
    assert_eq!(&w[0x1e..0x1e + 60], &frame[..]);
}

#[test]
fn small_frame_fits_one_wqebb() {
    let frame = [0xaau8; 18];
    let mut w = [0u8; wqe::MAX];
    // ctrl + eth (1 + 1) = 3 octowords.
    assert_eq!(wqe::send(&mut w, 0, 1, &frame, 0, 0), 1);
    assert_eq!(get(&w, 0x04, 5, 0), 3);
}

#[test]
fn long_frame_inlines_the_headers_and_points_at_the_rest() {
    let frame = vec![0x55u8; 1514];
    let mut w = [0u8; wqe::MAX];
    let bbs = wqe::send(&mut w, 7, 9, &frame, 0xabcd, 0x2000);
    assert_eq!(bbs, 2);
    assert_eq!(get(&w, 0x04, 5, 0), 8);
    assert_eq!(get(&w, 0x1c, 25, 16), wqe::INLINE_MAX as u32);
    // The pointer segment is the eighth octoword.
    assert_eq!(get(&w, 0x70, 31, 31), 0);
    assert_eq!(get(&w, 0x70, 30, 0), (1514 - wqe::INLINE_MAX) as u32);
    assert_eq!(get(&w, 0x74, 31, 0), 0xabcd);
    assert_eq!(u64::from_be_bytes(w[0x78..0x80].try_into().unwrap()), 0x2000 + wqe::INLINE_MAX as u64);
}

#[test]
fn cqe_ownership_follows_the_wrap_bit() {
    // Before the device writes anything, nothing is ready.
    assert!(!cqe::ready(cqe::INIT, 0, 4));
    // First pass: owner 0 is software's.
    assert!(cqe::ready(0x20, 3, 4));
    assert!(!cqe::ready(0x21, 3, 4));
    // Second pass (counter 16..31 with 16 entries): owner 1.
    assert!(cqe::ready(0x21, 16, 4));
    assert!(!cqe::ready(0x20, 16, 4));
    // An invalid opcode never is.
    assert!(!cqe::ready(0xf0, 0, 4));
    assert!(!cqe::eqe_ready(1, 0, 6));
    assert!(cqe::eqe_ready(0, 0, 6));
    assert!(cqe::eqe_ready(1, 64, 6));
}

#[test]
fn speeds() {
    assert_eq!(speed_name(1 << 13), "10G");
    assert_eq!(speed_name(1 << 27), "25G");
    assert_eq!(speed_name(1 << 1), "1G");
    assert_eq!(speed_name(0), "");
}
