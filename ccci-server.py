#!/usr/bin/env python3
"""Userspace side of the MT6878 modem: RPC and file system servers.

The kernel driver (mtk-ccci) exports the modem's logical channels as
character devices. Packets on /dev/ccci_fs and /dev/ccci_rpc keep their
16 byte CCCI header:

    u32 data0      FS: bit 31 = more fragments follow
    u32 len        packet length including the header
    u16 channel
    u16 seq
    u32 reserved   buffer slot of the request, echoed in the reply

followed by a request: u32 op, u32 count, count x {u32 len, data, pad to 4}.
Replies carry op | 0xffff0000 and the result parameters.

The modem reads its NVRAM through the FS channel between the two boot
handshakes, so both servers have to run before the modem is started; this
program starts it once the devices are open.
"""
import argparse
import mmap
import os
import select
import struct
import sys
import threading
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ccci_fs import FsServer  # noqa: E402

STATE = '/sys/bus/platform/drivers/mtk-ccci/10209000.modem/state'
HDR = struct.Struct('<IIHHI')
FS_MORE = 1 << 31
FS_WAKEUP = 1 << 30
FS_FRAG_DATA = 0xd80
RESP = 0xffff0000

verbose = False


def log(*a):
    print(time.strftime('%H:%M:%S'), *a, flush=True)


def parse_params(buf, off=8):
    op, n = struct.unpack_from('<II', buf, 0)
    args = []
    for _ in range(n):
        ln, = struct.unpack_from('<I', buf, off)
        args.append(bytes(buf[off + 4:off + 4 + ln]))
        off += 4 + ((ln + 3) & ~3)
    return op, args


def build_params(op, params):
    out = bytearray(struct.pack('<II', op, len(params)))
    for p in params:
        out += struct.pack('<I', len(p)) + p + bytes(-len(p) & 3)
    return bytes(out)


# RPC

RPC_GPIO_ADC = 0x4009
GPIO_MAX = 10
GPIO_NAME_LEN = 34
ADC_NAME_LEN = 33
RPC_REQ_GPIO_PIN = 1 << 0
RPC_REQ_GPIO_VALUE = 1 << 1

# MD GPIO name -> (AP GPIO, MD view number), from the stock device tree
MD_GPIOS = {
    'GPIO_FDD_Band_Support_Detection_1': (114, 114),
    'GPIO_FDD_Band_Support_Detection_2': (9, 9),
}


def gpio_value(gpio):
    try:
        import gpiod
        with gpiod.request_lines('/dev/gpiochip0', consumer='ccci',
                                 config={gpio: None}) as req:
            return int(req.get_value(gpio).value)
    except Exception:
        return 0


def rpc_gpio_adc(args):
    req = args[0]
    v2 = len(req) == 4 + GPIO_MAX * (GPIO_NAME_LEN + 4) + ADC_NAME_LEN + 8
    if not v2:
        log('rpc: unexpected GPIO_ADC request size', len(req))
        return [struct.pack('<i', -1), struct.pack('<i', -1)]
    mask, valid = struct.unpack_from('<HH', req, 0)
    nums = [0x0f0f0f0f] * GPIO_MAX
    vals = [0x0f0f0f0f] * GPIO_MAX
    for i in range(GPIO_MAX):
        if not valid & (1 << i):
            continue
        off = 4 + i * GPIO_NAME_LEN
        name = req[off:off + GPIO_NAME_LEN].split(b'\0')[0].decode()
        if name not in MD_GPIOS:
            continue
        ap, md = MD_GPIOS[name]
        if mask & RPC_REQ_GPIO_PIN:
            nums[i] = md
        if mask & RPC_REQ_GPIO_VALUE:
            vals[i] = gpio_value(ap)
        log(f'rpc: {name} = GPIO{ap}: {vals[i]}')
    out = struct.pack(f'<{GPIO_MAX}I{GPIO_MAX}III', *nums, *vals,
                      0x0f0f0f0f, 0x0f0f0f0f)
    return [struct.pack('<I', 0), out]


# MD GPIO numbers by name, from the stock "mediatek,gpio_usage_mapping" node
RPC_GET_GPIO_NUM = 0x4002
MD_GPIO_NUMS = {
    'GPIO_SIM1_HOT_PLUG': 46, 'GPIO_SIM2_HOT_PLUG': 47,
    'GPIO_SIM1_SCLK': 83, 'GPIO_SIM1_SRST': 84, 'GPIO_SIM1_SIO': 85,
    'GPIO_SIM2_SCLK': 86, 'GPIO_SIM2_SRST': 87, 'GPIO_SIM2_SIO': 88,
    'GPIO_FDD_BAND_SUPPORT_DETECT_1ST_PIN': 114,
    'GPIO_FDD_BAND_SUPPORT_DETECT_2ND_PIN': 9,
}
RPC_ERR_NOT_FOUND = -5


def rpc_get_gpio_num(args):
    name = args[0].split(b'\0')[0].decode(errors='replace')
    num = MD_GPIO_NUMS.get(name)
    log(f'rpc: gpio {name} = {num}')
    if num is None:
        err = struct.pack('<i', RPC_ERR_NOT_FOUND)
        return [err, err]
    return [struct.pack('<i', 0), struct.pack('<i', num)]


# EINT attributes of the SIM hot plug detection, from the stock
# MD1-SIMx-HOT-PLUG-EINT nodes: number, debounce (ms), polarity,
# sensitivity, socket type, dedicated, source pin
RPC_GET_EINT_ATTR = 0x4005
EINT_ATTR_MAX = 7
RPC_ERR_EINT_NOT_FOUND = -11
MD_EINTS = {
    'MD1_SIM1_HOT_PLUG_EINT': (0, 10, 0, 0, 0, 0, 1),
    'MD1_SIM2_HOT_PLUG_EINT': (1, 10, 0, 0, 0, 0, 2),
}


def rpc_get_eint_attr(args):
    name = args[0].split(b'\0')[0].decode(errors='replace')
    attr, = struct.unpack_from('<I', args[2])
    vals = MD_EINTS.get(name.replace('-', '_'))
    log(f'rpc: eint {name} attr {attr} = {vals[attr] if vals and attr < EINT_ATTR_MAX else None}')
    if vals is None or attr >= EINT_ATTR_MAX:
        err = struct.pack('<i', RPC_ERR_EINT_NOT_FOUND)
        return [err, err]
    return [struct.pack('<i', 0), struct.pack('<i', vals[attr])]


# DT value query: like the stock kernel, report "not set"
RPC_DTSI_QUERY = 0x400e
DTSI_NOT_SET = struct.pack('<I', 0x0f0f0f0f) * 17

RPC_QUERY_0x4016 = 0x4016
RPC_LHIF_MAPPING = 0x400d


# AMMS DRDI: copy the RF parameter sets of this board variant from the
# md1drdi image of the modem partition to the DRDI shared memory
RPC_AMMS_DRDI = 0x4014
DRDI_INIT = 1
DRDI_COPY = 2
DRDI_VERSION = 3
DRDI_SET_MAX = 16
DRDI_LEN_MAX = 0x10000
DRDI_SMEM_SIZE = 0x10000
MD_IMG_MAGIC = 0x58881688
MD_IMG_HDR = 0x200


def find_md_image(part, name):
    """Offset and size of a sub-image in the modem partition"""
    with open(part, 'rb') as f:
        off = 0
        while True:
            f.seek(off)
            hdr = f.read(MD_IMG_HDR)
            if len(hdr) < MD_IMG_HDR:
                return None
            magic, size = struct.unpack_from('<II', hdr)
            if magic != MD_IMG_MAGIC:
                return None
            if hdr[8:40].split(b'\0')[0] == name:
                return off + MD_IMG_HDR, size
            off = (off + MD_IMG_HDR + size + 15) & ~15


class Drdi:
    def __init__(self, rpc_fd, part):
        self.part = part
        self.img = find_md_image(part, b'md1drdi')
        self.smem = mmap.mmap(rpc_fd, DRDI_SMEM_SIZE)
        self.nsets = 0
        self.copied = False
        log(f'drdi: md1drdi at {self.img[0]:#x}, size {self.img[1]:#x}'
            if self.img else 'drdi: no md1drdi image')

    def handle(self, req):
        cmd, seq, ver, n = req[0], req[1], req[4], req[5]
        out = bytearray(8)
        out[1] = seq
        if not self.img:
            out[0] = 0xff
            return -1, out
        img_off, img_size = self.img
        if cmd == DRDI_INIT:
            out[4] = DRDI_VERSION
            if ver != DRDI_VERSION or n >= DRDI_SET_MAX:
                out[0] = out[6] = 0xff
                return -1, out
            for i in range(n):
                start, size = struct.unpack_from('<II', req, 8 + i * 8)
                if start + size > img_size:
                    out[0], out[6] = 0xfe, 0xff
                    return -1, out
            self.nsets = n
            self.copied = False
            return 0, out
        if cmd == DRDI_COPY:
            if self.copied:
                return 0, out
            sets = [struct.unpack_from('<III', req, 8 + i * 12)
                    for i in range(self.nsets)]
            if any(ln and (ln > DRDI_LEN_MAX or src + ln > img_size or
                           dst + ln > DRDI_SMEM_SIZE)
                   for src, dst, ln in sets):
                out[0] = 0xff
                return -1, out
            with open(self.part, 'rb') as f:
                for src, dst, ln in sets:
                    if ln:
                        f.seek(img_off + src)
                        self.smem[dst:dst + ln] = f.read(ln)
                        log(f'drdi: set {src:#x} -> {dst:#x}, {ln:#x}')
            last = req[4] == 1
            self.copied = last
            out[4] = 0 if last else 0xff
            return 0, out
        out[0] = 0xff
        return -1, out


drdi = None


def handle_rpc(op, args):
    if op == RPC_GPIO_ADC:
        return rpc_gpio_adc(args)
    if op == RPC_GET_GPIO_NUM and len(args) >= 2:
        return rpc_get_gpio_num(args)
    if op == RPC_GET_EINT_ATTR and len(args) >= 3:
        return rpc_get_eint_attr(args)
    if op == RPC_DTSI_QUERY:
        log('rpc: dtsi query', args[0][2:].split(b'\0')[0].decode(errors='replace'))
        return [struct.pack('<i', 0), DTSI_NOT_SET]
    if op == RPC_LHIF_MAPPING and len(args) == 1 and len(args[0]) >= 8:
        net_if, lhif_q = struct.unpack_from('<II', args[0])
        log(f'rpc: lhif mapping ccmni{net_if} -> queue {lhif_q}')
        return [struct.pack('<i', 0), struct.pack('<i', 0)]
    if op == RPC_QUERY_0x4016:
        return [struct.pack('<i', 0), struct.pack('<i', 0)]
    if op == RPC_AMMS_DRDI and drdi and len(args[0]) == 0xbc:
        ret, out = drdi.handle(args[0])
        if verbose:
            log(f'drdi: req {args[0][:8].hex()} -> {ret} {bytes(out).hex()}')
        return [struct.pack('<i', ret), bytes(out)]
    log(f'rpc: unhandled op {op:#x}', [a.hex() for a in args])
    return [struct.pack('<i', -1)]


class Channel:
    def __init__(self, path):
        self.fd = os.open(path, os.O_RDWR)
        self.name = os.path.basename(path)

    def read(self):
        return os.read(self.fd, 0x10000)

    def write(self, pkt):
        os.write(self.fd, pkt)


def serve_rpc(ch, pkt):
    d0, ln, chan, seq, slot = HDR.unpack_from(pkt)
    op, args = parse_params(pkt[16:])
    if verbose:
        log(f'rpc: op {op:#x} slot {slot}')
    reply = build_params(op | RESP, handle_rpc(op, args))
    ch.write(HDR.pack(d0, 0, 0, 0, slot) + reply)
    if verbose:
        log(f'rpc: replied {op:#x}')


class FsChannel(Channel):
    def __init__(self, path, server):
        super().__init__(path)
        self.server = server
        self.partial = {}

    def serve(self, pkt):
        d0, ln, chan, seq, slot = HDR.unpack_from(pkt)
        body = pkt[16:ln]
        if slot in self.partial:
            # continuation: 4 byte copy of the op id, then data
            self.partial[slot] += body[4:]
        else:
            self.partial[slot] = bytearray(body)
        if d0 & FS_MORE:
            return
        req = self.partial.pop(slot)
        op, args = parse_params(req)
        try:
            params = self.server.handle(op, args)
        except Exception as e:  # keep serving the modem
            log(f'fs: op {op:#x} failed: {e!r}')
            params = [struct.pack('<i', -2)]
        if verbose:
            log(f'fs: op {op:#x} -> {[p[:8].hex() for p in params]}')
        self.reply(op | RESP, params, slot)

    def reply(self, op, params, slot):
        stream = build_params(op, params)
        rest = stream[4:]
        opb = stream[:4]
        chunks = [rest[i:i + FS_FRAG_DATA]
                  for i in range(0, len(rest), FS_FRAG_DATA)] or [b'']
        for i, c in enumerate(chunks):
            more = FS_MORE if i < len(chunks) - 1 else 0
            self.write(HDR.pack(more, 0, 0, 0, slot) + opb + c)


# MIPC, as the stock RIL talks to the modem on ttyCMIPC1: the messages are
# the ones the stock RIL sends (session open, NW_SET_RADIO_STATE_REQ of SIM1)
MIPC_PORT = '/dev/ccci_mipc1'
MIPC_OPEN = bytes.fromhex(
    '8419542400000000ff000103010028000001040002000000'
    '01010f002f6465762f747479434d495043310000000000000201010001000000')
MIPC_RADIO_STATE = '84195424000000000100030402010800000101000{}000000'


def mipc_request(fd, msg):
    os.write(fd, msg)
    r, _, _ = select.select([fd], [], [], 2)
    return os.read(fd, 65536) if r else None


def mipc_radio_state(on):
    """Set the radio state of SIM1; the first call also starts the SIM"""
    fd = os.open(MIPC_PORT, os.O_RDWR)
    try:
        mipc_request(fd, MIPC_OPEN)
        cnf = mipc_request(fd, bytes.fromhex(MIPC_RADIO_STATE.format(int(on))))
        log(f'mipc: radio state {int(on)}:', cnf.hex() if cnf else 'no reply')
    finally:
        os.close(fd)


def start_modem(radio):
    log('starting the modem')
    fd = os.open(STATE, os.O_WRONLY)
    try:
        os.write(fd, b'start')
        log('modem ready')
    except OSError as e:
        log('modem start failed:', e)
        return
    finally:
        os.close(fd)
    if radio:
        # SIM PIN entry stays with the user (AT+CPIN), then radio state 1
        mipc_radio_state(False)


def main():
    global verbose
    ap = argparse.ArgumentParser()
    ap.add_argument('--root', default='/var/lib/modem')
    ap.add_argument('--modem-part', default='/dev/disk/by-partlabel/modem_a')
    ap.add_argument('--no-start', action='store_true')
    ap.add_argument('--radio', action='store_true',
                    help='start the SIM over MIPC once the modem is ready')
    ap.add_argument('-v', '--verbose', action='store_true')
    a = ap.parse_args()
    verbose = a.verbose

    fs = FsChannel('/dev/ccci_fs', FsServer(a.root))
    rpc = Channel('/dev/ccci_rpc')
    global drdi
    drdi = Drdi(rpc.fd, a.modem_part)
    if not a.no_start:
        threading.Thread(target=start_modem, args=(a.radio,), daemon=True).start()

    while True:
        r, _, _ = select.select([fs.fd, rpc.fd], [], [])
        if fs.fd in r:
            fs.serve(fs.read())
        if rpc.fd in r:
            serve_rpc(rpc, rpc.read())


if __name__ == '__main__':
    main()
