# Why a port has no link: the card's own account (#13)

forge1's ConnectX-4 Lx 84:00.0 (`…:a2:80`) has no link with RS set and its
QSFPTEK QT-SFP28-SR read, under this driver and under FlexBoot alike; :81, the
same card and optic model, is up (#13). Owner, 2026-11-14: "get the driver to
tell us why :80 is down". So under a port with no link once bring-up is done
(FEC written, port bounced, link wait over), the driver prints what the card
says, one line per register group (`Eth::diagnosis`, `layout::diag`). A linked
port stays one line; its diagnosis goes to the trace (verbose), to compare.

## Sources

mstflint's adb, as for PPLM/PDDR (docs/spec/fec.md, Sources; owner on #8:
"take it mellanox docs"). Register IDs from the access-register union:
PDDR 0x5031, PMLP 0x5002, PTYS 0x5004, PAOS 0x5006, PPLM 0x5023. Bit numbering
as fec.md 1. PDDR is read with `page_select` (04h 7:0) and, for the module
page, `module_info_ext` (04h 30:29: 0 dBm, 1 uW); page data starts at 08h,
offsets below are within the page.

| Line | Register / page | adb node | Fields |
|---|---|---|---|
| PDDR troubleshooting | PDDR page 1 | `pddr_troubleshooting_page_ext`, `pddr_monitor_opcode_ext` | `group_opcode` 00h 15:0 (0, monitor opcodes); `monitor_opcode` 04h 15:0, named per the adb's list; `status_message` 0Ch, NUL-terminated ASCII, quoted as is |
| PDDR operation | page 0 | `pddr_operation_info_page_ext` | `neg_mode_active` 00h 19:16, `proto_active` 00h 23:20, `eth_an_fsm_state` 04h 23:16, `phy_mngr_fsm_state` 04h 31:24, `phy_manager_link_enabled` 08h, `core_to_phy_link_enabled` 0Ch, `cable_proto_cap` 10h, `link_active` 14h, `loopback_mode` 18h 11:0, `fec_mode_request` 1Ch 15:0, `fec_mode_active` 1Ch 31:16 |
| PDDR last link down | page 6 | `pddr_link_down_info_page_ext` | `down_blame` 00h 3:0, `down_intent` 00h 7:4, `local_reason_opcode` 04h 7:0, `recovery_entry_reason` 04h 23:16, `remote_reason_opcode` 08h 7:0, `e2e_reason_opcode` 0Ch 7:0, `pcs_phy_state_latched` 34h |
| PDDR module | page 3, read twice (dBm, uW) | `pddr_module_info_ext` | `voltage` 4Ch 15:0 (100 uV), `temperature` 4Ch 31:16 (s16, 1/256 C), `rx_power_laneN` 50h.., `tx_power_laneN` 60h.., `tx_bias_laneN` 70h.. (2 uA), lane 2k at 31:16 and 2k+1 at 15:0; `monitor_cap_mask` 9Ch 15:8; `module_st` 94h 31:29 (CMIS only) |
| PDDR module flags | page 9 | `module_latched_flag_info_ext` | `rx_los_cap` 00h 0, `vcc_flags` 00h 27:24, `temp_flags` 00h 31:28, `tx_cdr_lol` 04h 15:8, `tx_los` 04h 23:16, `tx_fault` 04h 31:24, `tx_power_lo_al` 08h 23:16, `tx_bias_lo_al` 0Ch 23:16, `rx_cdr_lol` 10h 23:16, `rx_los` 10h 31:24, `rx_power_lo_al`/`hi_al` 14h 23:16 / 31:24 (bit per lane) |
| PTYS | PTYS (64 bytes) | `ptys_reg_ext` | `an_disable_cap` 00h 29, `an_disable_admin` 00h 30, `an_status` 04h 31:28, `ext_eth_proto_capability` 08h, `eth_proto_capability` 0Ch, `ext_eth_proto_admin` 14h, `eth_proto_admin` 18h, `ext_eth_proto_oper` 20h, `eth_proto_oper` 24h, `connector_type` 2Ch 3:0 |
| PAOS; PPLM raw | PAOS, PPLM | `paos_reg_ext`, `pplm_reg_ext` | `admin_status` 00h 11:8, `oper_status` 00h 3:0; PPLM dwords 0Ch, 10h, 14h raw |
| PMLP | PMLP (0x40) | `pmlp_reg_ext`, `lane_2_module_mapping_ext` | `width` 00h 7:0 (0 = unmapped: never links); lane i at 04h+4i: `module` 7:0, `tx_lane` 19:16, `rx_lane` 27:24 |

A register or page the firmware refuses prints `…: not answered`; a read too
short for its layout prints `…: short read`. Nothing here is written: every
read is an ACCESS_REG query, so the diagnosis cannot change the port.

## Reading it

- `monitor_opcode` and the firmware's message are the card's own verdict:
  1 PAOS closed it (our bring-up); 2–4/38/39/69 AN failure; 5–8 link training;
  9–13 logical mismatch with the partner; 14 remote fault (the far end); 15/17/42…
  bad signal integrity (optic, fibre, far end's optic); 16/24–32 cable compliance
  mismatch; 57 signal not detected / 60 no partner for long (nothing lit on the
  fibre); 1024+ module/management issues.
- rx power near zero (or the rx_los flag) with a sane tx power: the light does
  not arrive (fibre, patch, far end down). tx_fault or a zero tx power: our optic.
- PTYS `an_disable_admin 0` with only 25GBASE-SR admin: SR has no AN; whether
  the firmware holds the port in AN is `eth_an_fsm_state` / `an_status`.
- PMLP width 0, or PAOS admin not up: our bring-up.

## HW-CHECKs (forge1 boot)

1. The module page's power fields: which resolution? The line gives each
   lane in the dBm read (s16) and the uW read; :81 (up, light arriving) gives
   the pair to tell.
2. Do pages 0, 1, 6 and 9 answer on ConnectX-4 Lx firmware (14.x)? A page
   that does not says `not answered`.
3. Is `status_message` filled on this firmware?
