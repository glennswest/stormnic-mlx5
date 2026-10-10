//! The Ethernet data path (docs/spec/datapath.md, docs/spec/steering.md):
//! one EQ for page requests and port changes, a send CQ and SQ behind a
//! TIS, a receive CQ and RQ behind a direct TIR, and a NIC RX flow table
//! whose one match-all entry sends every frame the vport receives to that
//! TIR. Everything is polled; nothing is armed and no interrupt is used.
//!
//! The SNP's receive filter runs in software (`snp.rs`), so the vport is
//! asked for its own MAC, all multicast and, until a first hardware run
//! shows unicast arrives without it, everything (docs/spec/steering.md 5.6).

use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::sync::atomic::{fence, Ordering};

use core::time::Duration;

use uefi::boot;

use crate::cmd::BAR;
use crate::dma::{DmaBuf, Mem, PAGE};
use crate::hca::{self, input, Failed, Hca};
use crate::layout::{self, cqe, get, op, put, put32, put64, wqe};
use crate::pci::PciIo;

/// The largest frame handed to or taken from the SNP: 1500 + 14 + a VLAN tag.
pub const MAX_FRAME: usize = 1518;
/// Receive buffer per WQE: a frame, a second VLAN tag and an FCS if the
/// device scatters one (docs/spec/datapath.md HW-CHECK).
const RX_BUF: usize = 2048;

/// Queue sizes (log2). The EQ carries only page requests and port events.
const LOG_EQ: u32 = 6;
const LOG_SQ: u32 = 6; // WQEBBs
const LOG_TX_CQ: u32 = 6;
const LOG_RQ: u32 = 8;
const LOG_RX_CQ: u32 = 8;
/// A cyclic RQ WQE: one 16-byte scatter entry (docs/spec/datapath.md 5).
const LOG_RQ_STRIDE: u32 = 4;
/// TX copy buffers, one per WQEBB slot (a WQE starting there uses it).
const TX_SLOTS: usize = 1 << LOG_SQ;

/// Port registers (PRM 11.1, Table 104) and the local port of a PF.
const REG_PTYS: u32 = 0x5004;
const REG_PAOS: u32 = 0x5006;
const LOCAL_PORT: u32 = 1;

/// Event types (docs/spec/datapath.md 9.2).
const EV_CQ_ERROR: u32 = 0x04;
const EV_PORT_CHANGE: u32 = 0x09;
const EV_PAGE_REQUEST: u32 = 0x0b;

/// Doorbell record page layout: one 64-byte line each.
const DBR_SQ: usize = 0x00;
const DBR_RQ: usize = 0x40;
const DBR_TX_CQ: usize = 0x80;
const DBR_RX_CQ: usize = 0xc0;

/// UAR page offsets (docs/spec/datapath.md 1.2).
const UAR_EQ_UPDATE: u64 = 0x48;
const UAR_BF: [u64; 2] = [0x800, 0x900];

/// A MAC address, printed the usual way.
pub struct Mac(pub [u8; 6]);

impl core::fmt::Display for Mac {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let m = self.0;
        write!(f, "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}", m[0], m[1], m[2], m[3], m[4], m[5])
    }
}

/// Why `send` did not queue a frame.
pub enum Send {
    Full,
    TooLong,
    Broken,
}

/// A completion or event queue in DMA memory, polled.
struct Ring {
    buf: DmaBuf,
    num: u32,
    log: u32,
    cc: u32,
}

impl Ring {
    fn entry(&self, i: u32) -> Mem {
        self.buf.slice(((i & ((1 << self.log) - 1)) as usize) * cqe::SIZE, cqe::SIZE)
    }
}

/// A send WQE in flight.
struct Pending {
    index: u16,
    bbs: u16,
    token: usize,
}

pub struct Eth {
    mac: [u8; 6],
    link: bool,
    speed: &'static str,
    dbr: DmaBuf,
    eq: Ring,
    tx_cq: Ring,
    rx_cq: Ring,
    sq_buf: DmaBuf,
    tx_bufs: DmaBuf,
    rq_buf: DmaBuf,
    rx_bufs: DmaBuf,
    tis: u32,
    sqn: u32,
    rqn: u32,
    tir: u32,
    ft: u32,
    fg: u32,
    /// sq_wqebb_counter (16-bit, PRM 7.4.1.1).
    sq_pc: u16,
    bf: usize,
    inflight: VecDeque<Pending>,
    /// Tokens of frames the device has sent, for GetStatus.
    sent: VecDeque<usize>,
    /// RQ WQEs posted (rq_wqe_counter, 16-bit).
    rq_pc: u16,
    /// Received frames not yet taken: (slot, length).
    rx_ready: VecDeque<(u16, usize)>,
    /// The SQ went to error; nothing more is sent.
    broken: bool,
    errors_logged: u32,
    uar_off: u64,
    lkey: u32,
    made: Objects,
}

/// Which device objects exist, for `close`.
#[derive(Default)]
struct Objects {
    eq: bool,
    tx_cq: bool,
    rx_cq: bool,
    tis: bool,
    sq: bool,
    rq: bool,
    tir: bool,
    ft: bool,
    fg: bool,
    fte: bool,
    l2_entry: bool,
}

fn pas(i: &mut [u8], at: usize, b: &DmaBuf) {
    for p in 0..b.pages {
        put64(i, at + 8 * p, b.dev + (p * PAGE) as u64);
    }
}

fn dma(pci: &mut PciIo, bytes: usize, what: &str) -> Result<DmaBuf, Failed> {
    DmaBuf::new(pci, bytes).map_err(|e| {
        alarm!("stormnic-mlx5: {bytes} bytes for the {what}: {:?}", e.status());
        Failed
    })
}

/// A 16-byte command with one argument dword at 08h, output's 08h 23:0.
fn create_id(h: &mut Hca, pci: &mut PciIo, i: &[u8]) -> Result<u32, Failed> {
    let mut o = [0u8; 16];
    h.exec(pci, i, &mut o)?;
    Ok(get(&o, 0x08, 23, 0))
}

fn destroy(h: &mut Hca, pci: &mut PciIo, opcode: u16, id: u32) -> bool {
    h.simple(pci, opcode, 0, &[(0x08, id)]).is_ok()
}

/// ACCESS_REG read of a port register (docs/spec/cmd.md 6.20); its data.
fn read_reg(h: &mut Hca, pci: &mut PciIo, id: u32, len: usize) -> Result<Vec<u8>, Failed> {
    let mut i = input(op::ACCESS_REG, 1, 0x10 + len);
    put(&mut i, 0x08, 15, 0, id);
    // local_port is the index field of every port register (HW-CHECK 15).
    put(&mut i, 0x10, 23, 16, LOCAL_PORT);
    if id == REG_PTYS {
        put(&mut i, 0x10, 2, 0, 0b100);
    }
    let mut o = alloc::vec![0u8; 0x10 + len];
    h.exec(pci, &i, &mut o)?;
    o.drain(..0x10);
    Ok(o)
}

fn write_reg(h: &mut Hca, pci: &mut PciIo, id: u32, data: &[u8]) -> Result<(), Failed> {
    let mut i = input(op::ACCESS_REG, 0, 0x10 + data.len());
    put(&mut i, 0x08, 15, 0, id);
    i[0x10..].copy_from_slice(data);
    let mut o = alloc::vec![0u8; 0x10 + data.len()];
    h.exec(pci, &i, &mut o)
}

impl Eth {
    pub fn num(&self) -> u32 {
        LOCAL_PORT
    }
    pub fn mac(&self) -> [u8; 6] {
        self.mac
    }
    pub fn link_up(&self) -> bool {
        self.link
    }
    pub fn speed(&self) -> &'static str {
        self.speed
    }

    /// PAOS oper_status and PTYS eth_proto_oper: is the link up, how fast.
    pub fn refresh_link(&mut self, h: &mut Hca, pci: &mut PciIo) {
        let Ok(paos) = read_reg(h, pci, REG_PAOS, 16) else {
            return;
        };
        let up = get(&paos, 0, 3, 0) == 1;
        let speed = if up {
            read_reg(h, pci, REG_PTYS, 64).map_or("", |p| layout::speed_name(get(&p, 0x24, 31, 0)))
        } else {
            ""
        };
        if up != self.link {
            note!(
                self.link,
                "stormnic-mlx5: port {} link {}{}{}",
                LOCAL_PORT,
                if up { "up" } else { "down" },
                if speed.is_empty() { "" } else { " " },
                speed
            );
        }
        self.link = up;
        self.speed = speed;
    }

    /// Wait up to `ms` for link, polling the events and PAOS.
    pub fn wait_link(&mut self, h: &mut Hca, pci: &mut PciIo, ms: u32) {
        let mut waited = 0;
        loop {
            self.poll(h, pci);
            self.refresh_link(h, pci);
            if self.link || waited >= ms {
                break;
            }
            boot::stall(Duration::from_millis(100));
            waited += 100;
        }
        trace!(
            "  link {} after {waited} ms{}",
            if self.link { "up" } else { "still down" },
            if self.speed.is_empty() { alloc::string::String::new() } else { alloc::format!(" ({})", self.speed) }
        );
    }

    /// Drain the EQ, the send CQ and the receive CQ.
    pub fn poll(&mut self, h: &mut Hca, pci: &mut PciIo) {
        self.poll_eq(h, pci);
        self.poll_tx();
        self.poll_rx();
    }

    fn poll_eq(&mut self, h: &mut Hca, pci: &mut PciIo) {
        let mut any = false;
        let mut pages = false;
        let mut port = false;
        loop {
            let e = self.eq.entry(self.eq.cc);
            if !cqe::eqe_ready(e.u8(0x3f), self.eq.cc, self.eq.log) {
                break;
            }
            fence(Ordering::SeqCst);
            let kind = u32::from(e.u8(0x01));
            match kind {
                EV_PAGE_REQUEST => pages = true,
                EV_PORT_CHANGE => port = true,
                EV_CQ_ERROR => say!(
                    "stormnic-mlx5: CQ error event: CQ {:#x} syndrome {:#x}",
                    e.be32(0x20) & 0xff_ffff,
                    e.u8(0x2b)
                ),
                _ => trace!("stormnic-mlx5: event type {kind:#x} sub-type {:#x}", e.u8(0x03)),
            }
            self.eq.cc = self.eq.cc.wrapping_add(1);
            any = true;
        }
        if any {
            let v = (self.eq.num << 24) | (self.eq.cc & 0xff_ffff);
            let _ = pci.mem_write32(BAR, h.uar_off + UAR_EQ_UPDATE, v.to_be());
        }
        if pages {
            hca::page_request(h, pci);
        }
        if port {
            self.refresh_link(h, pci);
        }
    }

    fn cq_next(cq: &mut Ring) -> Option<Mem> {
        let e = cq.entry(cq.cc);
        if !cqe::ready(e.u8(cqe::OP_OWN), cq.cc, cq.log) {
            return None;
        }
        fence(Ordering::SeqCst);
        cq.cc = cq.cc.wrapping_add(1);
        Some(e)
    }

    fn cq_update(&self, dbr: usize, cq: &Ring) {
        fence(Ordering::SeqCst);
        self.dbr.set_be32(dbr, cq.cc & 0xff_ffff);
    }

    fn log_error(&mut self, what: &str, e: &Mem) {
        if self.errors_logged < 8 {
            self.errors_logged += 1;
            say!(
                "stormnic-mlx5: {what} completion error: syndrome {:#x} vendor {:#x}, WQE counter {}",
                e.u8(cqe::ERR_SYNDROME),
                e.u8(cqe::ERR_VENDOR),
                e.be16(cqe::WQE_COUNTER)
            );
        }
    }

    fn poll_tx(&mut self) {
        let mut any = false;
        while let Some(e) = Self::cq_next(&mut self.tx_cq) {
            any = true;
            let opcode = e.u8(cqe::OP_OWN) >> 4;
            let counter = e.be16(cqe::WQE_COUNTER);
            if opcode == cqe::OP_REQ_ERR {
                self.log_error("send", &e);
                self.broken = true;
            } else if opcode != cqe::OP_REQ {
                trace!("stormnic-mlx5: send CQE opcode {opcode:#x}");
            }
            // Completions come in order; retire everything up to this WQE.
            while let Some(p) = self.inflight.pop_front() {
                self.sent.push_back(p.token);
                if p.index == counter {
                    break;
                }
            }
        }
        if any {
            let cq = &self.tx_cq;
            self.cq_update(DBR_TX_CQ, cq);
        }
    }

    fn poll_rx(&mut self) {
        let mut any = false;
        while let Some(e) = Self::cq_next(&mut self.rx_cq) {
            any = true;
            let opcode = e.u8(cqe::OP_OWN) >> 4;
            let slot = e.be16(cqe::WQE_COUNTER) & ((1 << LOG_RQ) - 1);
            if opcode == cqe::OP_RESP_SEND {
                let len = e.be32(cqe::BYTE_CNT) as usize;
                self.rx_ready.push_back((slot, len.min(RX_BUF)));
            } else {
                if opcode == cqe::OP_RESP_ERR {
                    self.log_error("receive", &e);
                }
                // Give the buffer straight back.
                self.rx_ready.push_back((slot, 0));
            }
        }
        if any {
            let cq = &self.rx_cq;
            self.cq_update(DBR_RX_CQ, cq);
        }
    }

    fn rx_slot(&self, slot: u16) -> Mem {
        self.rx_bufs.slice(usize::from(slot) * RX_BUF, RX_BUF)
    }

    /// Post the receive WQE at the RQ's producer position (one scatter
    /// entry, docs/spec/datapath.md 5) and hand it to the device.
    fn post_rx(&mut self, lkey: u32) {
        let slot = self.rq_pc & ((1 << LOG_RQ) - 1);
        let w = self.rq_buf.slice(usize::from(slot) * 16, 16);
        let b = self.rx_slot(slot);
        w.set_be32(0, RX_BUF as u32);
        w.set_be32(4, lkey);
        w.set_be64(8, b.dev);
        self.rq_pc = self.rq_pc.wrapping_add(1);
        fence(Ordering::SeqCst);
        self.dbr.set_be32(DBR_RQ, u32::from(self.rq_pc));
    }

    /// Has a frame arrived (polling first)?
    pub fn rx_ready(&mut self) -> bool {
        self.drop_empty();
        !self.rx_ready.is_empty()
    }

    /// Recycle completions that carry no frame.
    fn drop_empty(&mut self) {
        while self.rx_ready.front().is_some_and(|&(_, len)| len == 0) {
            self.pop_rx();
        }
    }

    /// The oldest received frame, left in place.
    pub fn peek_rx(&mut self) -> Option<(Mem, usize)> {
        self.drop_empty();
        let &(slot, len) = self.rx_ready.front()?;
        Some((self.rx_slot(slot), len))
    }

    /// Done with the oldest received frame: its buffer goes back to the RQ.
    /// Completions arrive in posting order, so the slot freed is the one
    /// the producer position names next.
    pub fn pop_rx(&mut self) {
        if self.rx_ready.pop_front().is_some() {
            let lkey = self.lkey;
            self.post_rx(lkey);
        }
    }

    pub fn sent_pending(&self) -> bool {
        !self.sent.is_empty()
    }

    pub fn take_sent(&mut self) -> Option<usize> {
        self.sent.pop_front()
    }

    /// Queue one frame; `token` comes back from `take_sent` once sent.
    pub fn send(&mut self, pci: &mut PciIo, frame: &[u8], token: usize) -> Result<(), Send> {
        if self.broken {
            return Err(Send::Broken);
        }
        if frame.len() > MAX_FRAME || frame.len() < 14 {
            return Err(Send::TooLong);
        }
        self.poll_tx();
        let used: usize = self.inflight.iter().map(|p| usize::from(p.bbs)).sum();
        if used + wqe::MAX / wqe::WQEBB > (1 << LOG_SQ) {
            return Err(Send::Full);
        }
        let index = self.sq_pc;
        let slot = usize::from(index) % TX_SLOTS;
        let b = self.tx_bufs.slice(slot * RX_BUF, RX_BUF);
        b.write(0, frame);
        let mut w = [0u8; wqe::MAX];
        let bbs = wqe::send(&mut w, index, self.sqn, frame, self.lkey, b.dev);
        for bb in 0..bbs {
            let at = (usize::from(index) + bb) % (1 << LOG_SQ);
            self.sq_buf.write(at * wqe::WQEBB, &w[bb * wqe::WQEBB..(bb + 1) * wqe::WQEBB]);
        }
        self.sq_pc = index.wrapping_add(bbs as u16);
        self.inflight.push_back(Pending { index, bbs: bbs as u16, token });
        // PRM 7.4.3.1: WQE, then the doorbell record, then the doorbell.
        // The SQ's counter goes in both halves of its record (HW-CHECK).
        fence(Ordering::SeqCst);
        self.dbr.set_be32(DBR_SQ, u32::from(self.sq_pc));
        self.dbr.set_be32(DBR_SQ + 4, u32::from(self.sq_pc));
        fence(Ordering::SeqCst);
        let db = u64::from_le_bytes(w[..8].try_into().unwrap());
        let _ = pci.mem_write64(BAR, self.uar_off + UAR_BF[self.bf], db);
        self.bf ^= 1;
        Ok(())
    }
}

/// Build the data path. A failure part way destroys what exists.
pub fn open(h: &mut Hca, pci: &mut PciIo) -> Result<Eth, Failed> {
    if !h.caps.nic_flow_table || !h.caps.ft_support {
        alarm!("stormnic-mlx5: no NIC receive flow table (nic_flow_table {}, ft_support {})", h.caps.nic_flow_table, h.caps.ft_support);
        return Err(Failed);
    }
    let mut e = Eth {
        mac: [0; 6],
        link: false,
        speed: "",
        dbr: dma(pci, PAGE, "doorbell records")?,
        eq: Ring { buf: dma(pci, 64 << LOG_EQ, "EQ")?, num: 0, log: LOG_EQ, cc: 0 },
        tx_cq: Ring { buf: dma(pci, 64 << LOG_TX_CQ, "send CQ")?, num: 0, log: LOG_TX_CQ, cc: 0 },
        rx_cq: Ring { buf: dma(pci, 64 << LOG_RX_CQ, "receive CQ")?, num: 0, log: LOG_RX_CQ, cc: 0 },
        sq_buf: dma(pci, wqe::WQEBB << LOG_SQ, "SQ")?,
        tx_bufs: dma(pci, TX_SLOTS * RX_BUF, "send buffers")?,
        rq_buf: dma(pci, 16 << LOG_RQ, "RQ")?,
        rx_bufs: dma(pci, RX_BUF << LOG_RQ, "receive buffers")?,
        tis: 0,
        sqn: 0,
        rqn: 0,
        tir: 0,
        ft: 0,
        fg: 0,
        sq_pc: 0,
        bf: 0,
        inflight: VecDeque::new(),
        sent: VecDeque::new(),
        rq_pc: 0,
        rx_ready: VecDeque::new(),
        broken: false,
        errors_logged: 0,
        uar_off: h.uar_off,
        lkey: h.lkey,
        made: Objects::default(),
    };
    match bring_up(&mut e, h, pci) {
        Ok(()) => Ok(e),
        Err(Failed) => {
            close(e, h, pci);
            Err(Failed)
        }
    }
}

/// NIC RX flow table type and the TIR destination type (steering.md 2).
const FT_NIC_RX: u32 = 0x0;
const DEST_TIR: u32 = 2;
const ACTION_FWD_DEST: u32 = 0x4;

fn bring_up(e: &mut Eth, h: &mut Hca, pci: &mut PciIo) -> Result<(), Failed> {
    if e.uar_off < 0x4100 {
        say!("stormnic-mlx5: UAR {} at BAR offset {:#x} overlaps the init segment (HW-CHECK)", h.uar, e.uar_off);
    }

    // The vport (steering.md 5): admin up, the permanent MAC, the minimum
    // inline mode.
    let o = h.simple(pci, op::QUERY_VPORT_STATE, 0, &[])?;
    let (admin, state) = (get(&o, 0x0c, 7, 4), get(&o, 0x0c, 3, 0));
    trace!("  vport: admin state {admin}, state {state}");
    if admin != 1 {
        h.simple(pci, 0x751, 0, &[(0x0c, 1 << 4)])?;
    }
    let i = input(op::QUERY_NIC_VPORT_CONTEXT, 0, 16);
    let mut o = alloc::vec![0u8; 0x10 + 0x100 + 8 * 16];
    h.exec(pci, &i, &mut o)?;
    e.mac.copy_from_slice(&o[0x106..0x10c]);
    let vport_inline = get(&o, 0x10, 26, 24);
    let vport_mtu = get(&o, 0x34, 15, 0);
    trace!("  vport: permanent MAC {}, MTU {vport_mtu}, min_wqe_inline_mode {vport_inline}", Mac(e.mac));
    if e.mac == [0; 6] {
        alarm!("stormnic-mlx5: the vport has no permanent MAC address");
        return Err(Failed);
    }
    // The SQ's minimum inline mode (datapath.md 4.5): HCA_CAP says whether
    // it is L2 (0), the vport's (1) or not required (2).
    let min_inline = match h.caps.wqe_inline_mode {
        0 => 1,
        1 => vport_inline,
        _ => 0,
    };

    // Own MAC in the unicast list; promiscuous multicast and all (the
    // SNP filters in software; steering.md 5.5, HW-CHECK on delivery).
    let mut i = input(op::MODIFY_NIC_VPORT_CONTEXT, 0, 0x208);
    put32(&mut i, 0x0c, (1 << 2) | (1 << 4));
    put(&mut i, 0x1f0, 30, 29, 0b11);
    put(&mut i, 0x1f0, 26, 24, 0);
    put(&mut i, 0x1f0, 11, 0, 1);
    // MAC layout (steering.md 5.3): 00h[15:0] = mac[47:32], 04h = mac[31:0].
    i[0x202..0x208].copy_from_slice(&e.mac);
    let mut o = [0u8; 16];
    h.exec(pci, &i, &mut o)?;
    if h.caps.vport_group_manager {
        // PRM 12.15 says a PF "must" set its L2 address; harmless if not
        // needed (steering.md 4).
        let mut i = input(0x829, 0, 0x40);
        i[0x22..0x28].copy_from_slice(&e.mac);
        e.made.l2_entry = h.try_exec(pci, &i, &mut o).is_ok();
    }

    // EQ (datapath.md 9): every EQE owned by the hardware, page requests
    // and port changes mapped, never armed.
    for n in 0..1u32 << LOG_EQ {
        e.eq.entry(n).set_u8(0x3f, 1);
    }
    let mut i = input(op::CREATE_EQ, 0, 0x110 + 8 * e.eq.buf.pages);
    put(&mut i, 0x1c, 28, 24, LOG_EQ);
    put(&mut i, 0x1c, 23, 0, h.uar);
    put32(&mut i, 0x5c, (1 << EV_PORT_CHANGE) | (1 << EV_PAGE_REQUEST) | (1 << EV_CQ_ERROR));
    pas(&mut i, 0x110, &e.eq.buf);
    let mut o = [0u8; 16];
    h.exec(pci, &i, &mut o)?;
    e.eq.num = get(&o, 0x08, 7, 0);
    e.made.eq = true;

    // CQs (datapath.md 8.7): every CQE invalid and the hardware's.
    for (cq, log, dbr, made) in [(&mut e.tx_cq, LOG_TX_CQ, DBR_TX_CQ, &mut e.made.tx_cq), (&mut e.rx_cq, LOG_RX_CQ, DBR_RX_CQ, &mut e.made.rx_cq)] {
        for n in 0..1u32 << log {
            cq.entry(n).set_u8(cqe::OP_OWN, cqe::INIT);
        }
        let mut i = input(op::CREATE_CQ, 0, 0x110 + 8 * cq.buf.pages);
        put(&mut i, 0x1c, 28, 24, log);
        put(&mut i, 0x1c, 23, 0, h.uar);
        put(&mut i, 0x24, 7, 0, e.eq.num);
        put64(&mut i, 0x48, e.dbr.dev + dbr as u64);
        pas(&mut i, 0x110, &cq.buf);
        cq.num = create_id(h, pci, &i)?;
        *made = true;
    }

    // TIS (datapath.md 6.1).
    let mut i = input(op::CREATE_TIS, 0, 0xc0);
    put(&mut i, 0x44, 23, 0, h.td);
    e.tis = create_id(h, pci, &i)?;
    e.made.tis = true;

    // SQ (datapath.md 6.4, 7.1), then RST→RDY.
    let mut i = input(op::CREATE_SQ, 0, 0x110 + 8 * e.sq_buf.pages);
    put(&mut i, 0x20, 31, 31, 1); // rlkey
    put(&mut i, 0x20, 28, 28, 1); // flush_in_error_en
    put(&mut i, 0x20, 26, 24, min_inline);
    put(&mut i, 0x28, 23, 0, e.tx_cq.num);
    put(&mut i, 0x40, 31, 16, 1);
    put(&mut i, 0x4c, 23, 0, e.tis);
    put(&mut i, 0x50, 31, 28, 1); // cyclic
    put(&mut i, 0x58, 23, 0, h.pd);
    put(&mut i, 0x5c, 23, 0, h.uar);
    put64(&mut i, 0x60, e.dbr.dev + DBR_SQ as u64);
    put(&mut i, 0x70, 19, 16, 6);
    put(&mut i, 0x70, 4, 0, LOG_SQ);
    pas(&mut i, 0x110, &e.sq_buf);
    e.sqn = create_id(h, pci, &i)?;
    e.made.sq = true;
    let mut i = input(op::MODIFY_SQ, 0, 0x118);
    put(&mut i, 0x08, 23, 0, e.sqn);
    put(&mut i, 0x20, 23, 20, 1);
    let mut o = [0u8; 16];
    h.exec(pci, &i, &mut o)?;

    // RQ (datapath.md 6.3), its buffers posted, then RST→RDY.
    let mut i = input(op::CREATE_RQ, 0, 0x110 + 8 * e.rq_buf.pages);
    put(&mut i, 0x20, 31, 31, 1); // rlkey
    put(&mut i, 0x20, 28, 28, 1); // vlan_strip_disable: keep tags for the SNP
    put(&mut i, 0x20, 18, 18, 1); // flush_in_error_en
    put(&mut i, 0x28, 23, 0, e.rx_cq.num);
    put(&mut i, 0x50, 31, 28, 1);
    put(&mut i, 0x58, 23, 0, h.pd);
    put64(&mut i, 0x60, e.dbr.dev + DBR_RQ as u64);
    put(&mut i, 0x70, 19, 16, LOG_RQ_STRIDE);
    put(&mut i, 0x70, 4, 0, LOG_RQ);
    pas(&mut i, 0x110, &e.rq_buf);
    e.rqn = create_id(h, pci, &i)?;
    e.made.rq = true;
    // The WQEs are written now, but the doorbell record must read 0 until
    // the RQ is RDY (datapath.md 6.5); then it hands them all over.
    let lkey = e.lkey;
    for _ in 0..1u32 << LOG_RQ {
        e.post_rx(lkey);
    }
    e.dbr.set_be32(DBR_RQ, 0);
    let mut i = input(op::MODIFY_RQ, 0, 0x118);
    put(&mut i, 0x08, 23, 0, e.rqn);
    put(&mut i, 0x20, 23, 20, 1);
    h.exec(pci, &i, &mut o)?;
    fence(Ordering::SeqCst);
    e.dbr.set_be32(DBR_RQ, u32::from(e.rq_pc));

    // Direct TIR, own frames not looped back (datapath.md 6.2, steering.md 7).
    let mut i = input(op::CREATE_TIR, 0, 0x110);
    put(&mut i, 0x3c, 23, 0, e.rqn);
    put(&mut i, 0x44, 23, 0, h.td);
    e.tir = create_id(h, pci, &i)?;
    e.made.tir = true;

    // NIC RX flow table, a match-all group and its entry to the TIR
    // (steering.md 2, 3.2, 8 steps 11–14).
    let mut i = input(op::CREATE_FLOW_TABLE, 0, 0x40);
    put(&mut i, 0x10, 31, 24, FT_NIC_RX);
    put(&mut i, 0x18, 7, 0, 2);
    e.ft = create_id(h, pci, &i)?;
    e.made.ft = true;
    let mut i = input(op::CREATE_FLOW_GROUP, 0, 0x400);
    put(&mut i, 0x10, 31, 24, FT_NIC_RX);
    put(&mut i, 0x14, 23, 0, e.ft);
    e.fg = create_id(h, pci, &i)?;
    e.made.fg = true;
    let mut i = input(op::SET_FLOW_TABLE_ENTRY, 0, 0x348);
    put(&mut i, 0x10, 31, 24, FT_NIC_RX);
    put(&mut i, 0x14, 23, 0, e.ft);
    put32(&mut i, 0x44, e.fg);
    put(&mut i, 0x4c, 15, 0, ACTION_FWD_DEST);
    put(&mut i, 0x50, 23, 0, 1);
    put(&mut i, 0x340, 31, 24, DEST_TIR);
    put(&mut i, 0x340, 23, 0, e.tir);
    h.exec(pci, &i, &mut o)?;
    e.made.fte = true;
    if h.caps.modify_root {
        let mut i = input(op::SET_FLOW_TABLE_ROOT, 0, 0x40);
        put(&mut i, 0x10, 31, 24, FT_NIC_RX);
        put(&mut i, 0x14, 23, 0, e.ft);
        h.exec(pci, &i, &mut o)?;
    }
    trace!(
        "  data path: EQ {} CQs {}/{} TIS {} SQ {} RQ {} TIR {} flow table {} group {}, inline mode {min_inline}",
        e.eq.num, e.tx_cq.num, e.rx_cq.num, e.tis, e.sqn, e.rqn, e.tir, e.ft, e.fg
    );

    // Bring the port up if it was administratively down (PAOS, docs/spec/cmd.md 6.21).
    if let Ok(paos) = read_reg(h, pci, REG_PAOS, 16) {
        let admin = get(&paos, 0, 11, 8);
        trace!("  PAOS: admin {admin}, oper {}", get(&paos, 0, 3, 0));
        if admin != 1 && admin != 3 {
            let mut d = [0u8; 16];
            put(&mut d, 0, 23, 16, LOCAL_PORT);
            put(&mut d, 0, 11, 8, 1);
            put(&mut d, 4, 31, 31, 1);
            let _ = write_reg(h, pci, REG_PAOS, &d);
        }
    }
    Ok(())
}

/// Destroy the data path's objects (steering.md 8, teardown) and free its
/// memory. Memory of an object the device would not destroy is kept.
pub fn close(e: Eth, h: &mut Hca, pci: &mut PciIo) {
    let m = &e.made;
    let mut ok = !h.cmdq.stuck();
    if m.fte {
        let mut i = input(op::DELETE_FLOW_TABLE_ENTRY, 0, 0x40);
        put(&mut i, 0x10, 31, 24, FT_NIC_RX);
        put(&mut i, 0x14, 23, 0, e.ft);
        let mut o = [0u8; 16];
        ok &= h.exec(pci, &i, &mut o).is_ok();
    }
    if m.fg {
        let mut i = input(op::DESTROY_FLOW_GROUP, 0, 0x40);
        put(&mut i, 0x10, 31, 24, FT_NIC_RX);
        put(&mut i, 0x14, 23, 0, e.ft);
        put32(&mut i, 0x18, e.fg);
        let mut o = [0u8; 16];
        ok &= h.exec(pci, &i, &mut o).is_ok();
    }
    if m.ft {
        let mut i = input(op::DESTROY_FLOW_TABLE, 0, 0x40);
        put(&mut i, 0x10, 31, 24, FT_NIC_RX);
        put(&mut i, 0x14, 23, 0, e.ft);
        let mut o = [0u8; 16];
        ok &= h.exec(pci, &i, &mut o).is_ok();
    }
    if m.tir {
        ok &= destroy(h, pci, op::DESTROY_TIR, e.tir);
    }
    if m.rq {
        ok &= destroy(h, pci, op::DESTROY_RQ, e.rqn);
    }
    if m.sq {
        ok &= destroy(h, pci, op::DESTROY_SQ, e.sqn);
    }
    if m.tis {
        ok &= destroy(h, pci, op::DESTROY_TIS, e.tis);
    }
    if m.rx_cq {
        ok &= destroy(h, pci, op::DESTROY_CQ, e.rx_cq.num);
    }
    if m.tx_cq {
        ok &= destroy(h, pci, op::DESTROY_CQ, e.tx_cq.num);
    }
    if m.eq {
        ok &= destroy(h, pci, op::DESTROY_EQ, e.eq.num);
    }
    if m.l2_entry {
        let _ = h.simple(pci, 0x82b, 0, &[]);
    }
    if ok {
        // SAFETY: every object using these buffers is destroyed.
        unsafe {
            for b in [e.dbr, e.eq.buf, e.tx_cq.buf, e.rx_cq.buf, e.sq_buf, e.tx_bufs, e.rq_buf, e.rx_bufs] {
                b.free(pci);
            }
        }
    } else {
        say!("stormnic-mlx5: data path not fully destroyed; its memory stays allocated");
        core::mem::forget(e);
    }
}

