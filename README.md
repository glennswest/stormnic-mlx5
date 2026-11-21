# stormnic-mlx5

A UEFI driver, in Rust (`no_std`), that gives firmware an
`EFI_SIMPLE_NETWORK_PROTOCOL` for **Mellanox mlx5 Ethernet: ConnectX-4,
ConnectX-4 Lx, ConnectX-5, ConnectX-6**. stormbootx loads it from
`\stormboot\drivers` on its rustnic media so it can boot over these NICs
without iPXE or a vendor option ROM. It is a sibling of stormnic-ixgbe,
stormnic-mlx4 and stormnic-virtio, built and pinned the same way (stormbootx
`scripts/build-nic-drivers.sh`).

## Why it exists

stormbootx runs its own TCP/IP stack (smoltcp) on each NIC's SNP. Cheap
ConnectX-4 Lx cards often carry no FlexBoot UEFI driver, so the firmware
offers no SNP for them. The first target is the Dell R230 test machine
(C2NR0Q2), whose PCIe slot holds a ConnectX-4 Lx (MT27710 family,
15b3:1015) (#1): with this driver it boots stormbootx from a USB stick, as
the X9 blades do, with no iDRAC virtual media and no option ROM.

**The R230's card does carry FlexBoot today**, which holds the NIC, and this
driver leaves a NIC another driver holds alone (#2). So it binds there only
once FlexBoot is out of the way: stormbootx#131 (`prefer_media_drivers = mlx5`
disconnects it, on media that ask for that) or stormcentral#552's BIOS job
(the card's option ROM off). A function FlexBoot left initialized is reset
before ENABLE_HCA (`docs/spec/cmd.md` HW-CHECK 6).

## Hardware

| PCI ID | Family | Run on |
|---|---|---|
| 15b3:1013 | ConnectX-4 (MT27700) | not yet |
| 15b3:1015 | ConnectX-4 Lx (MT27710) | C2NR0Q2 (R230) and forge1 (R6415), 25GBASE-SR, both functions: bind, RS FEC, link, DHCP, NVMe/TCP attach, handoff ([`docs/spec/metal.md`](docs/spec/metal.md)) |
| 15b3:1017 | ConnectX-5 (MT27800) | not yet |
| 15b3:1019 | ConnectX-5 Ex (MT28800) | not yet |
| 15b3:101b | ConnectX-6 (MT28908) | not yet |
| 15b3:101d | ConnectX-6 Dx (MT2892) | not yet |
| 15b3:101f | ConnectX-6 Lx (MT2894) | not yet |

Physical functions only, checked against pci.ids (2026.10.09). Each port of
an mlx5 card is its own PCI function, so each gets its own SNP.

**Written from the public PRM only:** *Mellanox Adapters Programmer's
Reference Manual (PRM) Supporting ConnectX-4 and ConnectX-4 Lx*, Rev 0.40
(MLNX-15-4845), which NVIDIA publishes. `docs/spec/` condenses what the
driver uses, with PRM section and table numbers and the points the PRM leaves
open (HW-CHECK):

- [`docs/spec/cmd.md`](docs/spec/cmd.md): initialization segment, command
  queue and mailboxes, bring-up/teardown, capabilities, port registers.
- [`docs/spec/datapath.md`](docs/spec/datapath.md): UAR doorbells, EQ, CQ,
  SQ, RQ, TIS, TIR, WQE and CQE formats.
- [`docs/spec/steering.md`](docs/spec/steering.md): NIC RX flow tables and
  the NIC vport context.
- [`docs/spec/metal.md`](docs/spec/metal.md): which HW-CHECKs the first runs
  on a ConnectX-4 Lx settle.

Nothing is taken from the Linux mlx5 driver or iPXE's golan/flexboot (GPL);
the crate is MIT ([`NOTICE`](NOTICE)).

## How it works

- `src/cmd.rs`: one command-queue entry, polled, with mailbox chains and
  signatures (PRM 7.14). No allocation after setup, so ExitBootServices can
  still send commands.
- `src/hca.rs`: PRM 8.2 bring-up: the queue address, `initializing`,
  ENABLE_HCA, QUERY/SET_ISSI, boot and init pages (QUERY/MANAGE_PAGES),
  QUERY_HCA_CAP (general, Ethernet, NIC flow table), INIT_HCA,
  SET_DRIVER_VERSION, ALLOC_UAR/PD/TRANSPORT_DOMAIN and the reserved LKey
  (QUERY_SPECIAL_CONTEXTS, physical addressing, no MKey). PRM 8.3 teardown:
  TEARDOWN_HCA, every page reclaimed, DISABLE_HCA.
- `src/eth.rs`: the vport (admin up, permanent MAC, own MAC in the unicast
  list, promiscuous), one EQ for page requests, port changes and CQ errors,
  a send CQ/SQ behind a TIS, a receive CQ/RQ (256 × 10 KiB, so a jumbo frame cannot put the RQ in
  error) behind a direct TIR with self-loopback off, and a NIC RX flow table whose match-all entry
  forwards to the TIR. Link and speed from PAOS/PTYS. All polled.
- `src/snp.rs`: the SNP on a child handle with a MAC device path.

## Build

`x86_64-unknown-uefi`, built on a build VM with `sc-build` after pushing:

```bash
sc-build scripts/check.sh
```

That runs the locked release build, clippy with warnings as errors, the PE
subsystem check (11, boot-service driver), a `--features verbose` build and
the host tests. `Cargo.lock` is committed; builds use `--locked`. The image
is meant to be byte-reproducible like stormnic-mlx4's (`scripts/rustc-remap.sh`,
`/Brepro`; `sc-build scripts/repro-check.sh` checks it).

## Interfaces and configuration

- **Installs:** `EFI_DRIVER_BINDING_PROTOCOL` on its image handle (version
  0x10). For each started function, a child handle with
  `EFI_DEVICE_PATH_PROTOCOL` (PCI path + MAC node, IfType 1) and
  `EFI_SIMPLE_NETWORK_PROTOCOL` (revision 0x00010000), installed and
  uninstalled together (Install/UninstallMultipleProtocolInterfaces). The
  child is all or nothing (#19): a failed `BY_CHILD_CONTROLLER` open fails
  `Start`, and a child the firmware will not let go of (a consumer still has
  its SNP open) stays published and allocated, NIC with it, until a later
  `Stop` withdraws it.
- **SNP:** MaxPacketSize 1500 (jumbo, 9000, is #7), MediaHeaderSize 14, receive filters unicast,
  broadcast and up to 16 multicast addresses, applied in software (the
  hardware delivers every frame the vport takes), a frame over 1518 bytes is
  dropped and logged once, MAC not changeable,
  `MediaPresent` from port-change events and PAOS, `WaitForPacket`,
  `MCastIpToMac`. VLAN tags are kept. Own frames are not looped back (TIR
  self-loopback off); one that is, is dropped and logged once.
  `Receive` and `WaitForPacket` drain the receive CQ themselves, so a frame
  is handed up as soon as it lands, whether or not the caller calls
  `GetStatus` (#12). `WaitForPacket`'s notify function reads the receive CQ
  only; page requests and port changes on the EQ (firmware commands) are
  handled by `Receive` and `GetStatus` (#19).
  `Statistics`, `StationAddress` and `NvData` return `UNSUPPORTED`.
- **Consumes:** `EFI_PCI_IO_PROTOCOL` (`BY_DRIVER`, and `BY_CHILD_CONTROLLER`
  per child): config space, BAR 0 reads/writes (32- and 64-bit), DMA common
  buffers below 4 GiB, attributes. While started it keeps memory space, bus
  master and dual-address-cycle on; `Stop` puts the original attributes back.
- **ExitBootServices:** TEARDOWN_HCA (panic close, which stops bus mastering)
  and DISABLE_HCA, then bus master off. Nothing is freed or printed. The
  command wait is not yet bounded for this path (up to 60 s per command; #15).
- **A platform driver wins:** a NIC with an SNP already, or whose PCI I/O is
  held `BY_DRIVER`, is left alone.
- **Configuration:** one switch, console output only: the EFI variable
  `StormnicVerbose` under vendor GUID `ce1479a2-eab9-4176-b0ad-c909ea5b8e0b`
  (shared by every stormnic driver), read once at the entry point; a first
  byte other than 0 prints the bring-up trace. `--features verbose` does the
  same at build time.

## Console output

Quiet by default: one line per port (#8): driver and version, PCI location,
ID and name, MAC, media and speed, module, FEC, link,

```
stormnic-mlx5 0.1.0: 0000:02:00.0 15b3:1015 ConnectX-4 Lx port 1: MAC xx:xx:xx:xx:xx:xx, 25GBASE-SR 25g, SFP28/SFP+ optical <vendor> <part>, fec rs (standard for 25GBASE-SR, running rs), link up, SNP installed
```

The media and speed come from PTYS (PRM 11.1.2, Table 108): `eth_proto_oper`
when the link is up, else the fastest of `eth_proto_capability`, named
together (`25GBASE-SR/KR/CR 25g capable`). The module is PDDR's module page.
**FEC:** at bind, before the port trains, the driver sets each speed's IEEE
802.3 standard FEC in PPLM (25G, 50G and 100G RS(528); 10G and 40G none;
PAM4 RS(544)) where the firmware offers it, reads it back, and takes the port
down and up if it was already up. No configuration chooses it. If it cannot
(`PPLM … refused`, the firmware does not offer the standard, an admin that
differs, or a link running another FEC), the line says `FEC FAILED: …` and an
alarm replays the trace. It is judged after the link wait at the speed the
port runs (with no link, the fastest it can), so a 40G-capable port running
25G is checked on the 25G field (#22). A diagnostic register that does not
answer (a PDDR page, PMTU) says `not answered` on its line and raises no
alarm. Details: `docs/spec/fec.md`.

**No link (#13):** under a port that has no link once bring-up is done, the
card's own account of why, one indented line per register group: PDDR's
troubleshooting page (`monitor_opcode` named, and the firmware's own message),
operation page (phy manager and AN state machines, negotiation mode,
loopback), last link-down page, the module's monitors (voltage, temperature,
rx/tx power, bias per lane) and latched flags (rx_los, tx_fault, …), PTYS
capability/admin/oper with `an_status` and `an_disable_*`, PAOS, PPLM raw and
PMLP's lane map:

```
    port 1: PDDR troubleshooting: monitor_opcode 57 (signal not detected), firmware says "…"
    port 1: PTYS: eth_proto_capability 0x38000000 (25GBASE-SR,25GBASE-KR,25GBASE-CR), eth_proto_admin …, an_status 4 (AN not performed, link down), …
```

A linked port stays one line; the same lines go to the trace, to compare
with a port that is down. Details: `docs/spec/linkdiag.md`.

Besides these, warnings and failures print; a failure first replays the last 16 trace lines.
Verbose prints every command (`    INIT_HCA (0x0): ok`), the capabilities, the
pages lent, the vport state and the data-path object numbers.

## Tests

`scripts/test-host.sh` runs the host tests of the UEFI-free code
(`test/layout.rs`: bit fields, command header, entry and mailbox signatures,
send WQE construction, CQE/EQE ownership, PTYS speeds and media, the standard FEC per media, PPLM's
read-modify-write, PDDR's module page, the per-port report line, the no-link diagnosis lines; `test/trace.rs`: the
replay ring).

On metal: only the Dell (C2NR0Q2) and forge1 (R6415) have an mlx5 card, and no
pve VM can present one. A run boots the rustnic medium (or a stick) on one of
them with this driver, verbose; the master does the boot. The HW-CHECK lists in `docs/spec/` say
what that first run must answer.

## Licence

MIT (see LICENSE). NOTICE acknowledges the PRM.
