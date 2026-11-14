//! Device structure layouts and the arithmetic on them, with no UEFI in it,
//! so `test/layout.rs` runs it on the host. Every structure is a list of
//! big-endian dwords whose bits the PRM numbers 31..0 within the dword; a
//! field is named here by the byte offset of its dword and its bit range,
//! exactly as the PRM's "Field Descriptions" tables give them.

/// Write `v` into bits `hi..=lo` of the big-endian dword at byte `off`.
pub fn put(b: &mut [u8], off: usize, hi: u32, lo: u32, v: u32) {
    let width = hi - lo + 1;
    let mask = if width == 32 { u32::MAX } else { ((1u32 << width) - 1) << lo };
    let d = u32::from_be_bytes(b[off..off + 4].try_into().unwrap());
    let d = (d & !mask) | ((v << lo) & mask);
    b[off..off + 4].copy_from_slice(&d.to_be_bytes());
}

/// Bits `hi..=lo` of the big-endian dword at byte `off`.
pub fn get(b: &[u8], off: usize, hi: u32, lo: u32) -> u32 {
    let width = hi - lo + 1;
    let d = u32::from_be_bytes(b[off..off + 4].try_into().unwrap());
    if width == 32 {
        d
    } else {
        (d >> lo) & ((1u32 << width) - 1)
    }
}

/// A whole dword.
pub fn put32(b: &mut [u8], off: usize, v: u32) {
    b[off..off + 4].copy_from_slice(&v.to_be_bytes());
}

/// A 64-bit value split over two dwords, high first (PRM addresses).
pub fn put64(b: &mut [u8], off: usize, v: u64) {
    b[off..off + 8].copy_from_slice(&v.to_be_bytes());
}

/// A command input's header (PRM Table 99/100): opcode in 00h 31:16,
/// op_mod in 04h 15:0.
pub fn header(b: &mut [u8], opcode: u16, op_mod: u16) {
    put(b, 0x00, 31, 16, u32::from(opcode));
    put(b, 0x04, 15, 0, u32::from(op_mod));
}

/// Command queue entry (PRM Table 95/96), byte offsets.
pub mod entry {
    pub const SIZE: usize = 0x40;
    /// 00h 31:24.
    pub const TYPE: usize = 0x00;
    pub const TYPE_PCIE: u8 = 0x7;
    pub const IN_LEN: usize = 0x04;
    /// 08h–0Fh; bits 8:0 of the low dword are reserved (512-byte aligned).
    pub const IN_PTR: usize = 0x08;
    pub const IN_INLINE: usize = 0x10;
    pub const OUT_INLINE: usize = 0x20;
    pub const OUT_PTR: usize = 0x30;
    pub const OUT_LEN: usize = 0x38;
    /// 3Ch 31:24 token, 23:16 signature, 7:1 status, 0 ownership.
    pub const TOKEN: usize = 0x3c;
    pub const SIG: usize = 0x3d;
    pub const STATUS_OWN: usize = 0x3f;
}

/// Command interface mailbox block (PRM Table 97/98), byte offsets.
pub mod mbox {
    /// Data bytes per block (00h–1FFh).
    pub const DATA: usize = 0x200;
    /// The whole block, data and control.
    pub const SIZE: usize = 0x240;
    /// 230h–237h; bits 9:0 of the low dword reserved (1 KiB aligned).
    pub const NEXT: usize = 0x230;
    pub const BLOCK_NUM: usize = 0x238;
    /// 23Ch 23:16 token, 15:8 ctrl_signature, 7:0 signature.
    pub const TOKEN: usize = 0x23d;
    pub const CTRL_SIG: usize = 0x23e;
    pub const SIG: usize = 0x23f;
}

/// Signatures (PRM 7.14.1.1.1): a byte that makes the bytewise XOR of the
/// range, signature included, 0xFF. They are checked only when
/// `cmdif_checksum` is enabled (PRM 12.3.3.1); they are always computed.
pub mod sig {
    use super::{entry, mbox};

    fn xor(b: &[u8]) -> u8 {
        b.iter().fold(0, |a, &x| a ^ x)
    }

    /// The entry's signature, from its 64 bytes with the signature 0.
    pub fn entry(e: &[u8; entry::SIZE]) -> u8 {
        let mut e = *e;
        e[entry::SIG] = 0;
        !xor(&e)
    }

    /// A mailbox block's (ctrl_signature, signature), from the block with
    /// both 0. The control part is 200h–23Fh; its reserved bytes are 0, so
    /// the PRM's two descriptions of the range (7.14.1.1.1, Table 98) agree.
    pub fn mailbox(b: &[u8; mbox::SIZE]) -> (u8, u8) {
        let mut b = *b;
        b[mbox::CTRL_SIG] = 0;
        b[mbox::SIG] = 0;
        let ctrl = !xor(&b[mbox::DATA..]);
        b[mbox::CTRL_SIG] = ctrl;
        (ctrl, !xor(&b))
    }
}

/// Command opcodes (PRM Table 127).
pub mod op {
    pub const QUERY_HCA_CAP: u16 = 0x100;
    pub const INIT_HCA: u16 = 0x102;
    pub const TEARDOWN_HCA: u16 = 0x103;
    pub const ENABLE_HCA: u16 = 0x104;
    pub const DISABLE_HCA: u16 = 0x105;
    pub const QUERY_PAGES: u16 = 0x107;
    pub const MANAGE_PAGES: u16 = 0x108;
    pub const SET_HCA_CAP: u16 = 0x109;
    pub const QUERY_ISSI: u16 = 0x10a;
    pub const SET_ISSI: u16 = 0x10b;
    pub const CREATE_MKEY: u16 = 0x200;
    pub const DESTROY_MKEY: u16 = 0x202;
    pub const QUERY_SPECIAL_CONTEXTS: u16 = 0x203;
    pub const CREATE_EQ: u16 = 0x301;
    pub const DESTROY_EQ: u16 = 0x302;
    pub const CREATE_CQ: u16 = 0x400;
    pub const DESTROY_CQ: u16 = 0x401;
    pub const QUERY_VPORT_STATE: u16 = 0x750;
    pub const QUERY_NIC_VPORT_CONTEXT: u16 = 0x754;
    pub const MODIFY_NIC_VPORT_CONTEXT: u16 = 0x755;
    pub const ALLOC_PD: u16 = 0x800;
    pub const DEALLOC_PD: u16 = 0x801;
    pub const ALLOC_UAR: u16 = 0x802;
    pub const DEALLOC_UAR: u16 = 0x803;
    pub const ACCESS_REG: u16 = 0x805;
    pub const ALLOC_TRANSPORT_DOMAIN: u16 = 0x816;
    pub const DEALLOC_TRANSPORT_DOMAIN: u16 = 0x817;
    pub const CREATE_TIR: u16 = 0x900;
    pub const DESTROY_TIR: u16 = 0x902;
    pub const CREATE_SQ: u16 = 0x904;
    pub const MODIFY_SQ: u16 = 0x905;
    pub const DESTROY_SQ: u16 = 0x906;
    pub const CREATE_RQ: u16 = 0x908;
    pub const MODIFY_RQ: u16 = 0x909;
    pub const DESTROY_RQ: u16 = 0x90a;
    pub const CREATE_TIS: u16 = 0x912;
    pub const DESTROY_TIS: u16 = 0x914;
    pub const SET_FLOW_TABLE_ROOT: u16 = 0x92f;
    pub const CREATE_FLOW_TABLE: u16 = 0x930;
    pub const DESTROY_FLOW_TABLE: u16 = 0x931;
    pub const CREATE_FLOW_GROUP: u16 = 0x933;
    pub const DESTROY_FLOW_GROUP: u16 = 0x934;
    pub const SET_FLOW_TABLE_ENTRY: u16 = 0x936;
    pub const DELETE_FLOW_TABLE_ENTRY: u16 = 0x938;
}

/// The name of an opcode, for the log.
pub fn op_name(opcode: u16) -> &'static str {
    match opcode {
        op::QUERY_HCA_CAP => "QUERY_HCA_CAP",
        op::INIT_HCA => "INIT_HCA",
        op::TEARDOWN_HCA => "TEARDOWN_HCA",
        op::ENABLE_HCA => "ENABLE_HCA",
        op::DISABLE_HCA => "DISABLE_HCA",
        op::QUERY_PAGES => "QUERY_PAGES",
        op::MANAGE_PAGES => "MANAGE_PAGES",
        op::SET_HCA_CAP => "SET_HCA_CAP",
        op::QUERY_ISSI => "QUERY_ISSI",
        op::SET_ISSI => "SET_ISSI",
        op::CREATE_MKEY => "CREATE_MKEY",
        op::DESTROY_MKEY => "DESTROY_MKEY",
        op::QUERY_SPECIAL_CONTEXTS => "QUERY_SPECIAL_CONTEXTS",
        op::CREATE_EQ => "CREATE_EQ",
        op::DESTROY_EQ => "DESTROY_EQ",
        op::CREATE_CQ => "CREATE_CQ",
        op::DESTROY_CQ => "DESTROY_CQ",
        op::QUERY_VPORT_STATE => "QUERY_VPORT_STATE",
        op::QUERY_NIC_VPORT_CONTEXT => "QUERY_NIC_VPORT_CONTEXT",
        op::MODIFY_NIC_VPORT_CONTEXT => "MODIFY_NIC_VPORT_CONTEXT",
        op::ALLOC_PD => "ALLOC_PD",
        op::DEALLOC_PD => "DEALLOC_PD",
        op::ALLOC_UAR => "ALLOC_UAR",
        op::DEALLOC_UAR => "DEALLOC_UAR",
        op::ACCESS_REG => "ACCESS_REG",
        op::ALLOC_TRANSPORT_DOMAIN => "ALLOC_TRANSPORT_DOMAIN",
        op::DEALLOC_TRANSPORT_DOMAIN => "DEALLOC_TRANSPORT_DOMAIN",
        op::CREATE_TIR => "CREATE_TIR",
        op::DESTROY_TIR => "DESTROY_TIR",
        op::CREATE_SQ => "CREATE_SQ",
        op::MODIFY_SQ => "MODIFY_SQ",
        op::DESTROY_SQ => "DESTROY_SQ",
        op::CREATE_RQ => "CREATE_RQ",
        op::MODIFY_RQ => "MODIFY_RQ",
        op::DESTROY_RQ => "DESTROY_RQ",
        op::CREATE_TIS => "CREATE_TIS",
        op::DESTROY_TIS => "DESTROY_TIS",
        op::SET_FLOW_TABLE_ROOT => "SET_FLOW_TABLE_ROOT",
        op::CREATE_FLOW_TABLE => "CREATE_FLOW_TABLE",
        op::DESTROY_FLOW_TABLE => "DESTROY_FLOW_TABLE",
        op::CREATE_FLOW_GROUP => "CREATE_FLOW_GROUP",
        op::DESTROY_FLOW_GROUP => "DESTROY_FLOW_GROUP",
        op::SET_FLOW_TABLE_ENTRY => "SET_FLOW_TABLE_ENTRY",
        op::DELETE_FLOW_TABLE_ENTRY => "DELETE_FLOW_TABLE_ENTRY",
        _ => "?",
    }
}

/// Command delivery status (PRM Table 96, 3Ch 7:1).
pub fn delivery_name(s: u8) -> &'static str {
    match s {
        0x0 => "no error",
        0x1 => "signature error",
        0x2 => "token error",
        0x3 => "bad block number",
        0x4 => "bad output pointer",
        0x5 => "bad input pointer",
        0x6 => "internal error",
        0x7 => "input length error",
        0x8 => "output length error",
        0x9 => "reserved not zero",
        0x10 => "bad command type",
        _ => "?",
    }
}

/// Command return status (PRM Table 128).
pub fn status_name(s: u8) -> &'static str {
    match s {
        0x00 => "OK",
        0x01 => "INTERNAL_ERR",
        0x02 => "BAD_OP",
        0x03 => "BAD_PARAM",
        0x04 => "BAD_SYS_STATE",
        0x05 => "BAD_RESOURCE",
        0x06 => "RESOURCE_BUSY",
        0x08 => "EXCEED_LIM",
        0x09 => "BAD_RES_STATE",
        0x0a => "BAD_INDEX",
        0x0f => "NO_RESOURCES",
        0x10 => "BAD_RESOURCE_STATE",
        0x40 => "BAD_SIZE",
        0x50 => "BAD_INPUT_LEN",
        0x51 => "BAD_OUTPUT_LEN",
        _ => "?",
    }
}

/// Health syndrome (PRM Table 8, 1010h 31:24).
pub fn health_name(s: u8) -> &'static str {
    match s {
        0x0 => "none",
        0x1 => "FW_INTERNAL_ERR",
        0x7 => "DEAD_IRISC",
        0x8 => "HW_FATAL_ERR",
        0x9 => "FW_CRC_ERR",
        0xa => "ICM_FETCH_PCI_ERR",
        0xb => "ICM_PAGE_ERR",
        0xc => "ASYNCHRONOUS_EQ_BUF_OVERRUN",
        0xd => "EQ_IN_ERR",
        0xe => "EQ_INV",
        0xf => "FFSER_ERR",
        0x10 => "HIGH_TEMP_ERR",
        _ => "?",
    }
}

/// Send WQEs (docs/spec/datapath.md 4; PRM 7.4.4.1, Tables 25–28, 38).
pub mod wqe {
    use super::put;

    /// Send WQE basic block (PRM 7.4.1.1).
    pub const WQEBB: usize = 64;
    /// Largest WQE built: 2 WQEBBs.
    pub const MAX: usize = 2 * WQEBB;
    /// Bytes inlined in the Ethernet segment, at most: what fits in a
    /// 2-WQEBB WQE beside the control, Ethernet and pointer segments
    /// (2 + 5 octowords). Enough for the L2, IP and TCP/UDP headers of an
    /// ordinary frame, so every `min_wqe_inline_mode` up to TCP/UDP is met.
    pub const INLINE_MAX: usize = 2 + 5 * 16;
    pub const OPCODE_SEND: u32 = 0x0a;
    /// Ctrl 08h bits 3:2: CQE on every completion.
    const CE_ALWAYS: u32 = 0b10;

    /// Build the send WQE for `frame` at WQE index `index` of SQ `sqn` into
    /// `w`: the first `INLINE_MAX` bytes (or the whole frame) inline, the
    /// rest by pointer at `addr + inlined` with `lkey`. Returns the WQE's
    /// size in WQEBBs. `w` is zeroed first.
    pub fn send(w: &mut [u8; MAX], index: u16, sqn: u32, frame: &[u8], lkey: u32, addr: u64) -> usize {
        *w = [0; MAX];
        let n = frame.len().min(INLINE_MAX);
        // Ethernet segment at 10h: inline_header_size 0Ch 25:16, then the
        // inline bytes from segment byte 0Eh on.
        put(w, 0x1c, 25, 16, n as u32);
        w[0x1e..0x1e + n].copy_from_slice(&frame[..n]);
        let eth_octowords = 1 + n.saturating_sub(2).div_ceil(16);
        let mut ds = 1 + eth_octowords;
        let rest = frame.len() - n;
        if rest > 0 {
            let at = 16 * ds;
            put(w, at, 30, 0, rest as u32);
            put(w, at + 4, 31, 0, lkey);
            super::put64(w, at + 8, addr + n as u64);
            ds += 1;
        }
        // Control segment at 00h.
        put(w, 0x00, 23, 8, u32::from(index));
        put(w, 0x00, 7, 0, OPCODE_SEND);
        put(w, 0x04, 31, 8, sqn);
        put(w, 0x04, 5, 0, ds as u32);
        put(w, 0x08, 3, 2, CE_ALWAYS);
        ds.div_ceil(4)
    }
}

/// CQE and EQE fields (docs/spec/datapath.md 8, 9; PRM Tables 67, 77).
pub mod cqe {
    pub const SIZE: usize = 64;
    /// Byte 3Fh: opcode 7:4, cqe_format 3:2, se 1, owner 0.
    pub const OP_OWN: usize = 0x3f;
    /// The value every CQE starts with: opcode invalid (0xF), owner HW (1).
    pub const INIT: u8 = 0xf1;
    pub const OP_REQ: u8 = 0x0;
    pub const OP_RESP_SEND: u8 = 0x2;
    pub const OP_REQ_ERR: u8 = 0xd;
    pub const OP_RESP_ERR: u8 = 0xe;
    pub const OP_INVALID: u8 = 0xf;
    pub const BYTE_CNT: usize = 0x2c;
    /// 3Ch 31:16.
    pub const WQE_COUNTER: usize = 0x3c;
    /// Error CQE 34h: vendor syndrome 15:8, syndrome 7:0.
    pub const ERR_SYNDROME: usize = 0x37;
    pub const ERR_VENDOR: usize = 0x36;

    /// Is the entry the consumer counter `cc` points at software's?
    /// (PRM 7.12.3.1: owner equals the counter's wrap bit; an invalid
    /// opcode is never software's.)
    pub fn ready(op_own: u8, cc: u32, log_size: u32) -> bool {
        let owner = u32::from(op_own & 1);
        owner == (cc >> log_size) & 1 && op_own >> 4 != OP_INVALID
    }

    /// The same for an EQE, which has no opcode (PRM 7.13.2.1).
    pub fn eqe_ready(owner_byte: u8, cc: u32, log_size: u32) -> bool {
        u32::from(owner_byte & 1) == (cc >> log_size) & 1
    }
}

/// The IEEE 802.3 standard FEC for a media and speed (owner, 2026-11-14;
/// #8): no configuration, the driver picks it from what the port runs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fec {
    /// 1G, 10G and 40G: no FEC.
    None,
    /// RS(528,514) "Clause 91" FEC: 25G (802.3by), 50G NRZ and 100G NRZ
    /// (802.3bj/bm). PAM4 media would take RS(544,514); PRM Rev 0.40 names
    /// no PAM4 protocol, so no row here does.
    Rs,
}

impl Fec {
    pub fn name(self) -> &'static str {
        match self {
            Fec::None => "none",
            Fec::Rs => "rs",
        }
    }
}

/// One Ethernet protocol bit of PTYS `eth_proto_capability` / `_admin` /
/// `_oper` (PRM 11.1.2, Table 108).
#[derive(Debug, PartialEq, Eq)]
pub struct Proto {
    pub bit: u32,
    pub media: &'static str,
    pub speed: &'static str,
    pub mbps: u32,
    pub fec: Fec,
}

const fn p(bit: u32, media: &'static str, speed: &'static str, mbps: u32, fec: Fec) -> Proto {
    Proto { bit, media, speed, mbps, fec }
}

/// PRM Table 108, highest speed first, with each media's standard FEC.
pub const PROTOS: &[Proto] = &[
    p(22, "100GBASE-KR4", "100g", 100_000, Fec::Rs),
    p(21, "100GBASE-SR4", "100g", 100_000, Fec::Rs),
    p(20, "100GBASE-CR4", "100g", 100_000, Fec::Rs),
    p(31, "50GBASE-KR2", "50g", 50_000, Fec::Rs),
    p(30, "50GBASE-CR2", "50g", 50_000, Fec::Rs),
    p(18, "50GBASE-SR2", "50g", 50_000, Fec::Rs),
    p(16, "40GBASE-LR4/ER4", "40g", 40_000, Fec::None),
    p(15, "40GBASE-SR4", "40g", 40_000, Fec::None),
    p(7, "40GBASE-KR4", "40g", 40_000, Fec::None),
    p(6, "40GBASE-CR4", "40g", 40_000, Fec::None),
    p(29, "25GBASE-SR", "25g", 25_000, Fec::Rs),
    p(28, "25GBASE-KR", "25g", 25_000, Fec::Rs),
    p(27, "25GBASE-CR", "25g", 25_000, Fec::Rs),
    p(14, "10GBASE-ER/LR", "10g", 10_000, Fec::None),
    p(13, "10GBASE-SR", "10g", 10_000, Fec::None),
    p(12, "10GBASE-CR", "10g", 10_000, Fec::None),
    p(4, "10GBASE-KR", "10g", 10_000, Fec::None),
    p(3, "10GBASE-KX4", "10g", 10_000, Fec::None),
    p(2, "10GBASE-CX4", "10g", 10_000, Fec::None),
    p(1, "1000BASE-KX", "1g", 1_000, Fec::None),
    p(0, "SGMII", "1g", 1_000, Fec::None),
];

/// The protocols of a PTYS mask at its highest speed, fastest media first.
pub fn fastest(mask: u32) -> impl Iterator<Item = &'static Proto> {
    let top = PROTOS.iter().find(|p| mask & (1 << p.bit) != 0).map_or(0, |p| p.mbps);
    PROTOS.iter().filter(move |p| top != 0 && p.mbps == top && mask & (1 << p.bit) != 0)
}

/// The speed PTYS `eth_proto_oper` names (PRM Table 108), e.g. `25g`.
pub fn speed_name(oper: u32) -> &'static str {
    fastest(oper).next().map_or("", |p| p.speed)
}

/// What the port runs, or (with no link) the fastest it can: its media, the
/// speed and that media's standard FEC. With several media at the top speed
/// (a capability mask), they are named together, e.g. `25GBASE-SR/KR/CR`;
/// they share the speed's standard FEC.
pub struct Media {
    pub media: [&'static str; 6],
    pub count: usize,
    pub speed: &'static str,
    pub fec: Fec,
    /// From `eth_proto_oper` (the link), not the capability.
    pub running: bool,
}

pub fn media(cap: u32, oper: u32, up: bool) -> Option<Media> {
    let (mask, running) = if up && !speed_name(oper).is_empty() { (oper, true) } else { (cap, false) };
    let mut m = Media { media: [""; 6], count: 0, speed: "", fec: Fec::None, running };
    for p in fastest(mask) {
        if m.count == 0 {
            m.speed = p.speed;
            m.fec = p.fec;
        }
        if m.count < m.media.len() {
            m.media[m.count] = p.media;
            m.count += 1;
        }
    }
    (m.count != 0).then_some(m)
}

impl core::fmt::Display for Media {
    /// `25GBASE-SR`, or `25GBASE-SR/KR/CR`: the first name whole, the
    /// others by their suffix after `BASE-`.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for (i, n) in self.media[..self.count].iter().enumerate() {
            if i == 0 {
                f.write_str(n)?;
            } else {
                f.write_str("/")?;
                f.write_str(n.split_once("BASE-").map_or(n, |(_, s)| s))?;
            }
        }
        Ok(())
    }
}

/// PPLM, Port Phy Link Mode (register 0x5023): the FEC override per speed.
/// Mellanox's PRM register database as published in mstflint,
/// `tools_layouts/adb/prm/hca/ext/register_access_table.adb`, node
/// `pplm_reg_ext` (source_doc `NIC_PRM_internal` 0.0.65j; docs/spec/fec.md).
/// An adb offset `0xB.b` is byte B, bit b from the LSB of that big-endian
/// dword, checked against PRM Rev 0.40 Tables 108/110 (docs/spec/fec.md 1).
pub mod pplm {
    use super::{get, put};

    pub const ID: u32 = 0x5023;
    /// `pplm_reg_ext` size 0x80.
    pub const LEN: usize = 0x80;
    /// `fec_mode_active`, 0xC.0, 24 bits.
    pub const ACTIVE: (usize, u32, u32) = (0x0c, 23, 0);

    /// One speed's `fec_override_cap_*` / `fec_override_admin_*` pair: the
    /// cap is the bitmask the firmware offers, the admin a one-hot choice
    /// (0 = auto, the firmware's own).
    pub struct Speed {
        pub name: &'static str,
        pub cap: (usize, u32, u32),
        pub admin: (usize, u32, u32),
        /// The standard's one-hot value and its name.
        pub want: u32,
        pub fec: &'static str,
    }

    const NO_FEC: u32 = 0x1;
    const RS: u32 = 0x4;
    /// RS(544,514) in the 16-bit PAM4 fields.
    const RS_544: u32 = 0x80;

    /// Every speed whose standard FEC the driver sets: 10G/40G none,
    /// 25G/50G/100G (NRZ) RS(528), PAM4 lanes RS(544). The 25G and 50G
    /// admin fields must hold the same value (adb) - both RS here.
    pub const SPEEDS: &[Speed] = &[
        Speed { name: "10g/40g", cap: (0x10, 3, 0), admin: (0x14, 3, 0), want: NO_FEC, fec: "none" },
        Speed { name: "25g", cap: (0x10, 7, 4), admin: (0x14, 7, 4), want: RS, fec: "rs" },
        Speed { name: "50g", cap: (0x10, 11, 8), admin: (0x14, 11, 8), want: RS, fec: "rs" },
        Speed { name: "100g", cap: (0x10, 15, 12), admin: (0x14, 15, 12), want: RS, fec: "rs" },
        Speed { name: "50g-pam4", cap: (0x1c, 15, 0), admin: (0x24, 15, 0), want: RS_544, fec: "rs-544" },
        Speed { name: "100g-pam4x2", cap: (0x1c, 31, 16), admin: (0x24, 31, 16), want: RS_544, fec: "rs-544" },
        Speed { name: "200g-pam4x4", cap: (0x18, 15, 0), admin: (0x20, 15, 0), want: RS_544, fec: "rs-544" },
        Speed { name: "400g-pam4x8", cap: (0x18, 31, 16), admin: (0x20, 31, 16), want: RS_544, fec: "rs-544" },
    ];

    /// The PPLM speed field for a PTYS speed (`layout::PROTOS`).
    pub fn for_speed(speed: &str) -> Option<&'static Speed> {
        let name = match speed {
            "10g" | "40g" => "10g/40g",
            s => s,
        };
        SPEEDS.iter().find(|s| s.name == name)
    }

    fn field(b: &[u8], f: (usize, u32, u32)) -> u32 {
        get(b, f.0, f.1, f.2)
    }

    /// Does the firmware offer the standard for this speed?
    pub fn offered(reg: &[u8], s: &Speed) -> bool {
        field(reg, s.cap) & s.want != 0
    }

    pub fn admin(reg: &[u8], s: &Speed) -> u32 {
        field(reg, s.admin)
    }

    /// Set every offered speed's admin to its standard in a PPLM read
    /// (the rest of it kept, a read-modify-write). The admin of a speed
    /// that does not offer its standard is left as it is. True if anything
    /// changed.
    pub fn apply(reg: &mut [u8]) -> bool {
        let mut changed = false;
        for s in SPEEDS {
            if offered(reg, s) && admin(reg, s) != s.want {
                put(reg, s.admin.0, s.admin.1, s.admin.2, s.want);
                changed = true;
            }
        }
        changed
    }

    /// `fec_mode_active` (adb enum), as the report names it.
    pub fn active_name(v: u32) -> &'static str {
        match v {
            0x0 => "none",
            0x1 => "fc",
            0x2 => "rs",
            0x3 => "rs-ll",
            0x7 => "rs-544",
            0x9 => "rs-ll-50g",
            _ => "other",
        }
    }

    /// The `fec_mode_active` value that is a speed's standard.
    pub fn active_is(v: u32, fec: &str) -> bool {
        active_name(v) == fec
    }
}

/// PDDR, Port Diagnostics Database (register 0x5031), page 3 "Module Info":
/// what is plugged into the port. mstflint's adb, nodes `pddr_reg_ext`
/// (`page_select` 0x4.0, 8 bits; `page_data` from 0x8) and
/// `pddr_module_info_ext` (offsets below are within the page).
pub mod pddr {
    use super::get;

    pub const ID: u32 = 0x5031;
    pub const LEN: usize = 0x100;
    pub const PAGE_SELECT: (usize, u32, u32) = (0x04, 7, 0);
    pub const MODULE_INFO_PAGE: u32 = 3;
    const PAGE: usize = 0x08;

    #[derive(Debug, PartialEq, Eq)]
    pub struct Module {
        /// `cable_identifier`, 0x4.8.
        pub identifier: u32,
        /// `cable_type`, 0x4.28.
        pub cable_type: u32,
        /// `cable_length`, 0x4.16, metres (non-CMIS).
        pub length: u32,
        /// `vendor_name` 0x14, `vendor_pn` 0x24: 16 ASCII bytes each.
        pub vendor: [u8; 16],
        pub pn: [u8; 16],
    }

    pub fn module(reg: &[u8]) -> Option<Module> {
        if reg.len() < PAGE + 0x34 {
            return None;
        }
        let p = &reg[PAGE..];
        let mut m = Module {
            identifier: get(p, 0x04, 15, 8),
            cable_type: get(p, 0x04, 31, 28),
            length: get(p, 0x04, 23, 16),
            vendor: [0; 16],
            pn: [0; 16],
        };
        m.vendor.copy_from_slice(&p[0x14..0x24]);
        m.pn.copy_from_slice(&p[0x24..0x34]);
        Some(m)
    }

    fn identifier(v: u32) -> &'static str {
        match v {
            0 => "QSFP28",
            1 => "QSFP+",
            2 => "SFP28/SFP+",
            3 => "QSA",
            4 => "backplane",
            5 => "SFP-DD",
            6 => "QSFP-DD",
            7 => "QSFP-CMIS",
            8 => "OSFP",
            _ => "module",
        }
    }

    fn cable_type(v: u32) -> &'static str {
        match v {
            1 => "active cable",
            2 => "optical",
            3 => "passive copper",
            5 => "twisted pair",
            _ => "",
        }
    }

    fn ascii(b: &[u8]) -> &str {
        let n = b.iter().position(|&c| !(0x20..0x7f).contains(&c)).unwrap_or(b.len());
        core::str::from_utf8(&b[..n]).unwrap_or("").trim_end()
    }

    impl core::fmt::Display for Module {
        /// `SFP28/SFP+ optical FS SFP-25GSR-85`, `QSFP28 passive copper 3m …`
        /// or `no module`.
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            if self.cable_type == 4 {
                return f.write_str("no module");
            }
            f.write_str(identifier(self.identifier))?;
            let t = cable_type(self.cable_type);
            if !t.is_empty() {
                write!(f, " {t}")?;
            }
            if self.cable_type != 2 && self.length != 0 {
                write!(f, " {}m", self.length)?;
            }
            for s in [ascii(&self.vendor), ascii(&self.pn)] {
                if !s.is_empty() {
                    write!(f, " {s}")?;
                }
            }
            Ok(())
        }
    }
}

/// How setting the standard FEC went (#8), for the report line.
pub enum FecResult<'a> {
    /// The standard is the port's admin (set now or already); with the
    /// `fec_mode_active` read when the link is up.
    Standard { active: Option<u32> },
    /// No FEC field for the speed (1G): nothing to set, the standard is none.
    NoField,
    Failed(&'a str),
}

/// What a bound port reports: the one line per port (#8).
pub struct Report<'a> {
    pub version: &'a str,
    /// The PCI location, ID and name, e.g. `0000:84:00.0 15b3:1015 ConnectX-4 Lx`.
    pub nic: &'a str,
    pub port: u32,
    pub mac: [u8; 6],
    pub cap: u32,
    pub oper: u32,
    pub up: bool,
    /// What PDDR says is plugged in, if it answered.
    pub module: Option<&'a pddr::Module>,
    pub fec: FecResult<'a>,
}

impl core::fmt::Display for Report<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let m = self.mac;
        write!(
            f,
            "stormnic-mlx5 {}: {} port {}: MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}, ",
            self.version, self.nic, self.port, m[0], m[1], m[2], m[3], m[4], m[5]
        )?;
        match media(self.cap, self.oper, self.up) {
            Some(md) => {
                write!(f, "{} {}{}, ", md, md.speed, if md.running { "" } else { " capable" })?;
                if let Some(m) = self.module {
                    write!(f, "{m}, ")?;
                }
                let std = md.fec.name();
                match self.fec {
                    FecResult::Standard { active: Some(a) } if !pplm::active_is(a, std) => write!(
                        f,
                        "FEC FAILED: standard {std} for {md} set, but the link runs {}, ",
                        pplm::active_name(a)
                    )?,
                    FecResult::Standard { active } => {
                        write!(f, "fec {std} (standard for {md}")?;
                        if active.is_some() {
                            write!(f, ", running {std}")?;
                        }
                        f.write_str("), ")?
                    }
                    FecResult::NoField => write!(f, "fec {std} (standard for {md}), ")?,
                    FecResult::Failed(why) => write!(f, "FEC FAILED: standard {std} for {md} not set: {why}, ")?,
                }
            }
            None => write!(f, "media unknown (PTYS names no protocol), FEC FAILED: no media, so no standard, ")?,
        }
        f.write_str(if self.up { "link up" } else { "no link" })
    }
}
