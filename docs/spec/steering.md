# Receive steering and the NIC vport

Source: *Mellanox Adapters Programmer's Reference Manual (PRM) Supporting
ConnectX-4 and ConnectX-4 Lx, Rev 0.40* only. "§" = PRM section, "T" = PRM
table number, "p" = PDF page. Nothing here comes from any other driver.
Anything the PRM leaves open is marked **HW-CHECK**.

Conventions (as everywhere in this spec): every structure is a sequence of
big-endian 32-bit dwords; "off" is the byte offset of the dword; bits are
numbered 31..0 within that dword. Offsets are relative to the start of the
structure named in the heading unless marked *abs* (absolute offset in the
command input/output mailbox data, i.e. from the opcode dword). Fields not
listed are reserved and must be written as 0.

Goal of this driver: one PF, no SR-IOV, polling, boot-time only. Received
frames for its unicast MAC, broadcast and (some) multicast — or all frames,
filtered in software — must reach **one direct TIR → one RQ**.

---

## 1. Flow table concepts (§7.11)

| Topic | What the PRM says | Ref |
|---|---|---|
| Processing stages | Flow tables exist per stage: **NIC Receive** and **NIC Transmit**; each stage has its own set of tables. | §7.11.1 |
| Structure | A table is a TCAM; each entry (Flow) = match criteria (mask) + match value. Lookup tries entries **in order from index 0**; the first hit applies its actions. | §7.11.2 |
| Mask semantics | Set mask bit = compare that bit; clear = ignore. | §7.11.2.1 |
| Actions | Drop, Forward (to one or more destinations), Allow ("continue in a well-known pre-defined manner"), Count. **Drop, Forward and Allow are mutually exclusive and exactly one of them must be set on every Flow.** Order of multiple actions undefined. | §7.11.2.2 |
| Chaining | A Forward action may name another table of the same type; lookup restarts at its entry 0. One root table per type. No loops. | §7.11.2.3 |
| Multi-path | A Forward may name several destinations (replication). Must not be nested. | §7.11.2.4 |
| Default behaviour | If **no root table** exists for a stage, the stage's default behaviour applies to every packet. The same default applies to a packet that hits a Flow with the Default (=Allow) action, or **misses every Flow**. Override by creating a root table with a Flow that has an **empty match criteria** and the required action. | §7.11.2.5 |
| Invalid flows | Not-yet-set or deleted entries never hit; lookup skips them. | §7.11.2.6 |
| Ambiguity | Software must qualify fields (e.g. match ethertype before IP fields); violating this is undefined behaviour. | §7.11.2.7 |
| Flow tag | A Flow may carry a 24-bit tag reported in the CQE `flow_tag`; tag 0 = unmarked (does not change the current tag). | §7.11.2.9, §7.11.3.6 |
| Root table | By default the root is **the table with level 0**; only one level-0 table is allowed. `SET_FLOW_TABLE_ROOT` sets/replaces it explicitly (needs the `modify_root` capability); the old root is not freed. | §7.11.3.2 |
| Levels | A Flow in a table of level L may only forward to a table of level > L. A Flow forwarding to multiple tables/TIRs must sit in a table of level < 64 and the target tables must have level ≥ 64 (exceptions with `nic_rx_multi_path_tirs`). `level` ≤ `max_ft_level`. | §7.11.3.3 |
| Handles | `table_id` unique per table type (and vport); `group_id` unique per table. | §7.11.3.3.1, §7.11.3.4.1 |
| Flow groups | A contiguous index range `[start_flow_index, end_flow_index]` sharing one match criteria. Groups must fit in the table and must not overlap; a single-entry group is allowed. | §7.11.3.4 |
| Field support | NONE match (mask all-zero) is supported for **every** field. FULL match needs the field's `ft_field_support` bit; PARTIAL match needs `ft_field_bitmask_support`. **Note:** dmac mask `01:00:00:00:00:00` is supported whenever FULL dmac match is supported. | §7.11.3.4 |
| Limits | `log_size` ≤ `log_max_ft_size`; number of tables ≤ 2^`log_max_ft_num`; total valid flows of a type ≤ 2^`log_max_flow`; destinations per Flow ≤ 2^`log_max_destination`. | §7.11.3.1, §7.11.3.6 |
| TIR exclusivity | A TIR may be the destination of Flows in **only one** table type. | §7.11.3.6 |
| Modify | `SET_FLOW_TABLE_ENTRY` with op_mod=1 atomically modifies action / flow_tag / destination list / counter list (needs `flow_modify_en`). | §7.11.3.7 |
| Freeing | Order: invalidate entries → destroy groups (all entries invalid) → destroy table (all groups freed). | §7.11.3.8 |

### 1.1 Special flows (§7.11.3.10.1)

The PRM defines only prio-tagged / untagged matching:

| Want | Mask | Value |
|---|---|---|
| Prio-tagged only (VID 0) | `vlan_tag` + `first_vid` | `vlan_tag`=1, `first_vid`=0 |
| Untagged only | `vlan_tag` | `vlan_tag`=0 |
| Untagged **or** prio-tagged (single flow) | `vlan_tag` + `first_vid` | both 0 |

The match set has no field named `vlan_tag`; it has `cvlan_tag` (0x8100)
and `svlan_tag` (T410). Read "vlan_tag" as `cvlan_tag` — **HW-CHECK**.
This driver does not need any special flow (it does not match VLANs).

### 1.2 NIC Receive table type (§7.11.4.1)

| Property | Value |
|---|---|
| `table_type` encoding | **0x0** (NIC_RX); NIC_TX = 0x1 (T384, T388 etc.) |
| Default behaviour (no root table, miss, or Allow action) | **Drop packet** |
| Destination types | Flow Table, TIR |
| Multi-path | to multiple Flow Tables; to multiple TIRs only if `nic_rx_multi_path_tirs`=1; TIRs then tables only if `nic_rx_multi_path_tirs_fts`=1 |

Consequences for this driver:

* **With no NIC_RX root table, every received packet is dropped.** A root
  table with at least one Forward→TIR Flow is mandatory.
* `ALLOW` in a NIC_RX table means "default" = drop, so it is useless here.
  Use `FWD_DEST`.
* Table miss with `table_miss_action`=0 → default → drop. So anything not
  matched by a Flow is dropped.

(NIC Transmit, §7.11.4.2: default = forward to the vport, destinations =
Flow Table only, no multi-path, no flow_tag. The driver creates no TX table.)

### 1.3 Capabilities to read first

`QUERY_HCA_CAP` (§12.3.3, T141): op_mod bit 0 = 0 max / 1 current, op_mod
bits 15:1 = capability type (0x0 general, 0x1 Ethernet offload, **0x7 NIC
Flow Table**). So op_mod = 0x0001 / 0x0003 / **0x000F** for current caps.
Capability struct starts at output 0x10 (T142).

General caps (T144/T145), offsets inside the capability struct:

| off | bits | field | use |
|---|---|---|---|
| 30h | 30 | `vport_counters` | QUERY_VPORT_COUNTER allowed |
| 34h | 31 | `vport_group_manager` | L2 table commands allowed (§12.15) |
| 34h | 25 | `nic_flow_table` | NIC flow tables supported (§12.3.3.3) |
| 34h | 9:8 | `port_type` | 0x1 = Ethernet |
| 3Ch | 3:0 | `cqe_version` | CQE `flow_tag` valid only when 1 (T68) |
| 64h | ? | `log_max_transport_domain` | only in layout T144 (row 64h, beside `log_max_pd`); no field description — bits **HW-CHECK** |
| 6Ch | 12:8 / 4:0 | `log_max_tir` / `log_max_tis` | |
| 7Ch | 20:16 | `log_max_vlan_list` | vport allowed list |
| 7Ch | 12:8 | `log_max_current_mc_list` | vport allowed list |
| 7Ch | 4:0 | `log_max_current_uc_list` | vport allowed list |
| 90h | 28:24 | `log_max_l2_table` | L2 table size |

Ethernet offload caps (op_mod type 0x1, T146/T147), off 00h:
bit 23 `self_lb_en_modifiable`, bit 22 `self_lb_mc`, bit 21 `self_lb_uc`.

NIC Flow Table caps (op_mod type 0x7, T148/T149):

| off | bits | field |
|---|---|---|
| 00h | 31 | `nic_rx_multi_path_tirs` |
| 00h | 30 | `nic_rx_multi_path_tirs_fts` |
| 40h–7Ch | 512 | `flow_table_properties_nic_receive` (T150) |
| 100h–13Ch | 512 | `flow_table_properties_nic_transmit` (T150) |

Flow Table Properties (T150/T151), offsets inside one properties block
(add 40h for NIC RX):

| off | bits | field | meaning |
|---|---|---|---|
| 00h | 31 | `ft_support` | this table type is supported |
| 00h | 30 | `flow_tag` | flow_tag in CQE supported |
| 00h | 29 | `flow_counter` | flow counters supported |
| 00h | 28 | `flow_modify_en` | SET_FTE op_mod=1 supported |
| 00h | 27 | `modify_root` | SET_FLOW_TABLE_ROOT supported |
| 00h | 26 | `identified_miss_table` | table_miss_action=1 supported |
| 00h | 25 | `flow_table_modify` | MODIFY_FLOW_TABLE supported |
| 00h | 22 | `reset_root_to_default` | see §9 inconsistencies |
| 04h | 29:24 | `log_max_ft_size` | max log2 entries per table |
| 04h | 7:0 | `max_ft_level` | max `level` |
| 0Ch | 7:0 | `log_max_ft_num` | max log2 tables |
| 10h | 15:8 | `log_max_flow_counter` | per Flow |
| 10h | 7:0 | `log_max_destination` | per Flow |
| 14h | 7:0 | `log_max_flow` | total flows of this type |
| 20h–2Ch | 128 | `ft_field_support` (T152/T153) | FULL-match support per field |
| 30h–3Ch | 128 | `ft_field_bitmask_support` (T152/T153) | PARTIAL-match support per field |

`ft_field_support` / `ft_field_bitmask_support` dword 00h (T153):
31 outer_dmac, 30 outer_smac, 29 outer_ether_type, 27 outer_first_prio,
26 outer_first_cfi, 25 outer_first_vid, 23 outer_second_prio,
22 outer_second_cfi, 21 outer_second_vid, 20 outer_ipv6_flow_label,
19 outer_sip, 18 outer_dip, 17 outer_frag, 16 outer_ip_protocol,
15 outer_ip_ecn, 14 outer_ip_dscp, 13 outer_udp_sport, 12 outer_udp_dport,
11 outer_tcp_sport, 10 outer_tcp_dport, 9 outer_tcp_flags. Dwords 04h–0Ch:
not described in Rev 0.40.

---

## 2. Flow table commands (§12.14)

Opcodes (T127): SET_FLOW_TABLE_ROOT **0x92F**, CREATE_FLOW_TABLE **0x930**,
DESTROY_FLOW_TABLE **0x931**, QUERY_FLOW_TABLE 0x932, CREATE_FLOW_GROUP
**0x933**, DESTROY_FLOW_GROUP **0x934**, QUERY_FLOW_GROUP 0x935,
SET_FLOW_TABLE_ENTRY **0x936**, QUERY_FLOW_TABLE_ENTRY 0x937,
DELETE_FLOW_TABLE_ENTRY **0x938**, ALLOC_FLOW_COUNTER 0x939,
DEALLOC_FLOW_COUNTER 0x93A, QUERY_FLOW_COUNTER 0x93B, MODIFY_FLOW_TABLE 0x93C.

Every input starts `00h[31:16] opcode`, `04h[15:0] op_mod`; every output
starts `00h[31:24] status`, `04h[31:0] syndrome`. Only the rest is listed.
None of the Rev 0.40 flow-table inputs has a vport / other_vport field.

### 2.1 CREATE_FLOW_TABLE (§12.14.1, T383–T386)

Input — 0x40 bytes:

| off (abs) | bits | field | value / meaning |
|---|---|---|---|
| 04h | 15:0 | op_mod | 0 (no values defined) |
| 08h–0Ch | | reserved | |
| 10h | 31:24 | `table_type` | 0x0 NIC_RX, 0x1 NIC_TX |
| 14h | | reserved | |
| 18h–3Ch | 320 | flow table context | T387, begins at **abs 18h** |

Flow Table Context (T387/T388), 0x28 bytes:

| off | abs | bits | field | meaning |
|---|---|---|---|---|
| 00h | 18h | 31 | reserved | |
| 00h | 18h | 27:24 | `table_miss_action` | 0x0 default_miss_table (type default, §7.11.4 → drop for NIC_RX); 0x1 identified_miss_table (go to `table_miss_id`; needs `identified_miss_table`) |
| 00h | 18h | 23:16 | `level` | chaining level; 0 = default root |
| 00h | 18h | 7:0 | `log_size` | log2 number of flow entries |
| 04h | 1Ch | 23:0 | `table_miss_id` | next table on miss, valid when action=1; same type and higher level |
| 08h–24h | 20h–3Ch | | reserved | |

Output — 0x10 bytes: `08h[23:0] table_id`; 0Ch reserved.

### 2.2 SET_FLOW_TABLE_ROOT (§12.14.4, T397–T400)

Input — 0x40 bytes: 08h–0Ch reserved; `10h[31:24] table_type`;
`14h[23:0] table_id`; 18h–3Ch reserved. Output — 0x10 bytes (08h–0Ch reserved).
Replaces the root of that table type; the previous root stays allocated.
Supported only when `modify_root`=1 (§7.11.3.2, T151 bit 27).

### 2.3 CREATE_FLOW_GROUP (§12.14.6, T405, T406, T417, T418)

Input — **0x400 bytes**:

| off (abs) | bits | field | meaning |
|---|---|---|---|
| 08h–0Ch | | reserved | |
| 10h | 31:24 | `table_type` | 0x0 NIC_RX |
| 14h | 23:0 | `table_id` | from CREATE_FLOW_TABLE |
| 18h | | reserved | |
| 1Ch | 31:0 | `start_flow_index` | first entry of the group |
| 20h | | reserved | |
| 24h | 31:0 | `end_flow_index` | last entry of the group (inclusive) |
| 28h–38h | | reserved | |
| 3Ch | 7:0 | `match_criteria_enable` | bit 0 outer_headers, bit 1 misc_parameters, bit 2 inner_headers, others reserved |
| 40h–23Ch | 4096 | `match_criteria` | fte_match_param (T407), begins at **abs 40h** |
| 240h–3FCh | | reserved | |

Output — 0x10 bytes: `08h[23:0] group_id`; 0Ch reserved.

A sub-set of `match_criteria` whose enable bit is clear is reserved and
does not participate in matching (T406).

### 2.4 Match parameters — fte_match_param (T407/T408), 0x200 bytes

| off | size | sub-structure | enabled by |
|---|---|---|---|
| 00h–3Ch | 512 b | `outer_headers` (T409/T410) | `match_criteria_enable` bit 0 |
| 40h–7Ch | 512 b | `misc_parameters` (T415/T416) | bit 1 |
| 80h–BCh | 512 b | not named in T407; by the enable bit and the Layer 2-4 format, `inner_headers` (same layout as outer) — **HW-CHECK** | bit 2 |
| C0h–1FCh | | reserved | |

The same layout is used for the group's mask (`match_criteria`) and the
entry's value (`match_value`).

#### outer_headers — Flow Table Entry Match Set Layer 2-4 (T409/T410)

| off | bits | field | meaning |
|---|---|---|---|
| 00h | 31:0 | `smac[47:16]` | source MAC |
| 04h | 31:16 | `smac[15:0]` | |
| 04h | 15:0 | `ethertype` | ethertype **after the last VLAN tag** |
| 08h | 31:0 | `dmac[47:16]` | destination MAC |
| 0Ch | 31:16 | `dmac[15:0]` | |
| 0Ch | 15:13 | `first_prio` | PCP of first VLAN tag (valid if cvlan_tag or svlan_tag) |
| 0Ch | 12 | `first_cfi` | CFI of first tag |
| 0Ch | 11:0 | `first_vid` | VID of first tag |
| 10h | 31:24 | `ip_protocol` | |
| 10h | 23:18 | `ip_dscp` | |
| 10h | 17:16 | `ip_ecn` | |
| 10h | 15 | `cvlan_tag` | first VLAN is C-VLAN (0x8100) |
| 10h | 14 | `svlan_tag` | first VLAN is S-VLAN (PRM writes "0x8a88") |
| 10h | 13 | `frag` | IP fragment |
| 10h | 8:0 | `tcp_flags` | bit0 FIN … bit8 NS |
| 14h | 31:16 | `tcp_sport` | |
| 14h | 15:0 | `tcp_dport` | |
| 18h | | reserved | |
| 1Ch | 31:16 | `udp_sport` | |
| 1Ch | 15:0 | `udp_dport` | |
| 20h–2Ch | 128 | `src_ipv4/src_ipv6` | IPv4 in the last dword (2Ch) (T413/T414) |
| 30h–3Ch | 128 | `dst_ipv4/dst_ipv6` | IPv4 in 3Ch |

Byte view of the L2 part: smac occupies bytes 00h–05h, ethertype bytes
06h–07h, dmac bytes **08h–0Dh**, VLAN TCI-like fields bytes 0Eh–0Fh. If
`dmac[47:40]` is the first octet on the wire (the PRM's mask notation
`01:00:00:00:00:00` for the group bit implies it, but does not state it —
**HW-CHECK**), the MAC is stored in wire order: for `aa:bb:cc:dd:ee:ff`,
dword 08h = `0xaabbccdd`, dword 0Ch bits 31:16 = `0xeeff`.

#### misc_parameters (T415/T416)

| off | bits | field |
|---|---|---|
| 08h | 31:29 | `outer_second_prio` |
| 08h | 28 | `outer_second_cfi` |
| 08h | 27:16 | `outer_second_vid` |
| 0Ch | 31 | `outer_second_cvlan_tag` |
| 0Ch | 30 | `inner_second_cvlan_tag` (field description only) |
| 0Ch | 29 | `outer_second_svlan_tag` |
| 1Ch | 19:0 | `outer_ipv6_flow_label` |

Everything else in misc_parameters is reserved in Rev 0.40. Not used by
this driver.

### 2.5 SET_FLOW_TABLE_ENTRY (§12.14.9, T427–T436)

Input — 0x40 header + flow context:

| off (abs) | bits | field | meaning |
|---|---|---|---|
| 04h | 15:0 | op_mod | 0 = set new entry; 1 = modify existing (needs `flow_modify_en`) |
| 08h–0Ch | | reserved | |
| 10h | 31:24 | `table_type` | 0x0 NIC_RX |
| 14h | 23:0 | `table_id` | |
| 18h | 7:0 | `modify_enable_mask` | op_mod=1 only: bit0 action, bit1 flow_tag, bit2 destination list, bit3 flow counter list |
| 1Ch | | reserved | |
| 20h | 31:0 | `flow_index` | position in the table; 0 matched first; must lie in the group's range |
| 24h–3Ch | | reserved | |
| 40h–… | 6208 | `flow_context` | T429, begins at **abs 40h** |

FLOW_CONTEXT (T429/T430):

| off | abs | bits | field | meaning |
|---|---|---|---|---|
| 00h | 40h | | reserved | |
| 04h | 44h | 31:0 | `group_id` | from CREATE_FLOW_GROUP |
| 08h | 48h | 23:0 | `flow_tag` | reported in CQE; 0 = unmarked; needs `flow_tag` cap |
| 0Ch | 4Ch | 15:0 | `action` | bit0 **ALLOW** (0x1), bit1 **DROP** (0x2), bit2 **FWD_DEST** (0x4), bit3 **COUNT** (0x8) |
| 10h | 50h | 23:0 | `destination_list_size` | number of destinations; valid with FWD_DEST |
| 14h | 54h | 23:0 | `flow_counter_list_size` | number of counters after the destinations; valid with COUNT |
| 18h–3Ch | 58h–7Ch | | reserved | |
| 40h–23Ch | **80h–27Ch** | 4096 | `match_value` | fte_match_param (T407); only fields enabled by the group's criteria are meaningful, others must be 0 |
| 240h–2FCh | 280h–33Ch | | reserved | |
| 300h–… | **340h–…** | 64 each | `destination[]` then `flow_counter[]` | first `destination_list_size` entries are destinations, then `flow_counter_list_size` counters |

Destination entry (T433/T434), 8 bytes:

| off | bits | field | meaning |
|---|---|---|---|
| 00h | 31:24 | `destination_type` | **1 = FLOW_TABLE**, **2 = TIR** |
| 00h | 23:0 | `destination_id` | flow table id or **TIR number** |
| 04h | | reserved | |

Flow counter entry (T435/T436), 8 bytes: `00h[15:0] flow_counter_id`; 04h reserved.

With one TIR destination and no counter the input is **0x348 bytes**
(destination[0] at abs 340h, reserved dword at 344h). Output — 0x10 bytes
(08h–0Ch reserved).

All destinations of one Flow must be of the same type (§7.11.3.6).

### 2.6 DELETE_FLOW_TABLE_ENTRY (§12.14.11, T441–T444)

Input — 0x40 bytes: 08h–0Ch reserved; `10h[31:24] table_type`;
`14h[23:0] table_id`; 18h–1Ch reserved; `20h[31:0] flow_index`; 24h–3Ch
reserved. Output — 0x10 bytes. The entry must be valid.

### 2.7 DESTROY_FLOW_GROUP (§12.14.7, T419–T422)

Input — 0x40 bytes: 08h–0Ch reserved; `10h[31:24] table_type`;
`14h[23:0] table_id`; `18h[31:0] group_id`; 1Ch–3Ch reserved. Output —
0x10 bytes. All entries of the group must be invalid first (§7.11.3.8).

### 2.8 DESTROY_FLOW_TABLE (§12.14.3, T393–T396)

Input — 0x40 bytes: 08h–0Ch reserved; `10h[31:24] table_type`;
`14h[23:0] table_id`; 18h–3Ch reserved. Output — 0x10 bytes. All groups
must be destroyed first (§7.11.3.8). If the table is the root, the
behaviour after destroying it is not stated — see HW-CHECK.

### 2.9 Other flow-table commands (reference)

* QUERY_FLOW_TABLE (T401–T404): input like DESTROY_FLOW_TABLE; output
  flow table context at abs 18h–3Ch.
* MODIFY_FLOW_TABLE (T389–T392): `0Ch[15:0] modify_field_select` (bit 0 =
  table_miss_action/table_miss_id), `10h` table_type, `14h` table_id,
  context at 18h–3Ch. Needs `flow_table_modify`.
* QUERY_FLOW_GROUP (T423–T426): input like DESTROY_FLOW_GROUP; output
  `1Ch` start, `24h` end, `3Ch[7:0]` match_criteria_enable, `40h–23Ch`
  match_criteria.
* QUERY_FLOW_TABLE_ENTRY (T437–T440): input `10h` type, `14h` table_id,
  `20h` flow_index; output flow_context at abs 40h.
* Flow counters (T445–T456, T491): ALLOC returns `08h[15:0]
  flow_counter_id`; QUERY_FLOW_COUNTER input `18h[31]` clear, `1Ch[15:0]`
  id; output TRAFFIC_COUNTER (packets 64 b, octets 64 b) at 10h.

---

## 3. Building the entries

### 3.1 Is `match_criteria_enable` = 0 allowed?

Yes, by the PRM's own description: the default behaviour is overridden "by
allocating the root Flow Table, and defining a Flow with an **empty match
criteria** and the required action" (§7.11.2.5), and "A NONE match is
supported for all fields in the match_criteria" (§7.11.3.4). With
`match_criteria_enable` = 0 every sub-structure is reserved (T406), so the
whole mask is zero and every packet matches. Whether firmware accepts the
enable byte = 0 (rather than bit 0 set with an all-zero outer mask) is not
stated explicitly — **HW-CHECK**; fallback: `match_criteria_enable` = 0x01
with an all-zero `outer_headers` mask, which is equivalent.

### 3.2 (a) Match-all entry → TIR

| Command | Field (abs) | Value |
|---|---|---|
| CREATE_FLOW_GROUP | 1Ch start / 24h end | n / n |
| | 3Ch match_criteria_enable | 0x00 |
| | 40h–23Ch match_criteria | all 0 |
| SET_FLOW_TABLE_ENTRY | 20h flow_index | n |
| | 44h group_id | group |
| | 4Ch action | 0x0004 (FWD_DEST) |
| | 50h destination_list_size | 1 |
| | 80h–27Ch match_value | all 0 |
| | 340h | `(2 << 24) \| tirn` |

### 3.3 (b) dmac exact entry

Group: `match_criteria_enable` = 0x01; mask at abs **48h = 0xFFFFFFFF**,
abs **4Ch = 0xFFFF0000** (outer_headers 08h/0Ch). Needs `ft_field_support`
bit 31 (outer_dmac).
Entry: match_value abs **88h** = dmac[47:16], abs **8Ch[31:16]** =
dmac[15:0], everything else 0. E.g. own MAC `aa:bb:cc:dd:ee:ff` →
88h = 0xaabbccdd, 8Ch = 0xeeff0000; broadcast → 88h = 0xFFFFFFFF,
8Ch = 0xFFFF0000. One group can hold both (two entries, same mask).

### 3.4 (c) Multicast-bit entry (dmac[40] = 1)

Group: `match_criteria_enable` = 0x01; mask abs **48h = 0x01000000** (dmac
bit 40 = dword 08h bit 24 = first dmac byte, value 0x01), abs 4Ch = 0.
This exact mask (`01:00:00:00:00:00`) is guaranteed supported whenever
FULL dmac match is supported (§7.11.3.4 note) — no
`ft_field_bitmask_support` needed.
Entry: match_value abs **88h = 0x01000000**, rest 0. This also matches
broadcast (ff:ff:… has the group bit set). Because groups must not
overlap and entries are searched by index, put the exact-dmac group at
lower indices than the multicast group.

---

## 4. L2 table commands (§12.15)

Purpose (PRM wording): commands "that SW should use to inform the device of
its L2 addresses". "SW **must** use SET_L2_TABLE_ENTRY to set its L2
addresses in the stage defined in Section 11.2 HCA Driver Start-up", and
again for addresses learned later. An entry cannot be SET twice without a
DELETE in between. **Allowed only when `HCA_CAP.vport_group_manager` == 1.**

Opcodes (T127): SET_L2_TABLE_ENTRY **0x829**, QUERY_L2_TABLE_ENTRY 0x82A,
DELETE_L2_TABLE_ENTRY 0x82B.

SET_L2_TABLE_ENTRY input (T457/T458) — 0x40 bytes:

| off (abs) | bits | field | meaning |
|---|---|---|---|
| 08h–10h | | reserved | |
| 14h | 23:0 | `table_index` | < 2^`log_max_l2_table` (PRM: "less than log_max_l2_table") |
| 18h | | reserved | |
| 1Ch | 12 | `vlan_valid` | 1 = `vlan` valid; 0 = MAC only |
| 1Ch | 11:0 | `vlan` | |
| 20h–24h | 64 | `mac_address` | MAC Address Layout T19/T20: 20h[15:0] = mac[47:32], 24h = mac[31:0] |
| 28h–3Ch | | reserved | |

Output — 0x10 bytes. QUERY_L2_TABLE_ENTRY (T461–T464): input `14h[23:0]
table_index`; output `1Ch` vlan_valid/vlan, `20h–24h` mac. DELETE (T465–T468):
input `14h[23:0] table_index`.

**Does a PF driver need it to receive unicast?** The PRM does not say what
the L2 table does to delivery, nor how it relates to the NIC_RX flow table
or the vport allowed-UC list. Section 11.2 does not exist in Rev 0.40; the
start-up sequence that does exist (§8.2) never mentions SET_L2_TABLE_ENTRY
and instead sets the current MAC with MODIFY_(NIC_)VPORT_CONTEXT.
**PRM does not say — HW-CHECK.** Recommendation: if `vport_group_manager`
= 1, issue SET_L2_TABLE_ENTRY(index 0, vlan_valid 0, own MAC) after the
vport context is set (harmless and satisfies the "must"); ignore failure
with a log line; delete it at teardown. Verify on hardware whether unicast
arrives without it.

---

## 5. Vport (§12.16) and NIC_Vport context (§7.3)

Opcodes (T127): QUERY_VPORT_STATE **0x750**, MODIFY_VPORT_STATE **0x751**,
QUERY_NIC_VPORT_CONTEXT **0x754**, MODIFY_NIC_VPORT_CONTEXT **0x755**.
No command in Rev 0.40 has a `vport_number`/`other_vport` field: all act on
the calling function's own vport.

### 5.1 QUERY_VPORT_STATE (§12.16.1, T470–T473)

Input — 0x10 bytes: `04h[15:0] op_mod` = 0 (vnic_vport); 08h–0Ch reserved.

Output — 0x10 bytes:

| off | bits | field | values |
|---|---|---|---|
| 08h | | reserved | |
| 0Ch | 7:4 | `admin_state` | state required by SW: 0x0 down, 0x1 up |
| 0Ch | 3:0 | `state` | operational: 0x0 down, 0x1 up, others reserved |

### 5.2 MODIFY_VPORT_STATE (§12.16.2, T474–T477)

Input — 0x10 bytes: `04h[15:0] op_mod` = 0 (nic_vport); 08h reserved;
`0Ch[7:4] admin_state` = 0x0 down / 0x1 up. Output — 0x10 bytes.

The PRM does not say whether a PF must set its own vport admin up (§8.2
only queries it; `vport_group_manager` is "responsible for enabling other
Vports"). **HW-CHECK.** Recommendation: query; if `admin_state` = down,
issue MODIFY_VPORT_STATE(up); never set it down at teardown unless it was
down at start.

### 5.3 NIC_Vport context (§7.3, T15–T20)

| off | bits | field | meaning |
|---|---|---|---|
| 00h | 26:24 | `min_wqe_inline_mode` | minimal SQ inline mode for this vport; valid only when Ethernet cap `wqe_inline_mode` = 1. Not selectable by MODIFY. |
| 04h–20h | | reserved | |
| 24h | 15:0 | `mtu` | vport MTU; in query, 0 = equal to physical port MTU |
| 28h–ECh | | reserved | |
| F0h | 31 | `promisc_uc` | "request to allow any unicast address on vlan list" |
| F0h | 30 | `promisc_mc` | "request to allow any multicast address on vlan list" |
| F0h | 29 | `promisc_all` | "request to allow all traffic" |
| F0h | 26:24 | `allowed_list_type` | 0 current_uc_mac_address, 1 current_mc_mac_address, 2 vlan_list |
| F0h | 11:0 | `allowed_list_size` | number of list entries (limits: `log_max_current_uc_list` / `_mc_list` / `log_max_vlan_list`, presumably 2^n) |
| F4h–F8h | 64 | `permanent_address` | MAC layout T19: F4h[15:0] = mac[47:32], F8h[31:0] = mac[31:0] |
| FCh | | reserved | |
| 100h + 8·k | 64 | `current_uc_mac_address[k]` / `current_mc_mac_address[k]` / `vlan[k]` | the allowed list; type by `allowed_list_type` |

List entry formats: MAC (T19/T20) — `00h[15:0] mac_addr[47:32]`,
`04h[31:0] mac_addr[31:0]`. VLAN (T17/T18) — `00h[11:0] vlan`, 04h
reserved.

Nominal size 2112 bits = 0x108 bytes (through entry [0]); each extra entry
adds 8 bytes.

Fields asked about that **do not exist** in Rev 0.40 T15: `roce_en`, any
`vport_number`, `disable_uc_local_lb`/`disable_mc_local_lb`, a
`system_image_guid` etc. — PRM does not say.

### 5.4 QUERY_NIC_VPORT_CONTEXT (§12.16.3, T478–T481)

Input — 0x10 bytes: `04h` op_mod (no values defined; use 0); 08h reserved;
`0Ch[26:24] allowed_list_type` (which list to return). Output: 08h–0Ch
reserved, **nic_vport_context at abs 10h**. So `mtu` at abs 34h,
promisc/list type/size at abs 100h, `permanent_address` at abs **104h[15:0]
/ 108h**, list entry k at abs 110h + 8·k. Output length needed:
0x10 + 0x100 + 8·2^`log_max_current_*_list`.

### 5.5 MODIFY_NIC_VPORT_CONTEXT (§12.16.4, T482–T485)

Input:

| off (abs) | bits | field | meaning |
|---|---|---|---|
| 04h | 15:0 | op_mod | no values defined; use 0 |
| 08h | | reserved | |
| 0Ch | 31:0 | `field_select` | bit 2 **addresses_list** (allowed_list_type, allowed_list_size, the list); bit 4 **promisc** (promisc_uc/mc/all); bit 6 **mtu**; others not defined |
| 10h–FCh | | reserved | |
| 100h–… | 2112+ | `nic_vport_context` | T15, begins at **abs 100h** (note: query output puts it at 10h) |

So in MODIFY: `mtu` abs 124h[15:0]; promisc/list type/size abs **1F0h**;
permanent_address abs 1F4h/1F8h (not selectable); list entry k at abs
**200h + 8·k**. With one entry the input is 0x208 bytes. Output — 0x10
bytes.

A single MODIFY sets one list type (allowed_list_type is one field). To set
both a UC list and an MC list, issue two MODIFYs, each with field_select
bit 2 and its own list type — **HW-CHECK** that the second does not clear
the first.

### 5.6 What the vport context does for delivery

The PRM gives only the field descriptions above: the lists are "the
allowed addresses on a nic_vport", promisc bits "request to allow". It
does not say how vport filtering composes with the NIC_RX flow table (which
runs first, which drops), whether broadcast counts as multicast for
`promisc_mc`, or whether a PF without an e-switch filters on the lists at
all. §8.2 does require, for Ethernet, querying the vport context for the
permanent MAC and modifying it to set the current MAC. **HW-CHECK** all
delivery semantics; the vport counters (§6) are the tool for it.

---

## 6. Vport counters (§12.17, T486–T492)

Present when `vport_counters` (general caps 30h bit 30) = 1. Opcode
QUERY_VPORT_COUNTER **0x770**.

Input — 0x20 bytes: `04h[15:0] op_mod` = 0 (vport_counters); 08h–14h
reserved; `18h[31] clear` (read and reset); 1Ch reserved.

Output — 0x210 bytes, each counter a TRAFFIC_COUNTER (T491/T492: 00h
packets[63:32], 04h packets[31:0], 08h octets[63:32], 0Ch octets[31:0]):

| off (abs) | counter |
|---|---|
| 10h–1Ch | received errors |
| 20h–2Ch | transmit errors |
| 30h–6Ch | reserved |
| 70h–7Ch | received eth broadcast |
| 80h–8Ch | transmitted eth broadcast |
| 90h–9Ch | received eth unicast |
| A0h–ACh | transmitted eth unicast |
| B0h–BCh | received eth multicast |
| C0h–CCh | transmitted eth multicast |
| D0h–20Ch | reserved |

Diagnostic use: RX counters rising while the RQ's CQ stays empty points at
the flow table / TIR; counters not rising points at vport filtering, vport
or port state.

---

## 7. Self-loopback and transport domains (§3.4)

* Transport Domain = a set of TIS/TIRs belonging to one consumer. Packets
  looped back by the NIC that were sent by a TIS of the domain and would be
  delivered to a TIR of the **same** domain are "self-loopback" and are
  accepted or blocked per TIR (§3.4).
* Support: `HCA_CAP.log_max_transport_domain` (general caps, layout row
  64h; bits HW-CHECK).
* ALLOC_TRANSPORT_DOMAIN **0x816** (§12.3.18, T213–T216): input 0x10 bytes,
  nothing but opcode/op_mod; output `08h[23:0] transport_domain`.
  DEALLOC_TRANSPORT_DOMAIN **0x817** (§12.3.19, T217–T220): input
  `08h[23:0] transport_domain`.
* TIS context (T49/T50): `24h[23:0] transport_domain` (required at
  CREATE_TIS, T51). TIS has no loopback bit.
* TIR context (T44/T45): `24h[23:0] transport_domain`; **`24h[25:24]
  self_lb_en`**: bit 24 (mask bit 0) enable_unicast — needs Ethernet cap
  `self_lb_uc`; bit 25 (mask bit 1) enable_multicast — needs `self_lb_mc`.
  Clear = that self-loopback traffic is not delivered to this TIR.
  MODIFY_TIR bitmask bit 4 = self_lb_en, only if `self_lb_en_modifiable`
  (T48). Other TIR fields used here: `04h[31:28] disp_type` = 0x0 Direct,
  `1Ch[23:0] inline_rqn`, `24h[31:28] rx_hash_fn` = 0x0 HASH_NONE (required
  for Direct).

For this driver: allocate one transport domain, put the TIS and the TIR in
it, leave `self_lb_en` = 0 so the driver never receives its own transmitted
broadcasts/multicasts (e.g. its own DHCP DISCOVER). The PRM does not say
whether NIC-local loopback happens at all for a lone PF without other
consumers — **HW-CHECK**.

---

## 8. Recommended minimal sequence

Assumes the HCA is initialized (§8.2 up to CREATE_EQ), and a PD, CQ, RQ
(state RDY), TIS and SQ exist or are created as noted.

| # | Step | Ref |
|---|---|---|
| 1 | QUERY_HCA_CAP general (op_mod 0x0001): check `port_type`=1, `nic_flow_table`=1; note `vport_group_manager`, `vport_counters`, `log_max_current_uc_list`, `cqe_version`. | §12.3.3, T144/T145 |
| 2 | QUERY_HCA_CAP NIC flow table (op_mod 0x000F): NIC_RX props at 40h: require `ft_support`; note `modify_root`, `log_max_ft_size` ≥ 2, `max_ft_level`, `ft_field_support` bit 31 (outer_dmac) if filtering in HW. | T148–T153 |
| 3 | QUERY_HCA_CAP Ethernet (op_mod 0x0003): note `self_lb_uc/mc`. | T146/T147 |
| 4 | QUERY_VPORT_STATE; if `admin_state` ≠ up → MODIFY_VPORT_STATE(admin_state=1). | §8.2, §12.16.1–2 |
| 5 | QUERY_NIC_VPORT_CONTEXT(allowed_list_type=0) → `permanent_address` at abs 104h/108h = the station MAC. | §8.2, §12.16.3 |
| 6 | MODIFY_NIC_VPORT_CONTEXT: field_select = bit 2 \| bit 4 (0x14); abs 1F0h: `promisc_mc`=1 (bit 30), `allowed_list_type`=0, `allowed_list_size`=1; abs 200h/204h = own MAC. (Debug/fallback: `promisc_all`=1, bit 29.) | §8.2, §12.16.4, T15/T16 |
| 7 | If `vport_group_manager`=1: SET_L2_TABLE_ENTRY(table_index 0, vlan_valid 0, own MAC); tolerate failure. | §12.15.1 |
| 8 | ALLOC_TRANSPORT_DOMAIN → td. | §3.4, §12.3.18 |
| 9 | CREATE_TIS with `transport_domain`=td (TX side; other spec). | T49/T50 |
| 10 | CREATE_TIR: `disp_type`=0 Direct, `inline_rqn`=RQ, `rx_hash_fn`=0, `transport_domain`=td, `self_lb_en`=0. | T44/T45 |
| 11 | CREATE_FLOW_TABLE: table_type 0x0, level 0, log_size 2 (4 entries), table_miss_action 0 → table_id. | §12.14.1, T383–T388 |
| 12 | CREATE_FLOW_GROUP: index 0..0, match_criteria_enable 0, criteria all 0 → group_id. | §12.14.6, §3.1 above |
| 13 | SET_FLOW_TABLE_ENTRY: op_mod 0, flow_index 0, group_id, flow_tag 0 (or a non-zero tag if `flow_tag` cap and `cqe_version`=1, for diagnostics), action 0x4, destination_list_size 1, match_value 0, destination[0] = type 2 / TIR number. | §12.14.9, T427–T434 |
| 14 | If `modify_root`=1: SET_FLOW_TABLE_ROOT(table_type 0, table_id). Otherwise rely on the level-0 table being the default root. | §7.11.3.2, §12.14.4 |
| 15 | Post RX WQEs, ring the RQ doorbell; software drops frames whose dmac is not own MAC, broadcast or a wanted multicast. | §7.4 (other spec) |
| 16 | On no traffic: QUERY_VPORT_COUNTER and compare RX counters with CQEs (§6). | §12.17.1 |

Hardware-filtered variant (instead of steps 12–13): group A index 0..1 with
dmac FULL mask (entries own MAC, ff:ff:ff:ff:ff:ff), group B index 2..2 with
dmac mask 01:00:00:00:00:00 (multicast entry); all FWD_DEST → TIR. Misses
drop (§7.11.4.1).

Teardown (§7.11.3.8, §3.2.1): DELETE_FLOW_TABLE_ENTRY (each index) →
DESTROY_FLOW_GROUP → DESTROY_FLOW_TABLE → DESTROY_TIR → DESTROY_RQ/SQ/TIS →
DEALLOC_TRANSPORT_DOMAIN → DELETE_L2_TABLE_ENTRY (if set) → restore vport
context (promisc off) if changed. TEARDOWN_HCA (§8.3) then frees all.

---

## 9. HW-CHECK list and PRM inconsistencies

### HW-CHECK (PRM does not say)

1. Whether `match_criteria_enable` = 0 is accepted literally, or 0x01 with a
   zero mask is required (§3.1).
2. Whether `dmac[47:40]` is the first octet on the wire (byte order of MAC
   fields in match params, T19 MAC layout and vport lists).
3. Whether a PF must issue SET_L2_TABLE_ENTRY to receive unicast, and what
   the L2 table does (§12.15 vs §8.2).
4. How the NIC_Vport allowed lists / promisc bits interact with the NIC_RX
   flow table for a PF; whether broadcast is covered by `promisc_mc` or the
   MC list; whether `promisc_*` is honoured on a PF without e-switch.
5. Whether two MODIFY_NIC_VPORT_CONTEXT calls (UC list, then MC list)
   accumulate or replace each other.
6. Whether the PF's own vport needs MODIFY_VPORT_STATE(up), and the
   relationship of vport state to physical port state.
7. Whether a level-0 table becomes root automatically at creation (§7.11.3.2
   says "by default") when `modify_root` = 0, and what happens to traffic
   when the root table is destroyed (revert to default/drop?).
8. Bits of `log_max_transport_domain` (layout only, no description).
9. Whether NIC-local loopback to the PF occurs at all with one consumer, and
   the default if `self_lb_en` caps are 0.
10. `inner_headers` placement at 80h–BCh of fte_match_param (implied, not
    labelled).
11. CQE `flow_tag` location: layout T67 shows it in the 38h row beside
    `s_wqe_opcode` (bits 23:0 by width); valid only with `cqe_version`=1.

### PRM inconsistencies found

1. §7.3 refers to "Section 3.1.2 Vport Context, page 109" — no such section
   (3.1 has only 3.1.1). §12.15 refers to "Section 11.2 HCA Driver Start-up,
   page 366" and §8.2 to "Section 14.1.7 Virtual NIC Start-Up, page 409" —
   neither exists (the PRM has 316 pages; start-up is §8.2).
2. §8.2 names the commands "QUERY_VPORT_CONTEXT"/"MODIFY_VPORT_CONTEXT";
   the real ones are QUERY_/MODIFY_NIC_VPORT_CONTEXT.
3. T16 `allowed_list_size`: the third bullet says
   "allowed_list_type==current_uc_mac_address … <= log_max_vlan_list"
   (should be vlan_list), and the limits compare a size to log2 values.
4. QUERY_NIC_VPORT_CONTEXT output puts the context at 10h; MODIFY input at
   100h, with a reserved range written "0Ch–FCh" that overlaps
   `field_select` at 0Ch.
5. Context sizes cover a single list element: nic_vport_context "2112" bits
   (0x108, entry [0] only), flow_context "6208" bits (0x308, one
   destination only).
6. §7.11.3.2 and T430 cite `HCA_CAP.modify_root` / `HCA_CAP.flow_tag` /
   `HCA_CAP.modify_flow_en`; these are Flow Table Properties bits (T151;
   there named `flow_modify_en`).
7. T151 `reset_root_to_default` points to T398 (SET_FLOW_TABLE_ROOT) which
   has no field for it.
8. §7.11.3.8 says DELETE_FLOW_TABLE_ENTRY / DESTROY_FLOW_TABLE identify the
   group by `group_id`; T441 has no group_id field.
9. `group_id` is 23:0 in CREATE_FLOW_GROUP output (T418) but 31:0 in
   DESTROY_FLOW_GROUP (T420) and FLOW_CONTEXT (T430).
10. T407/T408 list only outer_headers and misc_parameters, but
    `match_criteria_enable` bit 2 = inner_headers (T406); QUERY_FLOW_GROUP
    (T426) omits bit 2.
11. §7.11.3.10.1 uses `vlan_tag`; the match set has only `cvlan_tag` /
    `svlan_tag`.
12. T410 gives the S-VLAN ethertype as 0x8a88 (802.1ad is 0x88a8).
13. T416 misc: bit 30 `inner_second_cvlan_tag`, bit 29
    `outer_second_svlan_tag`, while the T415 layout shows outer cvlan/svlan
    side by side.
14. §7.11.2.2 says Drop/Forward/Allow are exclusive; §7.11.3.6 says
    "Drop, Default and Forward" — Default = Allow (bit 0).
15. §7.11.3.6 Count text: "number of destinations in a single flow must not
    exceed log_max_flow_counter" (means counters).
16. T383 CREATE_FLOW_TABLE layout shows both an "08h" and an "08h–0Ch" row.
17. T430 `destination_list_size` described as "Handler of the size".
18. QUERY_VPORT_STATE op_mod 0 = "vnic_vport", MODIFY op_mod 0 =
    "nic_vport"; §12.15.3 is titled "Invalidate Flow Table Entry" for the L2
    delete; T458 says index "less that HCA_CAP.log_max_l2_table" (a log).
19. `promisc_uc`/`promisc_mc` "on vlan list" — undefined meaning when no
    VLAN list is set.
20. QUERY_FLOW_COUNTER (T454) refers to `number_of_flow_counter` /
    `num_of_counters`, a field absent from its input layout.
