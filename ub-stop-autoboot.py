#!/usr/bin/env python3
"""Catch the U-Boot autoboot countdown on the UART and start fastboot.

usage: ub-stop-autoboot.py [-t timeout] [cmd]   (cmd defaults to "run fastboot")
"""
import argparse
import serial
import sys
import time

p = argparse.ArgumentParser()
p.add_argument("-d", "--dev", default="/dev/ttyUSB0")
p.add_argument("-t", "--timeout", type=float, default=60)
p.add_argument("cmd", nargs="?", default="run fastboot")
a = p.parse_args()

s = serial.Serial(a.dev, 921600, timeout=0.05)
buf = b""
end = time.time() + a.timeout
while time.time() < end:
    buf = (buf + s.read(4096))[-4096:]
    if b"Hit any key" in buf or buf.endswith(b"=> "):
        # Ctrl-C stops the countdown as well as a running command
        s.write(b"\x03")
        time.sleep(0.2)
        s.write(b"\r\n" + a.cmd.encode() + b"\r\n")
        print("u-boot: autoboot stopped, ran", repr(a.cmd))
        sys.exit(0)
print("u-boot: no autoboot prompt seen", file=sys.stderr)
sys.exit(1)
