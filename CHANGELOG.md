# Changelog

## [Unreleased]

### 2026-11-20 (#18)
- **fix:** `give_pages` no longer frees the pages it offered when MANAGE_PAGES times out or its doorbell write fails: the firmware may hold them or take them later, so they are kept as lent and freed only when returned (finding 21b5b4e705e4). Only a delivery/command status or an unsent command frees them. `cmd::Error` moved to `src/cmderr.rs` with `refused()`, host-tested (`test/cmderr.rs`).

### 2026-11-18 (docs refresh)
- **docs:** README/CLAUDE.md brought to the code since 2026-11-11: forge1 also has a card, #15 (unbounded quiesce wait) and #7 (MaxPacketSize 1500 until jumbo) named, stale pin note and report-section wording fixed. No doc promises what the code lacks beyond the open issues #7/#9/#15.

### 2026-11-15 (#12, verified)
- **docs:** #12 closed: the receive fix (9d86045, `Receive`/`WaitForPacket` drain the CQ) took DHCP leases on metal,
  serial 14 on C2NR0Q2 and forge1 :81 (stormbootx#216); work plan marks it done.

### 2026-11-15 (#1)
- **docs:** `docs/spec/metal.md`: the HW-CHECKs the first metal runs settle (serial 12/14 on C2NR0Q2 and forge1,
  ConnectX-4 Lx); README's hardware table names the runs.
- **docs:** Work plan: #1 done (bind, link, TX/RX on metal, pinned); stormbootx#73 told.
- **chore:** `TESTED` = 15b3:1015, so its bind no longer traces "has not been run with this driver yet".

### 2026-11-15 (#13)
- **docs:** Work plan: #13 pinned in stormbootx (bdd4375); forge1 :80 links since the owner moved its cable (switch
  ports at fault). The diagnosis lines wait on a verbose `stormbootx-mlx5test` boot of forge1, where they are traced.

### 2026-11-14 (#8, verified)
- **docs:** fec.md HW-CHECK 5 answered from metal (32ddb3f): `fec_mode_active` 0x4 on a 25G RS link; every linked
  ConnectX-4 Lx port on C2NR0Q2 and forge1 ran the standard RS. #8 closed; forge1 :80's missing link is #13.

### 2026-11-14 (#13)
- **feat:** Under a port with no link after bring-up, the card's own account of why, one line per register group:
  PDDR troubleshooting (`monitor_opcode` named per the adb, and the firmware's status message), operation info
  (`phy_mngr_fsm_state`, `eth_an_fsm_state`, `neg_mode_active`, link-enabled masks, `loopback_mode`, FEC
  request/active), last link-down info (blame, intent, reason opcodes), module monitors (V, C, rx/tx power per lane
  in both of the page's units, bias), latched module flags (rx_los, tx_fault, CDR LOL, power alarms), PTYS
  cap/admin/oper with `an_status`/`an_disable_*`, PAOS, PPLM raw and PMLP's lane map. A linked port's line stays
  one line; its diagnosis goes to the trace. Layouts from mstflint's adb (docs/spec/linkdiag.md). For forge1 84:00.0.

### 2026-11-14 (#12)
- **fix:** `SNP.Receive` and `WaitForPacket`'s notify drain the EQ and CQs themselves; `Initialize`'s stale-frame
  drop covers the CQ too. Before, only `GetStatus` moved completions into the receive queue, and stormbootx calls it
  only before a transmit, so each DHCP OFFER reached smoltcp after the next DISCOVER (every 2 s) had changed the
  xid, and was dropped: no lease on C2NR0Q2 or forge1. g16's microdns logged every DISCOVER and offered each at once.

### 2026-11-14 (#8, metal)
- **fix:** `fec_mode_active` is decoded as the bitmask the adb says it is (bit n = mode n, bit 2 RS), not as the
  enum's index. 09b0615 on C2NR0Q2's 25G ConnectX-4 Lx ports printed `FEC FAILED: … but the link runs other`:
  a value no 25G index explains, and bit 2 is RS.
- **feat:** The port line's FEC FAILED for a running mode carries the raw PPLM values (`PPLM 25g cap … admin …`,
  `fec_mode_active …`), read after the link wait; PAOS admin/oper is traced after the down/up.

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
