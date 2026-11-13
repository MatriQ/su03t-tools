#!/usr/bin/env python3
"""Explicit-port macOS startup diagnostic: trigger only, never firmware or EOT.

Requires confirmed B6<-TXD, B7->RXD, shared GND; DTR/RTS unconnected.
Uses Python stdlib and Darwin ioctls, independent of the Rust transport.
"""
import argparse
import fcntl
import json
import os
import select
import struct
import sys
import termios
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", required=True)
    parser.add_argument("--wait", type=int, default=180)
    parser.add_argument("--trigger-hex", choices=("32", "0A"), default="32")
    args = parser.parse_args()
    trigger = bytes.fromhex(args.trigger_hex)
    if sys.platform != "darwin" or not 1 <= args.wait <= 600:
        parser.error("requires macOS and --wait 1..600")
    # Darwin SDK IOKit/serial/ioss.h: _IOW('T', 2, user_speed_t), uint64_t.
    ios_speed_64 = 0x80085402
    fd = os.open(args.port, os.O_RDWR | os.O_NOCTTY | os.O_NONBLOCK)
    started = None
    tx = rx = 0
    recent = b""
    sample = bytearray()
    previous_write = None
    gaps = []
    outcome = "setup_error"
    try:
        fcntl.ioctl(fd, termios.TIOCEXCL)
        attrs = termios.tcgetattr(fd)
        attrs[0] = attrs[1] = attrs[3] = 0
        attrs[2] = termios.CS8 | termios.CLOCAL | termios.CREAD
        attrs[4] = attrs[5] = termios.B9600
        attrs[6][termios.VMIN] = 0
        attrs[6][termios.VTIME] = 0
        termios.tcsetattr(fd, termios.TCSANOW, attrs)
        fcntl.ioctl(fd, ios_speed_64, struct.pack("=Q", 921600))
        fcntl.ioctl(fd, termios.TIOCMBIS, struct.pack("=I", termios.TIOCM_DTR))
        fcntl.ioctl(fd, termios.TIOCMBIC, struct.pack("=I", termios.TIOCM_RTS))
        checked = termios.tcgetattr(fd)
        prohibited = termios.PARENB | termios.CSTOPB
        for name in ("CRTSCTS", "CDTR_IFLOW", "CDSR_OFLOW", "CCAR_OFLOW"):
            prohibited |= getattr(termios, name, 0)
        if checked[2] & prohibited or checked[2] & termios.CSIZE != termios.CS8:
            raise RuntimeError("8N1/no-flow termios verification failed")
        print(json.dumps({"port": args.port, "baud_request": 921600,
                          "baud_ioctl": hex(ios_speed_64), "cflag": hex(checked[2]),
                          "iflag": hex(checked[0]), "api": "Darwin stdlib native",
                          "trigger_hex": args.trigger_hex,
                          "dtr": True, "rts": False}), flush=True)
        time.sleep(0.5)
        started = time.monotonic()
        deadline = started + args.wait
        next_write = started
        outcome = "timeout"
        print(f"READY unix_ms={time.time_ns() // 1000000}: trigger {args.trigger_hex} only, wait {args.wait}s; power-cycle now", flush=True)
        while time.monotonic() < deadline:
            now = time.monotonic()
            if now >= next_write:
                try:
                    count = os.write(fd, trigger)
                except BlockingIOError:
                    count = 0
                if count:
                    tx += count
                    if previous_write is not None:
                        gaps.append((now - previous_write) * 1000)
                    previous_write = now
                    next_write = now + 0.01
                else:
                    next_write = now + 0.001
            readable, _, _ = select.select([fd], [], [], max(0, min(next_write, deadline) - time.monotonic()))
            if readable:
                try:
                    data = os.read(fd, 4096)
                except BlockingIOError:
                    continue
                if not data:
                    raise RuntimeError("serial EOF/disconnection")
                rx += len(data)
                if len(sample) < 256:
                    sample.extend(data[:256 - len(sample)])
                    print(f"RX +{(time.monotonic()-started)*1000:.3f}ms {data.hex(' ').upper()}", flush=True)
                for byte in data:
                    recent = (recent + bytes([byte]))[-3:]
                    if recent in (b"CCC", b"NNN"):
                        outcome = recent.decode("ascii")
                        break
                if outcome in ("CCC", "NNN"):
                    break
    except KeyboardInterrupt:
        outcome = "cancelled"
    except Exception:
        outcome = "error"
        raise
    finally:
        os.close(fd)
        print(json.dumps({"outcome": outcome, "tx_trigger_bytes": tx, "trigger_hex": args.trigger_hex, "rx_bytes": rx,
                          "rx_first256_hex": sample.hex(" "),
                          "elapsed_s": None if started is None else time.monotonic()-started,
                          "trigger_gap_ms_mean": sum(gaps)/len(gaps) if gaps else None,
                          "trigger_gap_ms_max": max(gaps) if gaps else None,
                          "firmware_bytes_sent": 0, "port_closed": True}), flush=True)


if __name__ == "__main__":
    main()
