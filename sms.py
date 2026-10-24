#!/usr/bin/env python3
"""Send and receive SMS over the AT port of the MT6878 modem.

    sms.py send <number> <text>   send (long texts as a concatenated SMS)
    sms.py list [--delete]        print the messages stored on the SIM
    sms.py watch [--delete]       print new messages as they arrive
    sms.py delete <index>...      delete stored messages

Works in PDU mode: text mode of the modem can neither send UCS2 (Czech
diacritics) nor long messages. Received messages are stored on the SIM
(AT+CNMI=2,1), parts of a concatenated message are joined.

The modem has a single AT channel and every reader of it gets a part of
the responses, so its users (this script, mobile-data.py) hold a flock
on it while they talk to the modem. watch takes it every few seconds to
look at the SIM instead of waiting for the +CMTI URCs.
"""
import argparse
import contextlib
import fcntl
import os
import random
import re
import select
import time

AT_PORT = '/dev/ccci_at'
WATCH_INTERVAL = 5

GSM7 = ('@£$¥èéùìòÇ\nØø\rÅåΔ_ΦΓΛΩΠΨΣΘΞ\x1bÆæßÉ !"#¤%&\'()*+,-./0123456789:;<=>?'
        '¡ABCDEFGHIJKLMNOPQRSTUVWXYZÄÖÑÜ§¿abcdefghijklmnopqrstuvwxyzäöñüà')
GSM7_EXT = {'\f': 0x0a, '^': 0x14, '{': 0x28, '}': 0x29, '\\': 0x2f,
            '[': 0x3c, '~': 0x3d, ']': 0x3e, '|': 0x40, '€': 0x65}
GSM7_EXT_REV = {v: k for k, v in GSM7_EXT.items()}


class At:
    def __init__(self, path=AT_PORT):
        self.fd = os.open(path, os.O_RDWR | os.O_NOCTTY)
        self.buf = b''

    @contextlib.contextmanager
    def locked(self):
        fcntl.flock(self.fd, fcntl.LOCK_EX)
        try:
            # drop what came while others had the port
            while select.select([self.fd], [], [], 0)[0]:
                os.read(self.fd, 4096)
            self.buf = b''
            yield
        finally:
            fcntl.flock(self.fd, fcntl.LOCK_UN)

    def _read(self, timeout):
        r, _, _ = select.select([self.fd], [], [], timeout)
        if r:
            self.buf += os.read(self.fd, 4096)

    def _lines(self, timeout):
        self._read(timeout)
        *lines, self.buf = self.buf.split(b'\r\n')
        return [ln.decode(errors='replace') for ln in lines if ln]

    def cmd(self, c, timeout=30):
        """Send a command, return its response lines without URCs"""
        os.write(self.fd, (c + '\r').encode())
        resp, end = [], time.time() + timeout
        while time.time() < end:
            for ln in self._lines(0.3):
                if ln == c or ln.startswith('+CMTI:') or ln.startswith('+CDS'):
                    continue
                resp.append(ln)
                if ln == 'OK' or 'ERROR' in ln:
                    return resp
        raise TimeoutError(c)

    def cmgs(self, pdu, tpdu_len):
        """AT+CMGS: wait for the '> ' prompt, send the PDU and Ctrl-Z"""
        os.write(self.fd, f'AT+CMGS={tpdu_len}\r'.encode())
        end = time.time() + 10
        while b'> ' not in self.buf:
            if time.time() > end:
                raise TimeoutError('AT+CMGS prompt')
            self._read(0.3)
        self.buf = self.buf[self.buf.index(b'> ') + 2:]
        os.write(self.fd, pdu.encode() + b'\x1a')
        end, ref = time.time() + 60, None
        while time.time() < end:
            for ln in self._lines(0.3):
                if ln.startswith('+CMGS:'):
                    ref = ln
                elif ln == 'OK':
                    return ref
                elif 'ERROR' in ln:
                    raise RuntimeError(ln)
        raise TimeoutError('AT+CMGS')


def septets(text):
    """GSM 7-bit default alphabet septets, None if the text needs UCS2"""
    out = []
    for ch in text:
        if ch in GSM7_EXT:
            out += [0x1b, GSM7_EXT[ch]]
        elif ch in GSM7:
            out.append(GSM7.index(ch))
        else:
            return None
    return out


def pack7(sept, fill):
    """Pack septets into octets after <fill> padding bits"""
    bits, n, out = 0, fill, bytearray()
    for s in sept:
        bits |= s << n
        n += 7
        while n >= 8:
            out.append(bits & 0xff)
            bits >>= 8
            n -= 8
    if n:
        out.append(bits & 0xff)
    return bytes(out)


def unpack7(data, count, fill):
    bits = int.from_bytes(data, 'little') >> fill
    return [(bits >> (7 * i)) & 0x7f for i in range(count)]


def gsm7_text(sept):
    out, esc = [], False
    for s in sept:
        if esc:
            out.append(GSM7_EXT_REV.get(s, ' '))
            esc = False
        elif s == 0x1b:
            esc = True
        else:
            out.append(GSM7[s])
    return ''.join(out)


def bcd(digits):
    digits += 'F' * (len(digits) % 2)
    return bytes(int(digits[i + 1] + digits[i], 16) for i in range(0, len(digits), 2))


def unbcd(data):
    return ''.join(f'{b & 0xf:x}{b >> 4:x}' for b in data).rstrip('f')


def submit_pdus(number, text):
    """SMS-SUBMIT TPDUs, split with a concatenation UDH if needed"""
    toa = 0x91 if number.startswith('+') else 0x81
    digits = number.lstrip('+')
    da = bytes([len(digits), toa]) + bcd(digits)
    sept = septets(text)
    if sept is not None:
        parts = [sept]
        if len(sept) > 160:
            parts, cur = [], []
            for s in sept:
                # do not split an escape sequence
                if len(cur) >= 153 or len(cur) == 152 and s == 0x1b:
                    parts.append(cur)
                    cur = []
                cur.append(s)
            parts.append(cur)
    else:
        u16 = text.encode('utf-16-be')
        units = [u16[i:i + 2] for i in range(0, len(u16), 2)]
        size = 70 if len(units) <= 70 else 67
        parts, cur = [], []
        for u in units:
            # do not split a surrogate pair
            if len(cur) >= size or len(cur) == size - 1 and 0xd8 <= u[0] <= 0xdb:
                parts.append(cur)
                cur = []
            cur.append(u)
        parts.append(cur)
    ref = random.randrange(256)
    for i, part in enumerate(parts):
        udh = b''
        if len(parts) > 1:
            udh = bytes([5, 0, 3, ref, len(parts), i + 1])
        if sept is not None:
            fill = (7 - len(udh) * 8 % 7) % 7
            ud = udh + pack7(part, fill)
            udl = (len(udh) * 8 + fill) // 7 + len(part)
            dcs = 0
        else:
            ud = udh + b''.join(part)
            udl = len(ud)
            dcs = 8
        first = 0x01 | (0x40 if udh else 0)
        yield bytes([first, 0]) + da + bytes([0, dcs, udl]) + ud


def parse_deliver(pdu):
    """SMS-DELIVER PDU (with SMSC) -> dict, None for other types"""
    b = bytes.fromhex(pdu)
    b = b[1 + b[0]:]
    first = b[0]
    if first & 3 != 0:
        return None
    n, toa = b[1], b[2]
    oa_raw = b[3:3 + (n + 1) // 2]
    if toa & 0x70 == 0x50:
        sender = gsm7_text(unpack7(oa_raw, n * 4 // 7, 0))
    else:
        sender = ('+' if toa & 0x70 == 0x10 else '') + unbcd(oa_raw)
    p = 3 + (n + 1) // 2
    dcs = b[p + 1]
    ts = unbcd(b[p + 2:p + 9])
    # quarter hours in swapped BCD, bit 3 is the sign
    tz = int(f'{b[p + 8] & 0xf7:02x}'[::-1]) * (-15 if b[p + 8] & 0x08 else 15)
    when = (f'20{ts[0:2]}-{ts[2:4]}-{ts[4:6]} {ts[6:8]}:{ts[8:10]}:{ts[10:12]}'
            f'{"+" if tz >= 0 else "-"}{abs(tz) // 60:02}:{abs(tz) % 60:02}')
    udl, ud = b[p + 9], b[p + 10:]
    udh, concat = b'', None
    if first & 0x40:
        udh = ud[:1 + ud[0]]
        i = 1
        while i < len(udh):
            iei, iel = udh[i], udh[i + 1]
            v = udh[i + 2:i + 2 + iel]
            if iei == 0:
                concat = (v[0], v[1], v[2])
            elif iei == 8:
                concat = (v[0] << 8 | v[1], v[2], v[3])
            i += 2 + iel
    alphabet = 0
    if dcs & 0xc0 == 0:
        alphabet = (dcs >> 2) & 3
    elif dcs & 0xf0 == 0xf0:
        alphabet = (dcs >> 2) & 1
    if alphabet == 0:
        fill = (7 - len(udh) * 8 % 7) % 7
        hdr = (len(udh) * 8 + fill) // 7
        text = gsm7_text(unpack7(ud[len(udh):], udl - hdr, fill))
    elif alphabet == 2:
        # kept raw: a surrogate pair can be split between two parts
        raw = ud[len(udh):udl]
        text = raw.decode('utf-16-be', errors='replace')
    else:
        text = ud[len(udh):udl].hex()
    return {'from': sender, 'time': when, 'text': text, 'concat': concat,
            'ucs2': raw if alphabet == 2 else None}


def stored(at):
    """Messages on the SIM: [(indexes, message)], concatenated parts joined"""
    resp = at.cmd('AT+CMGL=4')
    msgs = []
    for i, ln in enumerate(resp):
        m = re.match(r'\+CMGL: (\d+),', ln)
        if m and i + 1 < len(resp):
            msg = parse_deliver(resp[i + 1])
            if msg:
                msgs.append((int(m[1]), msg))
    out, groups = [], {}
    for idx, msg in msgs:
        msg['complete'] = True
        if not msg['concat']:
            out.append(([idx], msg))
            continue
        ref, total, seq = msg['concat']
        g = groups.setdefault((msg['from'], ref, total), {})
        g[seq] = (idx, msg)
    for (_, _, total), g in groups.items():
        parts = [g[s] for s in sorted(g)]
        msg = dict(parts[0][1])
        if all(p[1]['ucs2'] is not None for p in parts):
            msg['text'] = b''.join(p[1]['ucs2'] for p in parts).decode(
                'utf-16-be', errors='replace')
        else:
            msg['text'] = ''.join(p[1]['text'] for p in parts)
        if len(g) < total:
            msg['complete'] = False
            msg['text'] += f' [{len(g)}/{total} parts]'
        out.append(([p[0] for p in parts], msg))
    return sorted(out, key=lambda m: m[1]['time'])


def show(indexes, msg):
    print(f'[{",".join(map(str, indexes))}] {msg["time"]} {msg["from"]}: {msg["text"]}',
          flush=True)


def setup(at):
    at.cmd('AT+CMGF=0')
    at.cmd('AT+CPMS="SM","SM","SM"')
    at.cmd('AT+CNMI=2,1,0,0,0')


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    sub = ap.add_subparsers(dest='op', required=True)
    s = sub.add_parser('send')
    s.add_argument('number')
    s.add_argument('text')
    for name in ('list', 'watch'):
        sub.add_parser(name).add_argument('--delete', action='store_true',
                                          help='delete the messages once printed')
    sub.add_parser('delete').add_argument('index', type=int, nargs='+')
    a = ap.parse_args()

    at = At()
    if a.op == 'send':
        with at.locked():
            setup(at)
            for tpdu in submit_pdus(a.number, a.text):
                print(at.cmgs('00' + tpdu.hex().upper(), len(tpdu)), flush=True)
        return
    if a.op == 'delete':
        with at.locked():
            for i in a.index:
                at.cmd(f'AT+CMGD={i}')
        return
    seen = set()
    while True:
        with at.locked():
            setup(at)
            for indexes, msg in stored(at):
                # SIM slots get reused, a message is its slot and time
                key = (indexes[0], msg['time'])
                # watch: wait for the missing parts of a concatenated SMS
                if key in seen or a.op == 'watch' and not msg['complete']:
                    continue
                show(indexes, msg)
                if a.delete:
                    for i in indexes:
                        at.cmd(f'AT+CMGD={i}')
                else:
                    seen.add(key)
        if a.op == 'list':
            break
        time.sleep(WATCH_INTERVAL)


if __name__ == '__main__':
    main()
