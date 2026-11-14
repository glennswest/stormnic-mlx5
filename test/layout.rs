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
    assert_eq!(speed_name(1 << 13), "10g");
    assert_eq!(speed_name(1 << 27), "25g");
    assert_eq!(speed_name(1 << 1), "1g");
    assert_eq!(speed_name(0), "");
    // The highest speed in the mask wins, whatever its bit number.
    assert_eq!(speed_name(1 << 29 | 1 << 20), "100g");
    assert_eq!(speed_name(1 << 31 | 1 << 16), "50g");
}

#[test]
fn every_prm_table_108_bit_is_named_once() {
    let bits = [0, 1, 2, 3, 4, 6, 7, 12, 13, 14, 15, 16, 18, 20, 21, 22, 27, 28, 29, 30, 31];
    for b in bits {
        assert_eq!(PROTOS.iter().filter(|p| p.bit == b).count(), 1, "bit {b}");
    }
    assert_eq!(PROTOS.len(), bits.len());
    // Highest speed first, so `fastest` finds the top speed first.
    assert!(PROTOS.windows(2).all(|w| w[0].mbps >= w[1].mbps));
}

#[test]
fn standard_fec_per_media_and_speed() {
    // Owner, 2026-11-14: 25G RS; 10G/40G none; 50G/100G (NRZ) RS.
    for p in PROTOS {
        let want = match p.mbps {
            25_000 | 50_000 | 100_000 => Fec::Rs,
            _ => Fec::None,
        };
        assert_eq!(p.fec, want, "{}", p.media);
        assert!(p.media.starts_with(&p.speed[..p.speed.len() - 1]) || p.mbps == 1_000, "{}", p.media);
    }
    assert_eq!(Fec::Rs.name(), "rs");
    assert_eq!(Fec::None.name(), "none");
}

#[test]
fn media_from_oper_when_up_else_capability() {
    // ConnectX-4 Lx: capability 1G..25G; running 25GBASE-SR.
    let cap = 1 << 29 | 1 << 28 | 1 << 27 | 1 << 13 | 1 << 12 | 1 << 4 | 1 << 1;
    let m = media(cap, 1 << 29, true).unwrap();
    assert!(m.running);
    assert_eq!((m.to_string().as_str(), m.speed, m.fec), ("25GBASE-SR", "25g", Fec::Rs));
    let m = media(cap, 0, false).unwrap();
    assert!(!m.running);
    assert_eq!(m.to_string(), "25GBASE-SR/KR/CR");
    // Up at 10G: the link's media and none.
    let m = media(cap, 1 << 12, true).unwrap();
    assert_eq!((m.to_string().as_str(), m.fec), ("10GBASE-CR", Fec::None));
    // Up but PTYS oper empty: fall back to the capability.
    assert!(!media(cap, 0, true).unwrap().running);
    assert!(media(0, 0, false).is_none());
}

#[test]
fn report_line() {
    let cap = 1 << 29 | 1 << 28 | 1 << 27 | 1 << 13;
    let mut r = Report {
        version: "0.2.0",
        nic: "0000:84:00.0 15b3:1015 ConnectX-4 Lx",
        port: 1,
        mac: [0x24, 0x8a, 0x07, 0x0a, 0x0b, 0x0c],
        cap,
        oper: 1 << 29,
        up: true,
        fec_failed: None,
    };
    assert_eq!(
        r.to_string(),
        "stormnic-mlx5 0.2.0: 0000:84:00.0 15b3:1015 ConnectX-4 Lx port 1: MAC 24:8a:07:0a:0b:0c, \
         25GBASE-SR 25g, fec rs (standard for 25GBASE-SR), link up"
    );
    r.fec_failed = Some("PPLM is not in PRM Rev 0.40 (#8)");
    r.up = false;
    r.oper = 0;
    assert_eq!(
        r.to_string(),
        "stormnic-mlx5 0.2.0: 0000:84:00.0 15b3:1015 ConnectX-4 Lx port 1: MAC 24:8a:07:0a:0b:0c, \
         25GBASE-SR/KR/CR 25g capable, FEC FAILED: standard rs for 25GBASE-SR/KR/CR not set: \
         PPLM is not in PRM Rev 0.40 (#8), no link"
    );
    r.cap = 0;
    assert!(r.to_string().contains("media unknown (PTYS names no protocol), FEC FAILED: no media"));
}
