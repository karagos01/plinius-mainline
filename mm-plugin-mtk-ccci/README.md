# ModemManager plugin `mtk-ccci`

ModemManager plugin for MediaTek modems behind the mainline CCCI driver
(`mtk-ccci`, e.g. the MT6878 of the Volla Phone Plinius). The modem is a
generic AT modem on the WWAN port `wwan0at0`; what the generic plugin lacks
is the data path, which MediaTek opens with the `+EIF` handshake that
Android's NetAgent does:

1. `AT+EIFCONFIG=3`, then `AT+CGACT=1,<cid>`.
2. The modem reports the interface: `+EIF: <id>, ifup, <type>, <ipv4>,
   <ipv6 link-local as hex bytes>, 0` (then `mtu` and `ifchange`). The
   interface is `ccmni<id % 100>` and changes between activations.
3. `AT+EIF=<id>,"ifst","up",<type>` opens the data path.
4. `AT+CGACT=0,<cid>` brings it down (`+EIF: <id>, ifdown, 0`, no answer).

The bearer reports IPv4 as static (the address is only in `+EIF`; `+CGPADDR`
is not supported and `+CGCONTRDP` leaves it empty), IPv6 as automatic with
the link-local address of `+EIF` (its interface identifier is the one the
network expects for SLAAC), DNS from `+CGCONTRDP` (IPv6 servers as 16
dotted decimal bytes) and the MTU from `+EIF`.

Context 0 is the PDN of the LTE attach. It can only be activated when the
attach set it up, and then the network refuses a second context to the same
APN. So the modem never picks context 0 for a new connection (minimum
profile id 1), and the bearer falls back to context 0 when context 1 is
refused.

Voice calls are the generic AT ones (`ATA`, `ATH`, `ATD`, `+CLCC`) with
three fixes:

1. **Incoming calls need approval over MIPC.** The modem does not ring
   by itself and sends no `RING`/`+CRING`/`+CLIP` on the AT port. It
   sends the MIPC command `CALL_APPROVE_INCOMING_CMD` (0x8901) to the MIPC
   client that registered for it and waits for `CALL_APPROVE_INCOMING_RSP`
   (0x8902). The plugin opens `/dev/ccci_mipc0` and registers for the
   command. It approves every incoming call, then reloads the call list,
   where the call shows up as ringing. It also registers for the MIPC call
   indications (0x4901, 0x4902) and reloads the call list on each one.
   This is how a call ends when the other side hangs up: the modem sends
   no `NO CARRIER` either. `mm-mtk-ccci-mipc.c` describes the message
   format.
2. **`AT+CLCC=?` fails** with `+CME ERROR: 100`, although `AT+CLCC`
   works. The generic code then drops a ringing call after 10 s and does
   not follow the call states. So the plugin probes `AT+CLCC` itself and
   creates calls with the `+CLCC` behaviour.
3. **International numbers come without the leading `+`**: `+CLCC`
   reports them only with type 145. The plugin adds the `+`, so numbers
   in the call log can be dialled back.

The call audio is not handled here: see `plinius-speechd` in
`modemd/` and the UCM profile of the device package.

Plugins use ModemManager's internal API: the pmOS package
`postmarketos/modemmanager-plugin-mtk-ccci` builds this directory inside
the exact ModemManager source and configuration of Alpine's `modemmanager`
package. Tested with ModemManager 1.25.95 (git 7f0bf17a) on postmarketOS
edge: SIM PIN, LTE registration, connect/disconnect through `mmcli` and
NetworkManager, IPv4 + IPv6, DNS, incoming and outgoing calls in Phosh
(gnome-calls).
