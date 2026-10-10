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

pub fn get64(b: &[u8], off: usize) -> u64 {
    u64::from_be_bytes(b[off..off + 8].try_into().unwrap())
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

/// The speed PTYS `eth_proto_oper` names (PRM Table 108).
pub fn speed_name(oper: u32) -> &'static str {
    const NAMES: &[(u32, &str)] = &[
        (31, "50G"),
        (30, "50G"),
        (29, "25G"),
        (28, "25G"),
        (27, "25G"),
        (22, "100G"),
        (21, "100G"),
        (20, "100G"),
        (18, "50G"),
        (16, "40G"),
        (15, "40G"),
        (14, "10G"),
        (13, "10G"),
        (12, "10G"),
        (7, "40G"),
        (6, "40G"),
        (4, "10G"),
        (3, "10G"),
        (2, "10G"),
        (1, "1G"),
        (0, "1G"),
    ];
    NAMES.iter().find(|(b, _)| oper & (1 << b) != 0).map_or("", |(_, n)| n)
}
