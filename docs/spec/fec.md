# FEC, module and the per-port line (#8)

The owner's rule (2026-11-14, #1/#8): the driver sets the IEEE 802.3 standard
FEC for the port's media and speed itself, with no configuration and nothing
from stormbootx, and reports driver, speed, FEC and link per port. Owner on
#8: "take it mellanox docs, no exception needed".

## Sources

- PRM Rev 0.40 (MLNX-15-4845): PTYS (11.1.2, Tables 107/108), PAOS
  (11.1.3, Tables 109/110). It has no PPLM and no PDDR.
- **Mellanox's PRM register database as published in mstflint**
  (github.com/Mellanox/mstflint, `tools_layouts/adb/prm/hca/ext/register_access_table.adb`,
  `<info source_doc_name="NIC_PRM_internal" source_doc_version="0.0.65j"/>`),
  Mellanox Technologies, under GPL-2.0 **or the OpenIB.org BSD licence** (its
  `LICENSE`); this crate takes it under the BSD terms (NOTICE). It is the PRM's
  own register tables in machine-readable form: register IDs (`selected_by`
  enum of the access-register union: `PPLM=0x5023`, `PDDR=0x5031`, and PTYS
  0x5004 / PAOS 0x5006 as Rev 0.40), field offsets, sizes and enum values.
  Nodes used: `pplm_reg_ext`, `pddr_reg_ext`, `pddr_module_info_ext`.
  Nothing comes from Linux mlx5 or iPXE. Downloaded to `tmp/mstflint/` to read.
- NVIDIA's WinOF-2 user manual ("Management Utilities") describes PPLM only
  in general terms (a `cap` and an `admin` field per speed), which agrees.

### 1. The adb's bit numbering, checked

An adb `offset="0xB.b" size="0x0.n"` is byte B (a big-endian dword) and bits
b+n-1..b counted from the dword's LSB, the PRM's own numbering: adb PAOS
`oper_status` 0x0.0/4 = PRM 3:0, `admin_status` 0x0.8/4 = 11:8,
`local_port` 0x0.16/8 = 23:16; PTYS `eth_proto_capability` 0xC, `eth_proto_oper`
0x24 (Tables 108/110). So `layout::put/get(b, B, b+n-1, b)` takes the adb as is.

## 2. PPLM — Port Phy Link Mode, 0x5023, 0x80 bytes (`pplm_reg_ext`)

| Offset | Bits | Field | Values |
|---|---|---|---|
| 00h | 23:16 | local_port | index |
| 0Ch | 23:0 | fec_mode_active (RO) | 0 none, 1 Firecode, 2 RS(528), 3 LL RS, 7 RS(544) KP4, 9 consortium LL 50G RS, … |
| 10h | 3:0 / 7:4 / 11:8 / 15:12 | fec_override_cap_10g_40g / _25g / _50g / _100g (RO) | bit0 no FEC, bit1 Firecode, bit2 RS (100G: bit1 reserved) |
| 14h | 3:0 / 7:4 / 11:8 / 15:12 | fec_override_admin_… (RW) | one-hot as the cap; 0 = auto (firmware's choice) |
| 14h | 19:16 | fec_override_admin_56g | left as read |
| 18h | 15:0 / 31:16 | fec_override_cap_200g_4x / _400g_8x | 16-bit masks |
| 1Ch | 15:0 / 31:16 | fec_override_cap_50g_1x / _100g_2x | |
| 20h | 15:0 / 31:16 | fec_override_admin_200g_4x / _400g_8x | bit7 RS(544,514), bit9 RS(272,257+1) |
| 24h | 15:0 / 31:16 | fec_override_admin_50g_1x / _100g_2x | same |

adb: "25g_fec_override_admin and 50g_fec_override_admin must be set with the
same value" (both RS here).

**What the driver does** (`eth::set_fec`, `layout::pplm`): before the port
trains, read PPLM; for every speed whose cap offers its standard (10G/40G
no FEC, 25G/50G/100G RS, PAM4 RS(544)), set that admin, keeping every other
bit (read-modify-write); write it with local_port; read it back. The port's own
speed (the fastest PTYS says it runs, or can) decides the line: its admin must
read back as the standard, else `FEC FAILED: …` (also `alarm!`, with the
trace). A speed whose cap does not offer its standard is left as it is
(auto). PPLM, like PTYS's admin fields (PRM 11.1.2 note), takes effect at
link training: if PAOS says the port was already administratively up and PPLM
changed, the driver writes PAOS admin down (2) and up (1), each with `ase`.
After the link wait, `fec_mode_active` is read when the link is up and a value
other than the standard is `FEC FAILED: … but the link runs …`.

## 3. PDDR — Port Diagnostics Database, 0x5031, 0x100 bytes

`pddr_reg_ext`: local_port 00h 23:16, `page_select` 04h 7:0 (index; 3 = Module
Info page), `page_data` from 08h. `pddr_module_info_ext`, offsets within the page:

| Offset | Bits | Field |
|---|---|---|
| 04h | 31:28 | cable_type: 1 active, 2 optical, 3 passive copper, 4 unplugged, 5 twisted pair |
| 04h | 23:16 | cable_length (m; CMIS: multiplier in 7:6) |
| 04h | 15:8 | cable_identifier: 0 QSFP28, 1 QSFP+, 2 SFP28/SFP+, 3 QSA, 4 backplane, 5 SFP-DD, 6 QSFP-DD, 7 QSFP-CMIS, 8 OSFP |
| 14h | 16 B | vendor_name (ASCII, space padded) |
| 24h | 16 B | vendor_pn |

Reported only; the standard FEC follows the PTYS media and speed.

## The line

```
stormnic-mlx5 0.1.0: 0000:84:00.0 15b3:1015 ConnectX-4 Lx port 1: MAC …, 25GBASE-SR 25g, SFP28/SFP+ optical FS SFP-25GSR-85, fec rs (standard for 25GBASE-SR, running rs), link up, SNP installed
```

## HW-CHECKs (first metal run)

1. Does the ConnectX-4 Lx firmware answer PPLM with 0x80 bytes (the adb's
   size), or refuse a length it does not know? (`PPLM read refused` on the line.)
2. The caps it shows for 10G/40G and 25G (traced `PPLM 25g: cap …`).
3. Does PDDR's module page answer on this firmware (`PCAM` bit 43
   `PDDR_register_is_supported`)? Otherwise the line has no module part.
4. After the PAOS down/up, does the link come back within the link wait?
