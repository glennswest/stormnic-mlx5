//! `EFI_SIMPLE_NETWORK_PROTOCOL` on a child handle, with a MAC device path
//! (UEFI spec, "Simple Network Protocol"). Each mlx5 port is its own PCI
//! function, so a controller has exactly one child. The data path behind it
//! is `eth.rs`.
//!
//! Every function raises the TPL to `TPL_CALLBACK`, so SNP calls, the
//! `WaitForPacket` notify function and the firmware's timer callbacks never
//! run inside one another. After ExitBootServices every call returns
//! `DEVICE_ERROR`.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::ptr;

use uefi::Status;
use uefi_raw::protocol::device_path::DevicePathProtocol;
use uefi_raw::protocol::network::snp::{
    InterruptStatus, NetworkMode, NetworkState, NetworkStatistics, ReceiveFlags, SimpleNetworkProtocol,
};
use uefi_raw::table::boot::{EventType, Tpl};
use uefi_raw::{Boolean, Event, Guid, Handle, IpAddress, MacAddress};

use crate::eth::{self, Mac, Send};
use crate::pci::PciIo;
use crate::publish::{self, Failed, Firmware};
use uefi::Identify;
use crate::{bs, Nic, TplGuard, OPEN_BY_CHILD_CONTROLLER};

/// `EFI_SIMPLE_NETWORK_PROTOCOL_REVISION`.
const REVISION: u64 = 0x0001_0000;
/// Multicast addresses the filter holds (the mode structure has room for 16).
const MAX_MCAST: usize = 16;
/// Filters implemented: unicast to the port MAC, broadcast, listed multicast.
const FILTERS: u32 = ReceiveFlags::UNICAST.bits() | ReceiveFlags::MULTICAST.bits() | ReceiveFlags::BROADCAST.bits();
/// Frames logged one line each, per direction and port.
const LOG_FRAMES: u32 = 16;
/// `GetStatus` re-reads the port's operational state this often (in calls),
/// in case a port-change event was missed.
const LINK_POLL_CALLS: u32 = 256;
/// Messaging device path, MAC address node (UEFI spec, "MAC Address Device Path").
const MSG: u8 = 3;
const MSG_MAC: u8 = 11;
const MAC_NODE_LEN: usize = 37;
/// `IfType` for Ethernet (RFC 3232, ARP hardware type 1).
const IF_ETHERNET: u8 = 1;

/// The NIC's SNP. `snp` comes first so the `this` pointer every SNP
/// function receives is also a pointer to the child.
#[repr(C)]
pub struct Child {
    snp: SimpleNetworkProtocol,
    mode: NetworkMode,
    nic: *mut Nic,
    pub handle: Handle,
    path: Vec<u8>,
    /// What of it is published (`publish.rs`).
    state: publish::State,
    rx_logged: u32,
    tx_logged: u32,
    /// An own frame looped back has been logged.
    own_logged: bool,
    /// A frame over `eth::MAX_FRAME` has been logged.
    long_logged: bool,
    status_calls: u32,
}

fn mac32(mac: [u8; 6]) -> MacAddress {
    let mut a = [0u8; 32];
    a[..6].copy_from_slice(&mac);
    MacAddress(a)
}

fn mac6(a: &MacAddress) -> [u8; 6] {
    a.0[..6].try_into().unwrap()
}

/// The parent's device path, its end node replaced by a MAC node and a new end.
fn child_path(parent: *const DevicePathProtocol, mac: [u8; 6]) -> Vec<u8> {
    let mut path = Vec::new();
    let mut p = parent.cast::<u8>();
    // SAFETY: a device path is a list of nodes, each giving its own length,
    // ending with the end-of-path node (type 0x7f, subtype 0xff).
    unsafe {
        loop {
            let (kind, sub) = (*p, *p.add(1));
            let len = usize::from(u16::from_le_bytes([*p.add(2), *p.add(3)]));
            if (kind == 0x7f && sub == 0xff) || len < 4 {
                break;
            }
            path.extend_from_slice(core::slice::from_raw_parts(p, len));
            p = p.add(len);
        }
    }
    let mut node = [0u8; MAC_NODE_LEN];
    node[0] = MSG;
    node[1] = MSG_MAC;
    node[2..4].copy_from_slice(&(MAC_NODE_LEN as u16).to_le_bytes());
    node[4..10].copy_from_slice(&mac);
    node[36] = IF_ETHERNET;
    path.extend_from_slice(&node);
    path.extend_from_slice(&[0x7f, 0xff, 4, 0]);
    path
}

/// The boot services behind `publish::Firmware`, for one child.
struct Boot<'a> {
    c: &'a mut Child,
    agent: Handle,
    controller: Handle,
}

fn result(st: Status) -> Result<(), Status> {
    if st.is_error() {
        Err(st)
    } else {
        Ok(())
    }
}

static PATH_GUID: Guid = DevicePathProtocol::GUID;
static SNP_GUID: Guid = SimpleNetworkProtocol::GUID;

impl Firmware for Boot<'_> {
    type Status = Status;

    fn create_event(&mut self) -> Result<(), Status> {
        let ctx: *mut Child = &raw mut *self.c;
        let mut ev: Event = ptr::null_mut();
        // SAFETY: the child is boxed and is not freed while the event is open
        // (`publish::State::free`).
        result(unsafe { (bs().create_event)(EventType::NOTIFY_WAIT, Tpl::CALLBACK, Some(wait_notify), ctx.cast(), &mut ev) })?;
        self.c.snp.wait_for_packet = ev;
        Ok(())
    }

    fn close_event(&mut self) {
        let _ = unsafe { (bs().close_event)(self.c.snp.wait_for_packet) };
        self.c.snp.wait_for_packet = ptr::null_mut();
    }

    fn install(&mut self) -> Result<(), Status> {
        let mut h: Handle = ptr::null_mut();
        let path: *const c_void = self.c.path.as_ptr().cast();
        let snp: *const c_void = (&raw const self.c.snp).cast();
        // SAFETY: GUID/interface pairs ending with a null GUID (UEFI 2.10
        // 7.3, InstallMultipleProtocolInterfaces); both interfaces live in
        // the child, which is kept while they are installed.
        result(unsafe {
            (bs().install_multiple_protocol_interfaces)(&mut h, &raw const PATH_GUID, path, &raw const SNP_GUID, snp, ptr::null::<Guid>())
        })?;
        self.c.handle = h;
        Ok(())
    }

    fn uninstall(&mut self) -> Result<(), Status> {
        let path: *const c_void = self.c.path.as_ptr().cast();
        let snp: *const c_void = (&raw const self.c.snp).cast();
        // SAFETY: as in `install`; on a failure the firmware reinstalls what
        // it had removed, so both stay or both go.
        result(unsafe {
            (bs().uninstall_multiple_protocol_interfaces)(self.c.handle, &raw const PATH_GUID, path, &raw const SNP_GUID, snp, ptr::null::<Guid>())
        })
    }

    fn open_by_child(&mut self) -> Result<(), Status> {
        let mut iface: *mut c_void = ptr::null_mut();
        let (c, agent, child) = (self.controller, self.agent, self.c.handle);
        result(unsafe { (bs().open_protocol)(c, &PciIo::GUID, &mut iface, agent, child, OPEN_BY_CHILD_CONTROLLER) })
    }

    fn close_by_child(&mut self) {
        let _ = unsafe { (bs().close_protocol)(self.controller, &PciIo::GUID, self.agent, self.c.handle) };
    }
}

impl Child {
    /// The child handle of `nic`: `WaitForPacket`, device path and SNP
    /// installed, the parent's PCI I/O opened `BY_CHILD_CONTROLLER`; all of
    /// it or none (`publish.rs`). A failure that left the interfaces
    /// installed hands the child back with the error: it must be kept.
    ///
    /// # Safety
    /// `nic` outlives the child; `parent` is the controller's device path.
    pub unsafe fn install(
        nic: *mut Nic,
        parent: *const DevicePathProtocol,
        agent: Handle,
        controller: Handle,
    ) -> Result<Box<Child>, (Status, Option<Box<Child>>)> {
        let n = unsafe { &mut *nic };
        let (num, mac, link) = (n.eth.num(), n.eth.mac(), n.eth.link_up());
        let mut c = Box::new(Child {
            snp: SimpleNetworkProtocol {
                revision: REVISION,
                start,
                stop,
                initialize,
                reset,
                shutdown,
                receive_filters,
                station_address,
                statistics,
                multicast_ip_to_mac,
                non_volatile_data,
                get_status,
                transmit,
                receive,
                wait_for_packet: ptr::null_mut(),
                mode: ptr::null_mut(),
            },
            mode: NetworkMode {
                state: NetworkState::STOPPED,
                hw_address_size: 6,
                media_header_size: 14,
                max_packet_size: 1500,
                nv_ram_size: 0,
                nv_ram_access_size: 0,
                receive_filter_mask: FILTERS,
                receive_filter_setting: 0,
                max_mcast_filter_count: MAX_MCAST as u32,
                mcast_filter_count: 0,
                mcast_filter: [MacAddress([0; 32]); 16],
                current_address: mac32(mac),
                broadcast_address: mac32([0xff; 6]),
                permanent_address: mac32(mac),
                if_type: IF_ETHERNET,
                mac_address_changeable: Boolean::FALSE,
                multiple_tx_supported: Boolean::TRUE,
                media_present_supported: Boolean::TRUE,
                media_present: link.into(),
            },
            nic,
            handle: ptr::null_mut(),
            path: child_path(parent, mac),
            state: publish::State::default(),
            rx_logged: 0,
            tx_logged: 0,
            own_logged: false,
            long_logged: false,
            status_calls: 0,
        });
        c.snp.mode = &raw mut c.mode;
        let mut st = c.state;
        let r = publish::install(&mut Boot { c: &mut c, agent, controller }, &mut st);
        c.state = st;
        if let Err(Failed { status, kept }) = r {
            alarm!("  port {num}: child handle not published: {status:?}");
            if kept {
                // Its interfaces stayed installed: the firmware still points
                // into it, so it lives on, to be withdrawn by Stop.
                alarm!("  port {num}: and its interfaces could not be removed; the child is kept");
                return Err((status, Some(c)));
            }
            return Err((status, None));
        }
        trace!(
            "  port {num}: SNP installed on a child handle, MAC {}, media {}",
            Mac(mac),
            if link { "present" } else { "absent" }
        );
        Ok(c)
    }

    /// Take the child down: close the parent's PCI I/O, uninstall the SNP
    /// and the device path together, close `WaitForPacket`. False, and the
    /// child left published as it was, when the firmware refuses, e.g.
    /// because MNP will not let go of the SNP: then it must not be freed.
    pub fn uninstall(&mut self, agent: Handle, controller: Handle) -> bool {
        let mut st = self.state;
        let r = publish::uninstall(&mut Boot { c: self, agent, controller }, &mut st);
        self.state = st;
        match r {
            Ok(()) => true,
            Err(e) => {
                say!("stormnic-mlx5: SNP uninstall refused ({e:?}); the child is kept");
                false
            }
        }
    }

    fn port(&mut self) -> &mut eth::Eth {
        // SAFETY: the NIC outlives its children.
        let n = unsafe { &mut *self.nic };
        &mut n.eth
    }

    fn num(&mut self) -> u32 {
        self.port().num()
    }

    /// Drain the EQ and both CQs, so frames that have arrived are queued
    /// (#12). `Receive` and `WaitForPacket` do this themselves: a caller
    /// need not call GetStatus to see a frame. stormbootx calls GetStatus
    /// only before it transmits, so an OFFER that waited for it reached
    /// smoltcp after the next DISCOVER had changed the xid.
    fn poll(&mut self) {
        // SAFETY: the NIC outlives its children, and TPL_CALLBACK keeps
        // every other user of it out.
        let n = unsafe { &mut *self.nic };
        let pci = unsafe { &mut *n.pci };
        n.eth.poll(&mut n.hca, pci);
    }

    /// Poll the queues and events and refresh `MediaPresent`.
    fn refresh(&mut self) {
        self.poll();
        // SAFETY: as in `poll`.
        let n = unsafe { &mut *self.nic };
        let pci = unsafe { &mut *n.pci };
        self.status_calls = self.status_calls.wrapping_add(1);
        if self.status_calls % LINK_POLL_CALLS == 1 {
            n.eth.refresh_link(&mut n.hca, pci);
        }
        self.mode.media_present = n.eth.link_up().into();
    }

    fn log_frame(&mut self, dir: &str, f: &[u8], len: usize) {
        let logged = if dir == "rx" { &mut self.rx_logged } else { &mut self.tx_logged };
        if *logged >= LOG_FRAMES {
            return;
        }
        *logged += 1;
        let last = *logged == LOG_FRAMES;
        let num = self.num();
        trace!(
            "stormnic-mlx5: port {num} {dir}: {len} bytes {} <- {} type {:04x}{}",
            Mac(f[0..6].try_into().unwrap()),
            Mac(f[6..12].try_into().unwrap()),
            u16::from_be_bytes([f[12], f[13]]),
            if last { " (no more logged)" } else { "" }
        );
    }

    /// Should a frame to `dst` from `src` reach the caller? Own frames looped
    /// back by the adapter never do. The steering delivers every frame
    /// (`eth.rs`), so this is the receive filter.
    fn wanted(&mut self, dst: [u8; 6], src: [u8; 6], len: usize) -> bool {
        let mac = self.port().mac();
        let on = |f: ReceiveFlags| self.mode.receive_filter_setting & f.bits() != 0;
        if src == mac {
            // The TIR's self-loopback block should keep these out (PRM
            // 3.4); say so once if one gets through.
            if !core::mem::replace(&mut self.own_logged, true) {
                say!(
                    "stormnic-mlx5: port {} rx: own frame looped back ({len} bytes to {}), dropped; no more logged",
                    self.num(),
                    Mac(dst)
                );
            }
            false
        } else if dst == mac {
            on(ReceiveFlags::UNICAST)
        } else if dst == [0xff; 6] {
            on(ReceiveFlags::BROADCAST)
        } else if dst[0] & 1 != 0 {
            on(ReceiveFlags::MULTICAST)
                && self.mode.mcast_filter[..self.mode.mcast_filter_count as usize].iter().any(|m| mac6(m) == dst)
        } else {
            false
        }
    }
}

/// The child behind `this`, with the TPL raised; `None` after
/// ExitBootServices.
///
/// # Safety
/// `this` is an SNP this driver installed.
unsafe fn enter<'a>(this: *const SimpleNetworkProtocol) -> Option<(&'a mut Child, TplGuard)> {
    let guard = TplGuard::raise(Tpl::CALLBACK);
    let c = unsafe { &mut *this.cast_mut().cast::<Child>() };
    if unsafe { (*c.nic).dead } {
        return None;
    }
    Some((c, guard))
}

/// `NOT_STARTED` if stopped, `DEVICE_ERROR` if started but not initialised.
fn not_initialized(c: &Child) -> Status {
    if c.mode.state == NetworkState::STOPPED {
        Status::NOT_STARTED
    } else {
        Status::DEVICE_ERROR
    }
}

macro_rules! enter {
    ($this:expr) => {
        match unsafe { enter($this) } {
            Some(c) => c,
            None => return Status::DEVICE_ERROR,
        }
    };
}

macro_rules! initialized {
    ($c:expr) => {
        if $c.mode.state != NetworkState::INITIALIZED {
            return not_initialized($c);
        }
    };
}

unsafe extern "efiapi" fn start(this: *const SimpleNetworkProtocol) -> Status {
    let (c, _tpl) = enter!(this);
    if c.mode.state != NetworkState::STOPPED {
        return Status::ALREADY_STARTED;
    }
    c.mode.state = NetworkState::STARTED;
    trace!("stormnic-mlx5: port {} SNP: started", c.num());
    Status::SUCCESS
}

unsafe extern "efiapi" fn stop(this: *const SimpleNetworkProtocol) -> Status {
    let (c, _tpl) = enter!(this);
    match c.mode.state {
        NetworkState::STARTED => {
            c.mode.state = NetworkState::STOPPED;
            trace!("stormnic-mlx5: port {} SNP: stopped", c.num());
            Status::SUCCESS
        }
        NetworkState::STOPPED => Status::NOT_STARTED,
        _ => Status::DEVICE_ERROR,
    }
}

unsafe extern "efiapi" fn initialize(this: *const SimpleNetworkProtocol, _rx_extra: usize, _tx_extra: usize) -> Status {
    let (c, _tpl) = enter!(this);
    if c.mode.state != NetworkState::STARTED {
        return if c.mode.state == NetworkState::STOPPED { Status::NOT_STARTED } else { Status::DEVICE_ERROR };
    }
    // Frames that arrived before anyone listened are stale: drop them,
    // those still in the CQ too.
    c.poll();
    while c.port().peek_rx().is_some() {
        c.port().pop_rx();
    }
    // Unicast and broadcast from the start; a caller that wants otherwise
    // says so with ReceiveFilters.
    c.mode.receive_filter_setting = ReceiveFlags::UNICAST.bits() | ReceiveFlags::BROADCAST.bits();
    c.mode.mcast_filter_count = 0;
    c.refresh();
    c.mode.state = NetworkState::INITIALIZED;
    let media = if bool::from(c.mode.media_present) { "present" } else { "absent" };
    trace!("stormnic-mlx5: port {} SNP: initialized, media {media}", c.num());
    Status::SUCCESS
}

unsafe extern "efiapi" fn reset(this: *const SimpleNetworkProtocol, _extended: Boolean) -> Status {
    let (c, _tpl) = enter!(this);
    initialized!(c);
    // The queues run continuously; there is nothing to reinitialise.
    Status::SUCCESS
}

unsafe extern "efiapi" fn shutdown(this: *const SimpleNetworkProtocol) -> Status {
    let (c, _tpl) = enter!(this);
    initialized!(c);
    c.mode.receive_filter_setting = 0;
    c.mode.mcast_filter_count = 0;
    c.mode.state = NetworkState::STARTED;
    trace!("stormnic-mlx5: port {} SNP: shut down", c.num());
    Status::SUCCESS
}

unsafe extern "efiapi" fn receive_filters(
    this: *const SimpleNetworkProtocol,
    enable: ReceiveFlags,
    disable: ReceiveFlags,
    reset_mcast: Boolean,
    count: usize,
    list: *const MacAddress,
) -> Status {
    let (c, _tpl) = enter!(this);
    initialized!(c);
    let reset_mcast = bool::from(reset_mcast);
    if enable.bits() & !FILTERS != 0 {
        return Status::INVALID_PARAMETER;
    }
    let mut new = Vec::new();
    if !reset_mcast && count != 0 {
        if count > MAX_MCAST || list.is_null() {
            return Status::INVALID_PARAMETER;
        }
        // SAFETY: the caller passes `count` addresses.
        let l = unsafe { core::slice::from_raw_parts(list, count) };
        if l.iter().any(|m| m.0[0] & 1 == 0) {
            return Status::INVALID_PARAMETER;
        }
        new.extend(l.iter().map(mac6));
    }
    // Disabling a filter that is not supported disables nothing.
    let setting = (c.mode.receive_filter_setting | enable.bits()) & !disable.bits();
    // Every frame reaches the RQ (eth.rs); multicast is filtered in
    // `wanted`, so the list needs no hardware.
    if reset_mcast {
        c.mode.mcast_filter_count = 0;
    } else if !new.is_empty() {
        for (i, &m) in new.iter().enumerate() {
            c.mode.mcast_filter[i] = mac32(m);
        }
        c.mode.mcast_filter_count = new.len() as u32;
    }
    if setting != c.mode.receive_filter_setting || !new.is_empty() {
        trace!(
            "stormnic-mlx5: port {} SNP: receive filters {setting:#x}, {} multicast address(es)",
            c.num(),
            c.mode.mcast_filter_count
        );
    }
    c.mode.receive_filter_setting = setting;
    Status::SUCCESS
}

unsafe extern "efiapi" fn station_address(this: *const SimpleNetworkProtocol, _reset: Boolean, _new: *const MacAddress) -> Status {
    let (c, _tpl) = enter!(this);
    initialized!(c);
    // MacAddressChangeable is FALSE.
    Status::UNSUPPORTED
}

unsafe extern "efiapi" fn statistics(
    this: *const SimpleNetworkProtocol,
    _reset: Boolean,
    _size: *mut usize,
    _table: *mut NetworkStatistics,
) -> Status {
    let (c, _tpl) = enter!(this);
    initialized!(c);
    Status::UNSUPPORTED
}

unsafe extern "efiapi" fn multicast_ip_to_mac(
    this: *const SimpleNetworkProtocol,
    ipv6: Boolean,
    ip: *const IpAddress,
    mac: *mut MacAddress,
) -> Status {
    let (c, _tpl) = enter!(this);
    initialized!(c);
    if ip.is_null() || mac.is_null() {
        return Status::INVALID_PARAMETER;
    }
    // SAFETY: an EFI_IP_ADDRESS is 16 bytes, an IPv4 address in the first 4.
    let a = unsafe { *ip.cast::<[u8; 16]>() };
    // RFC 1112 6.4 and RFC 2464 7.
    let m = if bool::from(ipv6) {
        [0x33, 0x33, a[12], a[13], a[14], a[15]]
    } else {
        if a[0] & 0xf0 != 0xe0 {
            return Status::INVALID_PARAMETER;
        }
        [0x01, 0x00, 0x5e, a[1] & 0x7f, a[2], a[3]]
    };
    unsafe { *mac = mac32(m) };
    Status::SUCCESS
}

unsafe extern "efiapi" fn non_volatile_data(
    this: *const SimpleNetworkProtocol,
    _read: Boolean,
    _offset: usize,
    _size: usize,
    _buffer: *mut c_void,
) -> Status {
    let (c, _tpl) = enter!(this);
    initialized!(c);
    Status::UNSUPPORTED
}

unsafe extern "efiapi" fn get_status(
    this: *const SimpleNetworkProtocol,
    irq: *mut InterruptStatus,
    tx_buf: *mut *mut c_void,
) -> Status {
    let (c, _tpl) = enter!(this);
    initialized!(c);
    c.refresh();
    if !irq.is_null() {
        let mut s = InterruptStatus::empty();
        if c.port().rx_ready() {
            s |= InterruptStatus::RECEIVE;
        }
        if c.port().sent_pending() {
            s |= InterruptStatus::TRANSMIT;
        }
        unsafe { *irq = s };
    }
    if !tx_buf.is_null() {
        let b = c.port().take_sent().map_or(ptr::null_mut(), |t| t as *mut c_void);
        unsafe { *tx_buf = b };
    }
    Status::SUCCESS
}

unsafe extern "efiapi" fn transmit(
    this: *const SimpleNetworkProtocol,
    header_size: usize,
    size: usize,
    buffer: *const c_void,
    src: *const MacAddress,
    dst: *const MacAddress,
    protocol: *const u16,
) -> Status {
    let (c, _tpl) = enter!(this);
    initialized!(c);
    if buffer.is_null() {
        return Status::INVALID_PARAMETER;
    }
    if header_size != 0 && (header_size != 14 || dst.is_null() || protocol.is_null()) {
        return Status::INVALID_PARAMETER;
    }
    if size < 14 {
        return Status::BUFFER_TOO_SMALL;
    }
    if size > eth::MAX_FRAME {
        return Status::INVALID_PARAMETER;
    }
    let mut f = [0u8; eth::MAX_FRAME];
    // SAFETY: the caller passes `size` bytes.
    f[..size].copy_from_slice(unsafe { core::slice::from_raw_parts(buffer.cast::<u8>(), size) });
    if header_size != 0 {
        let from = if src.is_null() { c.port().mac() } else { mac6(unsafe { &*src }) };
        f[0..6].copy_from_slice(&mac6(unsafe { &*dst }));
        f[6..12].copy_from_slice(&from);
        f[12..14].copy_from_slice(&unsafe { *protocol }.to_be_bytes());
    }
    let n = unsafe { &mut *c.nic };
    let pci = unsafe { &mut *n.pci };
    match n.eth.send(pci, &f[..size], buffer as usize) {
        Ok(()) => {
            c.log_frame("tx", &f, size);
            Status::SUCCESS
        }
        Err(Send::Full) => Status::NOT_READY,
        Err(Send::TooLong) => Status::INVALID_PARAMETER,
        Err(Send::Broken) => Status::DEVICE_ERROR,
    }
}

unsafe extern "efiapi" fn receive(
    this: *const SimpleNetworkProtocol,
    header_size: *mut usize,
    size: *mut usize,
    buffer: *mut c_void,
    src: *mut MacAddress,
    dst: *mut MacAddress,
    protocol: *mut u16,
) -> Status {
    let (c, _tpl) = enter!(this);
    initialized!(c);
    if size.is_null() || buffer.is_null() {
        return Status::INVALID_PARAMETER;
    }
    // The CQ, not only what an earlier GetStatus queued (#12).
    c.poll();
    loop {
        let Some((mem, len)) = c.port().peek_rx() else {
            return Status::NOT_READY;
        };
        let mut h = [0u8; 14];
        if len < h.len() {
            c.port().pop_rx();
            continue;
        }
        if len > eth::MAX_FRAME {
            // A jumbo frame on a jumbo network: the SNP says 1500, so no
            // caller's buffer can take it, and BUFFER_TOO_SMALL would stop
            // the queue on it.
            if !core::mem::replace(&mut c.long_logged, true) {
                say!("stormnic-mlx5: port {} rx: {len}-byte frame is over the {} this SNP carries, dropped; no more logged", c.num(), eth::MAX_FRAME);
            }
            c.port().pop_rx();
            continue;
        }
        mem.read(0, &mut h);
        let (d, s): ([u8; 6], [u8; 6]) = (h[0..6].try_into().unwrap(), h[6..12].try_into().unwrap());
        if !c.wanted(d, s, len) {
            c.port().pop_rx();
            continue;
        }
        // SAFETY: the caller's buffer holds `*size` bytes.
        unsafe {
            if *size < len {
                *size = len;
                return Status::BUFFER_TOO_SMALL;
            }
            mem.read(0, core::slice::from_raw_parts_mut(buffer.cast::<u8>(), len));
            *size = len;
            if !header_size.is_null() {
                *header_size = 14;
            }
            if !src.is_null() {
                *src = mac32(s);
            }
            if !dst.is_null() {
                *dst = mac32(d);
            }
            if !protocol.is_null() {
                *protocol = u16::from_be_bytes([h[12], h[13]]);
            }
        }
        c.port().pop_rx();
        c.log_frame("rx", &h, len);
        return Status::SUCCESS;
    }
}

/// `WaitForPacket`'s notify function: signal the event when a frame waits.
unsafe extern "efiapi" fn wait_notify(event: Event, ctx: *mut c_void) {
    // SAFETY: the context is the child, which closes this event before it goes.
    let c = unsafe { &mut *ctx.cast::<Child>() };
    if unsafe { (*c.nic).dead } {
        return;
    }
    if c.mode.state != NetworkState::INITIALIZED {
        return;
    }
    // Notify functions run at TPL_CALLBACK, which every SNP call holds, so
    // this never runs inside one (#12: the CQ, not only what GetStatus
    // queued). The receive CQ only: the EQ's page requests and port changes
    // are firmware commands, which wait for Receive or GetStatus (#19).
    c.port().poll_receive();
    if c.port().rx_ready() {
        let _ = unsafe { (bs().signal_event)(event) };
    }
}
