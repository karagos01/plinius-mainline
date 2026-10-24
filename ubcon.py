#!/usr/bin/env python3
"""Run commands on the U-Boot serial console and print the output.

usage: ubcon.py [-t timeout] 'cmd1' ['cmd2' ...]
"""
import argparse
import serial
import time

PROMPT = b"=> "

p = argparse.ArgumentParser()
p.add_argument("-d", "--dev", default="/dev/ttyUSB0")
p.add_argument("-t", "--timeout", type=float, default=10)
p.add_argument("cmds", nargs="+")
a = p.parse_args()

s = serial.Serial(a.dev, 921600, timeout=0.2)
# Drain whatever is still being printed before sending anything
s.write(b"\r\n")
while s.read(4096):
    pass
for c in a.cmds:
    s.write(c.encode() + b"\r\n")
    buf = b""
    end = time.time() + a.timeout
    while time.time() < end:
        buf += s.read(4096)
        if buf.endswith(PROMPT):
            break
    print(buf.decode(errors="replace").replace("\r", ""), end="", flush=True)
print()
