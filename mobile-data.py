#!/usr/bin/env python3
"""Bring up mobile data on the MT6878 modem (what the stock RIL and
NetAgent do, reduced to the minimum).

Needs the modem ready (ccci-server.py --radio started the SIM). Steps:
unlock the SIM (only with all three PIN tries left), turn the radio on
over MIPC, set the initial attach APN (else no LTE attach, 2G only),
wait for the network, activate PDP context 1 (on LTE the default bearer
of the attach, context 0) and configure the ccmni interface the modem
reports in its +EIF URC:

    +EIF: <id>, ifup, <type>, <ipv4>, <ipv6 as hex bytes>, 0

The interface is ccmni<id % 100>. NetAgent has to confirm it with
AT+EIF=<id>,"ifst","up",<type>, before that the modem keeps the data path
of the PDN closed. IPv6 is configured by the router advertisement.

The PIN and the APN can also come from the environment (SIM_PIN, APN),
as the systemd unit passes them from /etc/default/plinius-modem.
"""
import argparse
import fcntl
import os
import re
import select
import subprocess
import sys
import time

AT_PORT = '/dev/ccci_at'
MIPC_PORT = '/dev/ccci_mipc1'
MIPC_OPEN = bytes.fromhex(
    '8419542400000000ff000103010028000001040002000000'
    '01010f002f6465762f747479434d495043310000000000000201010001000000')
MIPC_RADIO_ON = bytes.fromhex('841954240000000001000304020108000001010001000000')


def log(*a):
    print(time.strftime('%H:%M:%S'), *a, flush=True)


class At:
    def __init__(self, path):
        self.fd = os.open(path, os.O_RDWR | os.O_NOCTTY)
        self.urcs = []
        self.buf = b''

    def _lines(self, timeout):
        r, _, _ = select.select([self.fd], [], [], timeout)
        if r:
            self.buf += os.read(self.fd, 4096)
        *lines, self.buf = self.buf.split(b'\r\n')
        return [ln.decode(errors='replace') for ln in lines if ln]

    def cmd(self, c, timeout=30):
        """Send a command, return its response lines; keep the URCs"""
        os.write(self.fd, (c + '\r').encode())
        resp, end = [], time.time() + timeout
        while time.time() < end:
            for ln in self._lines(0.3):
                if ln == c:
                    continue
                if ln.startswith('+EIF:') or ln.startswith('+CEREG:') and ',' not in ln[8:]:
                    self.urcs.append(ln)
                    continue
                resp.append(ln)
                if ln == 'OK' or 'ERROR' in ln:
                    return resp
        raise TimeoutError(c)

    def wait_urc(self, pattern, timeout):
        end = time.time() + timeout
        while time.time() < end:
            for i, u in enumerate(self.urcs):
                m = re.match(pattern, u)
                if m:
                    del self.urcs[:i + 1]
                    return m
            self.urcs += [ln for ln in self._lines(0.5)]
        return None


def query(at, c, prefix):
    for ln in at.cmd(c):
        if ln.startswith(prefix):
            return ln[len(prefix):].strip()
    return None


def unlock(at, pin):
    st = query(at, 'AT+CPIN?', '+CPIN:')
    log('SIM:', st)
    if st == 'READY':
        return True
    if st != 'SIM PIN':
        return False
    tries = query(at, 'AT+EPINC', '+EPINC:')
    if not pin or not tries or tries.split(',')[0].strip() != '3':
        log(f'not entering the PIN (tries {tries})')
        return False
    return at.cmd(f'AT+CPIN="{pin}"')[-1] == 'OK'


def lte(at):
    """Registered on LTE (or NR): the access technology of +COPS is 7+"""
    cops = query(at, 'AT+COPS?', '+COPS:')
    return bool(cops) and cops.split(',')[-1].isdigit() and int(cops.split(',')[-1]) >= 7


def radio_on():
    fd = os.open(MIPC_PORT, os.O_RDWR)
    try:
        for msg in (MIPC_OPEN, MIPC_RADIO_ON):
            os.write(fd, msg)
            r, _, _ = select.select([fd], [], [], 2)
            if r:
                os.read(fd, 65536)
    finally:
        os.close(fd)


def ip(*args):
    subprocess.run(['ip', *args], check=False)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    ap.add_argument('--pin', default=os.environ.get('SIM_PIN'),
                    help='SIM PIN, entered only with 3 tries left (env SIM_PIN)')
    ap.add_argument('--apn', default=os.environ.get('APN') or 'internet')
    ap.add_argument('--default-route', action='store_true',
                    help='add a default route over the modem (metric 2048)')
    a = ap.parse_args()

    at = At(AT_PORT)
    # The single AT channel is shared with sms.py: keep it to ourselves
    fcntl.flock(at.fd, fcntl.LOCK_EX)
    if not unlock(at, a.pin):
        sys.exit(1)
    radio_on()
    # Without an initial attach APN the modem does not attach to LTE and
    # stays on 2G. Both settings are lost when the radio turns on, so set
    # them after that; EGTYPE 4 attaches for packet service right away.
    at.cmd(f'AT+EIAAPN="{a.apn}",0')
    at.cmd('AT+EGTYPE=4')
    at.cmd('AT+EIFCONFIG=3')
    # Store incoming SMS on the SIM and report +CMTI (sms.py reads them).
    # The modem default 2,2 routes them as +CMT, which needs AT+CNMA acks.
    at.cmd('AT+CNMI=2,1,0,0,0')
    for _ in range(60):
        reg = query(at, 'AT+CEREG?', '+CEREG:')
        if reg and reg.split(',')[1] in ('1', '5'):
            break
        time.sleep(2)
    else:
        log('not registered:', reg)
        sys.exit(1)
    # Registered on 2G anyway: the APN again makes it reselect LTE
    if not lte(at):
        at.cmd(f'AT+EIAAPN="{a.apn}",0')
        for _ in range(15):
            time.sleep(2)
            if lte(at):
                break
    log('registered:', query(at, 'AT+COPS?', '+COPS:'))

    at.cmd(f'AT+CGDCONT=1,"IPV4V6","{a.apn}"')
    cid = 1
    if at.cmd('AT+CGACT=1,1', timeout=60)[-1] != 'OK':
        # On LTE the attach set up the default bearer to the same APN as
        # context 0, a second one is refused: use that one
        cid = 0
        if at.cmd('AT+CGACT=1,0', timeout=60)[-1] != 'OK':
            log('PDP activation failed')
            sys.exit(1)
    m = at.wait_urc(r'\+EIF: (\d+), ifup, (\d+), ([\d.]*)', 20)
    if not m:
        log('no +EIF ifup from the modem')
        sys.exit(1)
    pid, atype, v4 = int(m[1]), int(m[2]), m[3]
    dev = f'ccmni{pid % 100}'
    at.cmd(f'AT+EIF={pid}, "ifst", "up", {atype}')

    ip('link', 'set', dev, 'up')
    if v4:
        ip('addr', 'add', f'{v4}/32', 'dev', dev)
        if a.default_route:
            ip('route', 'add', 'default', 'dev', dev, 'metric', '2048')
    dns = [d for ln in at.cmd(f'AT+CGCONTRDP={cid}') if ln.startswith('+CGCONTRDP:')
           for d in ln.split(',')[5:7]]
    log(f'{dev} up: {v4}, dns', ' '.join(d.strip('"') for d in dns))


if __name__ == '__main__':
    main()
