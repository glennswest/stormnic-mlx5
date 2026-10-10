//! HCA bring-up and teardown (PRM 8.2, 8.3; docs/spec/cmd.md 4): the
//! command queue, ENABLE_HCA, ISSI, the pages the firmware asks for,
//! capabilities, INIT_HCA, and the function-wide objects every queue needs
//! (UAR, PD, transport domain, the reserved LKey). Every command and its
//! outcome goes to the trace; a failure is an `alarm!` with the health
//! syndrome.

use alloc::vec;
use alloc::vec::Vec;

use core::time::Duration;

use uefi::boot;

use crate::cmd::{self, init, Cmdq, Error};
use crate::dma::{DmaBuf, PAGE};
use crate::layout::{self, get, header, op, put, put64};
use crate::pci::{self, PciIo};

/// Pages given to the firmware per MANAGE_PAGES and per allocation: 1 MiB,
/// four mailbox blocks of PAS entries (docs/spec/cmd.md 6.6).
const PAGES_PER_CHUNK: usize = 256;
/// How long `initializing` may stay set after the queue address is written
/// (PRM 8.2 gives no bound; docs/spec/cmd.md HW-CHECK 7).
const INIT_WAIT_MS: u32 = 10_000;
/// The ISSI this driver is written for (PRM 7.1: Rev 0.40 is ISSI 1).
const ISSI: u32 = 1;

/// QUERY_PAGES op_mod (docs/spec/cmd.md 6.5).
const PAGES_BOOT: u16 = 1;
const PAGES_INIT: u16 = 2;
pub const PAGES_REGULAR: u16 = 3;
/// MANAGE_PAGES op_mod.
const MANAGE_GIVE: u16 = 1;
const MANAGE_RETURN: u16 = 2;

/// Capability types for QUERY_HCA_CAP op_mod[15:1] (6.7).
const CAP_GENERAL: u16 = 0x0;
const CAP_ETHERNET: u16 = 0x1;
const CAP_FLOW_TABLE: u16 = 0x7;

/// A failed step, already logged.
#[derive(Debug, Clone, Copy)]
pub struct Failed;

/// What the driver uses from the capabilities (docs/spec/cmd.md 6.8–6.10).
#[derive(Default, Clone, Copy)]
pub struct Caps {
    pub log_max_cq_sz: u32,
    pub log_max_eq_sz: u32,
    pub log_max_wq_sz: u32,
    pub port_type: u32,
    pub num_ports: u32,
    pub cqe_version: u32,
    pub log_pg_sz: u32,
    pub log_uar_page_sz: u32,
    pub driver_version: bool,
    pub eth_net_offloads: bool,
    pub nic_flow_table: bool,
    pub vport_group_manager: bool,
    pub pad_tx_eth_packet: bool,
    /// Ethernet offload caps, 0x00 13:12 (0 L2, 1 vport context, 2 none).
    pub wqe_inline_mode: u32,
    /// NIC RX flow table properties (6.10).
    pub ft_support: bool,
    pub max_ft_level: u32,
    pub log_max_ft_size: u32,
    pub modify_root: bool,
}

/// Pages lent to the firmware: one allocation.
struct Chunk {
    buf: DmaBuf,
}

pub struct Hca {
    pub cmdq: Cmdq,
    chunks: Vec<Chunk>,
    /// Pages the firmware holds now.
    pub lent: usize,
    pub caps: Caps,
    pub uar: u32,
    /// Byte offset of the UAR page in BAR 0.
    pub uar_off: u64,
    pub pd: u32,
    pub td: u32,
    /// The reserved LKey: physical addresses, no MKey (PRM 6.3).
    pub lkey: u32,
    /// PciIo attributes to put back when the driver lets go.
    attrs: u64,
    enabled: bool,
    initialized: bool,
}

/// A command's input buffer, header filled in.
pub fn input(opcode: u16, op_mod: u16, len: usize) -> Vec<u8> {
    let mut b = vec![0u8; len];
    header(&mut b, opcode, op_mod);
    b
}

fn read_health(pci: &mut PciIo) -> (u8, u32) {
    let h = cmd::read_be(pci, init::HEALTH).unwrap_or(0);
    ((h >> 24) as u8, h & 0x00ff_ffff)
}

impl Hca {
    /// Log a command's outcome. Quiet commands log nothing on failure; the
    /// caller decides whether the failure matters.
    fn report(&mut self, pci: &mut PciIo, input: &[u8], r: Result<(), Error>, quiet: bool) -> Result<(), Error> {
        let opcode = get(input, 0, 31, 16) as u16;
        let op_mod = get(input, 4, 15, 0);
        let name = layout::op_name(opcode);
        match r {
            Ok(()) => trace!("    {name} ({op_mod:#x}): ok"),
            Err(e) if quiet => trace!("    {name} ({op_mod:#x}): {e}"),
            Err(e) => {
                let (syn, count) = read_health(pci);
                alarm!(
                    "stormnic-mlx5: {name} ({op_mod:#x}): {e}; health syndrome {syn:#x} ({}), counter {count:#x}",
                    layout::health_name(syn)
                );
            }
        }
        r
    }

    /// Run a command; failures are alarms.
    pub fn exec(&mut self, pci: &mut PciIo, input: &[u8], out: &mut [u8]) -> Result<(), Failed> {
        let r = self.cmdq.exec(pci, input, out);
        self.report(pci, input, r, false).map_err(|_| Failed)
    }

    /// Run a command whose failure the caller handles.
    pub fn try_exec(&mut self, pci: &mut PciIo, input: &[u8], out: &mut [u8]) -> Result<(), Error> {
        let r = self.cmdq.exec(pci, input, out);
        self.report(pci, input, r, true)
    }

    /// A command with only header input and a 16-byte output; the output.
    pub fn simple(&mut self, pci: &mut PciIo, opcode: u16, op_mod: u16, args: &[(usize, u32)]) -> Result<[u8; 16], Failed> {
        let mut i = input(opcode, op_mod, 16);
        for &(off, v) in args {
            layout::put32(&mut i, off, v);
        }
        let mut o = [0u8; 16];
        self.exec(pci, &i, &mut o)?;
        Ok(o)
    }

    fn query_pages(&mut self, pci: &mut PciIo, op_mod: u16) -> Result<i32, Failed> {
        let o = self.simple(pci, op::QUERY_PAGES, op_mod, &[])?;
        Ok(get(&o, 0x0c, 31, 0) as i32)
    }

    /// Give the firmware `n` more pages (MANAGE_PAGES, ALLOCATION_SUCCESS).
    pub fn give_pages(&mut self, pci: &mut PciIo, n: usize) -> Result<(), Failed> {
        let mut left = n;
        while left > 0 {
            let pages = left.min(PAGES_PER_CHUNK);
            let buf = match DmaBuf::new(pci, pages * PAGE) {
                Ok(b) => b,
                Err(e) => {
                    alarm!("stormnic-mlx5: {pages} pages for the firmware: {:?}", e.status());
                    // Tell the firmware it gets none (op_mod 0, ALLOCATION_FAIL).
                    let _ = self.simple(pci, op::MANAGE_PAGES, 0, &[]);
                    return Err(Failed);
                }
            };
            let mut i = input(op::MANAGE_PAGES, MANAGE_GIVE, 0x10 + 8 * pages);
            put(&mut i, 0x0c, 31, 0, pages as u32);
            for p in 0..pages {
                put64(&mut i, 0x10 + 8 * p, buf.dev + (p * PAGE) as u64);
            }
            let mut o = [0u8; 16];
            if self.exec(pci, &i, &mut o).is_err() {
                // SAFETY: the firmware refused the pages, so it never uses them.
                unsafe { buf.free(pci) };
                return Err(Failed);
            }
            self.chunks.push(Chunk { buf });
            self.lent += pages;
            left -= pages;
        }
        Ok(())
    }

    /// Ask for every page back (MANAGE_PAGES, HCA_RETURN_PAGES) until the
    /// firmware holds none. True when all came back.
    fn reclaim(&mut self, pci: &mut PciIo) -> bool {
        let mut rounds = 0;
        while self.lent > 0 && rounds < 1000 {
            rounds += 1;
            let want = self.lent.min(512);
            let mut i = input(op::MANAGE_PAGES, MANAGE_RETURN, 0x10);
            put(&mut i, 0x0c, 31, 0, want as u32);
            let mut o = vec![0u8; 0x10 + 8 * want];
            if self.exec(pci, &i, &mut o).is_err() {
                return false;
            }
            let got = get(&o, 0x08, 31, 0) as usize;
            if got == 0 {
                say!("stormnic-mlx5: the firmware returned no pages; {} still lent", self.lent);
                return false;
            }
            self.lent = self.lent.saturating_sub(got);
        }
        trace!("    every page returned ({rounds} MANAGE_PAGES)");
        self.lent == 0
    }

    /// QUERY_PAGES `op_mod` and give what it asks for.
    fn satisfy(&mut self, pci: &mut PciIo, op_mod: u16, what: &str) -> Result<(), Failed> {
        let n = self.query_pages(pci, op_mod)?;
        trace!("    {what} pages wanted: {n}");
        if n > 0 {
            self.give_pages(pci, n as usize)?;
        }
        Ok(())
    }

    fn query_cap(&mut self, pci: &mut PciIo, kind: u16) -> Result<Vec<u8>, Failed> {
        let i = input(op::QUERY_HCA_CAP, (kind << 1) | 1, 16);
        let mut o = vec![0u8; 0x1010];
        self.exec(pci, &i, &mut o)?;
        o.drain(..0x10);
        Ok(o)
    }

    fn read_caps(&mut self, pci: &mut PciIo) -> Result<(), Failed> {
        let g = self.query_cap(pci, CAP_GENERAL)?;
        let mut c = Caps {
            log_max_cq_sz: get(&g, 0x18, 23, 16),
            log_max_eq_sz: get(&g, 0x1c, 31, 24),
            log_max_wq_sz: get(&g, 0x78, 4, 0),
            port_type: get(&g, 0x34, 9, 8),
            num_ports: get(&g, 0x34, 7, 0),
            cqe_version: get(&g, 0x3c, 3, 0),
            log_pg_sz: get(&g, 0x48, 7, 0),
            log_uar_page_sz: get(&g, 0x90, 15, 0),
            driver_version: get(&g, 0x4c, 30, 30) != 0,
            eth_net_offloads: get(&g, 0x40, 3, 3) != 0,
            nic_flow_table: get(&g, 0x34, 25, 25) != 0,
            vport_group_manager: get(&g, 0x34, 31, 31) != 0,
            pad_tx_eth_packet: get(&g, 0x4c, 29, 29) != 0,
            ..Caps::default()
        };
        trace!(
            "    caps: port_type {} ports {} cqe_version {} log_pg_sz {} log_uar_page_sz {} cq_sz 2^{} eq_sz 2^{} wq_sz 2^{} \
             eth {} flow_table {} vport_group_manager {} pad_tx {} cmdif_checksum {}",
            c.port_type, c.num_ports, c.cqe_version, c.log_pg_sz, c.log_uar_page_sz, c.log_max_cq_sz, c.log_max_eq_sz,
            c.log_max_wq_sz, c.eth_net_offloads, c.nic_flow_table, c.vport_group_manager, c.pad_tx_eth_packet,
            get(&g, 0x40, 15, 14)
        );
        if c.eth_net_offloads {
            let e = self.query_cap(pci, CAP_ETHERNET)?;
            c.wqe_inline_mode = get(&e, 0x00, 13, 12);
            trace!(
                "    ethernet caps: wqe_inline_mode {} self_lb_uc {} self_lb_mc {}",
                c.wqe_inline_mode,
                get(&e, 0, 21, 21),
                get(&e, 0, 22, 22)
            );
        }
        if c.nic_flow_table {
            let f = self.query_cap(pci, CAP_FLOW_TABLE)?;
            c.ft_support = get(&f, 0x40, 31, 31) != 0;
            c.log_max_ft_size = get(&f, 0x44, 29, 24);
            c.max_ft_level = get(&f, 0x44, 7, 0);
            c.modify_root = get(&f, 0x40, 27, 27) != 0;
            trace!(
                "    NIC RX flow table: supported {} modify_root {} log_max_ft_size {} max_ft_level {} dmac match {}",
                c.ft_support,
                c.modify_root,
                c.log_max_ft_size,
                c.max_ft_level,
                get(&f, 0x60, 31, 31)
            );
        }
        self.caps = c;
        Ok(())
    }

    fn set_driver_version(&mut self, pci: &mut PciIo) {
        // "OS_name,Driver_name,Main.Minor.Subminor" (docs/spec/cmd.md 6.13).
        let v = alloc::format!(
            "UEFI,stormnic-mlx5,{}.{:03}.{:06}",
            env!("CARGO_PKG_VERSION_MAJOR"),
            env!("CARGO_PKG_VERSION_MINOR").parse::<u32>().unwrap_or(0),
            env!("CARGO_PKG_VERSION_PATCH").parse::<u32>().unwrap_or(0)
        );
        let mut i = input(0x10d, 0, 0x50);
        let n = v.len().min(63);
        i[0x10..0x10 + n].copy_from_slice(&v.as_bytes()[..n]);
        let mut o = [0u8; 16];
        let _ = self.try_exec(pci, &i, &mut o);
    }

    fn alloc_id(&mut self, pci: &mut PciIo, opcode: u16) -> Result<u32, Failed> {
        let o = self.simple(pci, opcode, 0, &[])?;
        Ok(get(&o, 0x08, 23, 0))
    }
}

/// Bring the function up, through the function-wide objects.
pub fn open(pci: &mut PciIo) -> Result<Hca, Failed> {
    let attrs = pci.get_attributes().unwrap_or(0);
    let supported = pci.supported_attributes().unwrap_or(0);
    let want = (pci::ATTR_MEMORY | pci::ATTR_BUS_MASTER | pci::ATTR_DUAL_ADDRESS_CYCLE) & supported;
    if let Err(e) = pci.enable_attributes(want) {
        alarm!("stormnic-mlx5: PCI attributes {want:#x}: {:?}", e.status());
        return Err(Failed);
    }

    let fw = cmd::read_be(pci, init::FW_REV).map_err(|_| Failed)?;
    let rev = cmd::read_be(pci, init::CMDIF_REV).map_err(|_| Failed)?;
    let lo = cmd::read_be(pci, init::CMDQ_LO).map_err(|_| Failed)?;
    trace!(
        "  firmware {}.{}.{}, command interface rev {}, {} command entries, stride 2^{}",
        fw & 0xffff,
        fw >> 16,
        rev & 0xffff,
        rev >> 16,
        1u32 << ((lo >> 4) & 0xf),
        lo & 0xf
    );

    let cmdq = match Cmdq::new(pci) {
        Ok(q) => q,
        Err(e) => {
            alarm!("stormnic-mlx5: command queue memory: {:?}", e.status());
            let _ = pci.set_attributes(attrs);
            return Err(Failed);
        }
    };
    let mut h = Hca {
        cmdq,
        chunks: Vec::new(),
        lent: 0,
        caps: Caps::default(),
        uar: 0,
        uar_off: 0,
        pd: 0,
        td: 0,
        lkey: 0,
        attrs,
        enabled: false,
        initialized: false,
    };
    match bring_up(&mut h, pci) {
        Ok(()) => Ok(h),
        Err(Failed) => {
            close(h, pci);
            Err(Failed)
        }
    }
}

fn bring_up(h: &mut Hca, pci: &mut PciIo) -> Result<(), Failed> {
    // PRM 8.2: the queue address, high dword first; the low dword carries
    // nic_interface 0 (full driver).
    let a = h.cmdq.queue_addr();
    cmd::write_be(pci, init::CMDQ_HI, (a >> 32) as u32).map_err(|_| Failed)?;
    cmd::write_be(pci, init::CMDQ_LO, a as u32 & 0xffff_f000).map_err(|_| Failed)?;
    let mut waited = 0;
    loop {
        let v = cmd::read_be(pci, init::INITIALIZING).map_err(|_| Failed)?;
        if v >> 31 == 0 {
            break;
        }
        if waited >= INIT_WAIT_MS {
            alarm!("stormnic-mlx5: the device stayed `initializing` for {} s (init segment 1FCh = {v:#010x})", INIT_WAIT_MS / 1000);
            return Err(Failed);
        }
        boot::stall(Duration::from_millis(1));
        waited += 1;
    }
    trace!("  command queue at {a:#x}, device ready after {waited} ms");

    h.simple(pci, op::ENABLE_HCA, 0, &[])?;
    h.enabled = true;

    // ISSI (PRM 7.1): BAD_OP means ISSI 0 only, and no SET_ISSI.
    let i = input(op::QUERY_ISSI, 0, 16);
    let mut o = [0u8; 0x70];
    match h.try_exec(pci, &i, &mut o) {
        Ok(()) => {
            let supported = o[0x6f];
            let current = get(&o, 0x08, 15, 0);
            let issi = if supported & (1 << ISSI) != 0 { ISSI } else { 0 };
            trace!("    ISSI: current {current}, supported mask {supported:#x}, using {issi}");
            h.simple(pci, op::SET_ISSI, 0, &[(0x08, issi)])?;
        }
        Err(Error::Status(0x02, _)) => trace!("    ISSI: QUERY_ISSI not supported, ISSI 0"),
        Err(e) => {
            alarm!("stormnic-mlx5: QUERY_ISSI: {e}");
            return Err(Failed);
        }
    }

    h.satisfy(pci, PAGES_BOOT, "boot")?;
    h.read_caps(pci)?;
    if h.caps.port_type != 1 {
        say!("stormnic-mlx5: port type {} is not Ethernet (1); not taking the NIC", h.caps.port_type);
        return Err(Failed);
    }
    h.satisfy(pci, PAGES_INIT, "init")?;
    h.simple(pci, op::INIT_HCA, 0, &[])?;
    h.initialized = true;
    if h.caps.driver_version {
        h.set_driver_version(pci);
    }

    h.uar = h.alloc_id(pci, op::ALLOC_UAR)?;
    h.uar_off = u64::from(h.uar) << (12 + h.caps.log_uar_page_sz);
    h.pd = h.alloc_id(pci, op::ALLOC_PD)?;
    h.td = h.alloc_id(pci, op::ALLOC_TRANSPORT_DOMAIN)?;
    let o = h.simple(pci, op::QUERY_SPECIAL_CONTEXTS, 0, &[])?;
    h.lkey = get(&o, 0x0c, 31, 0);
    trace!(
        "  HCA initialised: {} pages lent, UAR {} (BAR offset {:#x}), PD {}, transport domain {}, reserved LKey {:#x}",
        h.lent, h.uar, h.uar_off, h.pd, h.td, h.lkey
    );
    Ok(())
}

/// Tear the function down (PRM 8.3) and give back everything: the pages
/// when the firmware returns them, the queue, the PCI attributes. If the
/// firmware cannot be shut down cleanly its memory is left allocated, since
/// it may still be using it. The objects and EQ are `eth.rs`'s to destroy
/// first.
pub fn close(mut h: Hca, pci: &mut PciIo) {
    let mut clean = !h.cmdq.stuck();
    if h.initialized {
        if h.td != 0 {
            let _ = h.simple(pci, op::DEALLOC_TRANSPORT_DOMAIN, 0, &[(0x08, h.td)]);
        }
        if h.pd != 0 {
            let _ = h.simple(pci, op::DEALLOC_PD, 0, &[(0x08, h.pd)]);
        }
        if h.uar != 0 {
            let _ = h.simple(pci, op::DEALLOC_UAR, 0, &[(0x08, h.uar)]);
        }
        clean &= h.simple(pci, op::TEARDOWN_HCA, 0, &[(0x08, 0)]).is_ok();
    }
    if h.lent > 0 && clean {
        clean = h.reclaim(pci);
    }
    if h.enabled {
        clean &= h.simple(pci, op::DISABLE_HCA, 0, &[]).is_ok();
    }
    let _ = pci.set_attributes(h.attrs);
    if clean {
        for c in h.chunks.drain(..) {
            // SAFETY: returned by the firmware, and the function is disabled.
            unsafe { c.buf.free(pci) };
        }
        // SAFETY: DISABLE_HCA done; the device no longer reads the queue.
        unsafe { h.cmdq.free(pci) };
        trace!("stormnic-mlx5: HCA torn down, memory returned");
    } else {
        say!(
            "stormnic-mlx5: the HCA did not shut down cleanly; its {} pages and command queue stay allocated",
            h.lent
        );
        core::mem::forget(h);
    }
}

/// ExitBootServices: stop all DMA without freeing anything or logging
/// (PRM 6.12: TEARDOWN_HCA panic close stops bus mastering), then
/// DISABLE_HCA and bus master off.
pub fn quiesce(h: &mut Hca, pci: &mut PciIo) {
    let mut i = [0u8; 16];
    let mut o = [0u8; 16];
    if h.initialized {
        header(&mut i, op::TEARDOWN_HCA, 0);
        put(&mut i, 0x08, 15, 0, 1);
        let _ = h.cmdq.exec(pci, &i, &mut o);
        h.initialized = false;
    }
    if h.enabled {
        i = [0u8; 16];
        header(&mut i, op::DISABLE_HCA, 0);
        let _ = h.cmdq.exec(pci, &i, &mut o);
        h.enabled = false;
    }
    let _ = pci.set_attributes(h.attrs & !pci::ATTR_BUS_MASTER);
}

/// The firmware asked for pages (or to give some back) through the EQ.
pub fn page_request(h: &mut Hca, pci: &mut PciIo) {
    match h.query_pages(pci, PAGES_REGULAR) {
        Ok(n) if n > 0 => {
            trace!("stormnic-mlx5: firmware page request: {n} more");
            let _ = h.give_pages(pci, n as usize);
        }
        Ok(n) => trace!("stormnic-mlx5: firmware page request: {n} (nothing to give)"),
        Err(Failed) => {}
    }
}
