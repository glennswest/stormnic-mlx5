# CLAUDE.md — stormnic-mlx5

A `no_std` UEFI driver giving firmware an `EFI_SIMPLE_NETWORK_PROTOCOL` for
Mellanox ConnectX-4 / 4 Lx / 5 / 6 Ethernet (the Linux `mlx5` family). It is
loaded by stormbootx from `\stormboot\drivers` on its rustnic media, on
machines whose firmware has no UEFI driver for the NIC. The first target is
the Dell R230 test machine (C2NR0Q2), which carries a ConnectX-4 Lx
(MT27710 family, 15b3:1015). **Its card firmware carries FlexBoot**, Mellanox's
UEFI driver, which holds the NIC, so this driver declines there (#2) until
stormbootx#131 (`prefer_media_drivers = mlx5`, take-over) or stormcentral#552's
BIOS job (the card's option ROM off) removes FlexBoot. stormbootx v0.37.0 has
the former, on its test medium only so far. Read README.md first.

Read the cross-project rules in `../CLAUDE.md` first. In particular, **build
with `sc-build` after pushing, never on this VM and never as root**, and
scratch files go in `tmp/`.

## Rules for this crate

- **Written from the public PRM only:** *Mellanox Adapters Programmer's
  Reference Manual (PRM) Supporting ConnectX-4 and ConnectX-4 Lx*, Rev 0.40,
  MLNX-15-4845 (2016), published by NVIDIA at
  `https://network.nvidia.com/files/doc-2020/ethernet-adapters-programming-manual.pdf`.
  Cite PRM sections/tables in code (`PRM 7.14.1`, `PRM Table 95`). Never
  translated from Linux mlx5 or iPXE golan/flexboot (GPL); the crate is MIT.
  `NOTICE` acknowledges the PRM. Download it into `tmp/prm/` to read it (it
  is not committed). Rev 0.40 has no opcode for anything after chapter 12;
  anything it does not settle is a hardware check (docs/spec/connectx4.md §HW).
  Registers Rev 0.40 lacks (PPLM, PDDR) come from **Mellanox's own PRM register
  database in mstflint** (`tools_layouts/adb/prm/hca/ext/register_access_table.adb`,
  GPL-2.0 or OpenIB.org BSD, taken under BSD; owner on #8, 2026-11-14: "take it
  mellanox docs, no exception needed"). Cite the adb node (`pplm_reg_ext`) as a PRM
  section is cited; see docs/spec/fec.md.
- It is a *driver*: an EFI boot-service driver (subsystem 11) that installs
  `EFI_DRIVER_BINDING_PROTOCOL`. stormbootx loads it, then connects controllers.
- A platform's own driver must win: Supported tries a `BY_DRIVER` open of the
  NIC's PciIo and declines if anyone holds it.
- Console (`src/console.rs`, as stormnic-mlx4#16 / stormnic-ixgbe#22):
  `trace!` for the bring-up trace (printed only when verbose: EFI variable
  `StormnicVerbose` or `--features verbose`; else kept in a 16-line ring that
  a failure replays), `say!` for the one line per NIC and every warning,
  `alarm!` for failures. Never `uefi::println!` directly.

## Build

```bash
sc-build 'cargo build --locked --release --target x86_64-unknown-uefi && scripts/pe-subsystem.sh "${CARGO_TARGET_DIR:-target}"/x86_64-unknown-uefi/release/stormnic-mlx5.efi && scripts/test-host.sh'
```

`Cargo.lock` is committed; keep `--locked`. Zero warnings: `[lints.rust]
warnings = "deny"` and clippy `-D warnings` (`scripts/check.sh`).

## How it ships

No golden of its own. stormbootx's `scripts/build-nic-drivers.sh` builds it
from a pinned `STORMNIC_MLX5_REF` (stormbootx#73), and the
`stormbootx-rustnic` media carries `stormnic-mlx5.efi`.

## Test

Host tests: `scripts/test-host.sh` (UEFI-independent decoding: mailbox
signatures, PAS layout, CQE/opcode decoding, PTYS speeds).

Hardware: only the Dell (C2NR0Q2) has the card; no pve VM can present it.
A run is booting the rustnic medium (or a stick) on the Dell — the master's
job, coordinated on the issue (the Dell is held for shelf work, but a
stormbootx boot test does not touch the shelf).

## Version

`Cargo.toml` → `package.version`. Current: `v0.1.0` (unreleased).

## Work plan

#1 — bind, link, transmit/receive; pinned in stormbootx's nic-drivers (P1, owner 2026-10-10).

- [x] Crate scaffold from stormnic-mlx4's shape (binding, PciIo, DMA, console/trace, verbose switch,
      reproducible build), PCI IDs checked against pci.ids.
- [x] Command interface (PRM 7.14, 8.2): init segment, cmdq, mailboxes + signatures, polling.
- [x] Bring-up: ENABLE_HCA, ISSI, pages (QUERY/MANAGE_PAGES), QUERY_HCA_CAP, INIT_HCA, teardown. SET_HCA_CAP
      is not needed: the defaults are used as read.
- [x] Objects: UAR, PD, transport domain, reserved LKey, EQ (pages/port events), CQ×2, TIS, SQ, RQ, TIR,
      NIC RX flow table + group + match-all entry → TIR, vport context (MAC, promisc), PAOS/PTYS link.
- [x] SNP on a child handle: Transmit/Receive/GetStatus, ExitBootServices teardown.
- [x] Spec notes `docs/spec/` (cmd.md, datapath.md, steering.md: PRM tables per step, HW-CHECKs).
- [x] Docs say the R230 has FlexBoot (#2).
- [x] The g16 network is jumbo (switch 9216): a frame over the RQ's scatter size is a Local_Length error that
      takes the RQ to ERR for good (PRM 7.12.7), so RX buffers hold any frame the port can pass; oversize frames
      are dropped in software. PMTU traced for the first run.
- [x] sc-build passes (subsystem 11, no warnings, host tests, reproducible image): 66de545.
- [x] stormbootx build with this driver, and a way past FlexBoot: stormbootx#131 done in v0.37.0 (pin 66de545,
      `prefer_media_drivers = virtio,mlx5` on the verbose test medium `stormbootx-mlx5test`). No source change here
      since 66de545, so the pin is current.
- [ ] **P0 (owner, 2026-11-14).** The first metal run comes by canary promotion (the master's): the rustnic medium with
      this driver and its take-over goes canary-only to C2NR0Q2's MAC, then forge1's. Its console is posted on #1.
- [x] **FEC at bind (owner, 2026-11-14; #8):** the standard FEC per media/speed (25G/50G/100G RS, 10G/40G none, PAM4
      RS-544) via PPLM (read-modify-write, read back, PAOS down/up if it was up), module from PDDR, one line per port
      (driver, BDF, MAC, PTYS media/speed, module, FEC, link; `FEC FAILED: …` loud). Layouts from mstflint's adb
      (owner: Mellanox docs), docs/spec/fec.md. Its HW-CHECKs come with the first metal run.
  - [x] Pin the commit in stormbootx (`STORMNIC_MLX5_REF`): 09b0615 asked for on stormbootx#202 (2026-11-14; sc-build passed).
        stormbootx still pins 66de545; #1 waits on stormbootx#202 (pin + rustnic medium), then the master's canary boot.
- [ ] With the boot's result: answer the HW-CHECK lists and fill in `TESTED` and the README table.
- [ ] Say so on stormbootx#73 once it passes, so the driver joins the default `STORMNIC_DRIVERS` and the rustnic media.

After #1: jumbo frames (#7), for stormbootx#165.
