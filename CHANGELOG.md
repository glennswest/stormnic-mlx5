# Changelog

## [Unreleased]

### 2026-11-14 (#1)
- **docs:** Work plan: stormnic-mlx5 09b0615 asked for as the stormbootx pin (stormbootx#202); #1 waits on that pin and the canary boot.

### 2026-11-14 (#8)
- **feat:** The standard FEC at bind: PPLM (0x5023) is read, each speed's `fec_override_admin` set to its IEEE
  standard where `fec_override_cap` offers it (10G/40G none, 25G/50G/100G RS, PAM4 RS-544), written and read back;
  the port is taken down and up (PAOS) if it was already up. The port line gives the module (PDDR 0x5031 page 3),
  `fec rs (standard for …, running rs)` from `fec_mode_active`, or `FEC FAILED: …` (also an alarm).
- **docs:** `docs/spec/fec.md`: PPLM and PDDR layouts from Mellanox's PRM register database in mstflint (BSD
  terms; owner: "take it mellanox docs"), the adb bit numbering checked against PRM Rev 0.40. NOTICE and the
  crate's source rule say so.

### 2026-11-13 (#8)
- **feat:** One console line per port: driver, PCI location, MAC, media and speed from PTYS (`eth_proto_oper` when
  up, else the fastest of `eth_proto_capability`), FEC and link. `layout::PROTOS` names every PRM Table 108 protocol
  with its speed and IEEE 802.3 standard FEC (25G/50G/100G RS, 1G/10G/40G none), host-tested. The FEC is not set yet
  (PPLM is not in PRM Rev 0.40; owner decision on #8), so the line says `FEC FAILED: standard … not set: …`. Speeds
  are now lower case (`25g`).

### 2026-11-13
- **docs:** Work plan: #1 is P0 (owner, 2026-11-14). The driver must set the standard FEC per media/speed at bind and
  report driver, speed, FEC and link per port, with no FEC config from stormbootx (replaces #8's `StormnicFec`). The
  public PRM Rev 0.40 has no PPLM (FEC) register, so that waits on an owner decision about the layout's source (#1, #8).
  The first metal run comes by canary promotion of the rustnic medium to C2NR0Q2, then forge1.

### 2026-11-11
- **docs:** Refreshed from the code since 2026-11-04: no source change since 66de545, so README, `docs/spec/` and
  CLAUDE.md already match. No promise without code found, so no issue filed. #1 still waits on the Dell's boot.

### 2026-11-04
- **docs:** Work plan: stormbootx#131 is done (v0.37.0, pin 66de545, test medium `stormbootx-mlx5test`), and #1 now
  waits on the Dell's boot of that medium (#1).
- **docs:** Refreshed from the code since 2026-10-14: README, `docs/spec/datapath.md` and CLAUDE.md already match the
  10 KiB receive buffers, software drop over 1518, PMTU trace and the FlexBoot decline; README line wrap fixed. No
  promise without code found, so no issue filed.
- **fix:** Receive buffers are 10 KiB, and a frame over 1518 bytes is dropped in software and logged once. The Dell's
  network is jumbo and the vport promiscuous: a frame over the 2 KiB buffer was a Local_Length error, which takes the RQ
  to ERR for good. PMTU goes to the trace for the first hardware run (#1).
- **fix:** The image is byte-reproducible: link with `/DEBUG:NONE`, as stormbootx's nic-drivers build does (#6).
- **docs:** The R230's ConnectX-4 Lx carries FlexBoot, so the driver binds there only with stormbootx#131 or
  stormcentral#552's BIOS job (#2).

### 2026-10-10
- **feat:** UEFI SNP driver for mlx5 (#1): driver binding for 15b3:1013/1015/1017/1019/101b/101d/101f (PFs, checked
  against pci.ids 2026.10.09), command queue (PRM 7.14), HCA bring-up and teardown (PRM 8.2/8.3), one SQ/RQ pair
  with a direct TIR and a match-all NIC RX flow entry, EFI_SIMPLE_NETWORK_PROTOCOL on a child handle.
- **docs:** CLAUDE.md work plan; `docs/spec/` notes from the public ConnectX-4 PRM Rev 0.40.

### 2026-10-07
- **chore:** repository created (owner: low priority, paused)
