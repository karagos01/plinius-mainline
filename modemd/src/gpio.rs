//! Read one GPIO line through the GPIO character device (uAPI v2), the way
//! libgpiod's request_lines() with default settings does.

use std::fs::File;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

#[repr(C)]
struct LineConfigAttribute {
    id: u32,
    padding: u32,
    value: u64,
    mask: u64,
}

#[repr(C)]
struct LineConfig {
    flags: u64,
    num_attrs: u32,
    padding: [u32; 5],
    attrs: [LineConfigAttribute; 10],
}

#[repr(C)]
struct LineRequest {
    offsets: [u32; 64],
    consumer: [u8; 32],
    config: LineConfig,
    num_lines: u32,
    event_buffer_size: u32,
    padding: [u32; 5],
    fd: i32,
}

#[repr(C)]
struct LineValues {
    bits: u64,
    mask: u64,
}

const fn iowr(nr: u64, size: usize) -> u64 {
    (3 << 30) | ((size as u64) << 16) | (0xb4 << 8) | nr
}
const GPIO_V2_GET_LINE_IOCTL: u64 = iowr(0x07, std::mem::size_of::<LineRequest>());
const GPIO_V2_LINE_GET_VALUES_IOCTL: u64 = iowr(0x0e, std::mem::size_of::<LineValues>());

/// Value of `line` on /dev/gpiochip0, left in its current direction
pub fn value(line: u32) -> Option<u32> {
    let chip = File::open("/dev/gpiochip0").ok()?;
    // SAFETY: plain-old-data uAPI structures, all-zero is valid
    let mut req: LineRequest = unsafe { std::mem::zeroed() };
    req.offsets[0] = line;
    req.consumer[..4].copy_from_slice(b"ccci");
    req.num_lines = 1;
    // SAFETY: ioctl with a correctly sized v2 line request
    if unsafe { libc::ioctl(chip.as_raw_fd(), GPIO_V2_GET_LINE_IOCTL as _, &mut req) } < 0 {
        return None;
    }
    // SAFETY: the kernel returned a new line file descriptor we now own
    let line_fd = unsafe { OwnedFd::from_raw_fd(req.fd) };
    let mut vals = LineValues { bits: 0, mask: 1 };
    // SAFETY: ioctl with a correctly sized v2 line values structure
    if unsafe { libc::ioctl(line_fd.as_raw_fd(), GPIO_V2_LINE_GET_VALUES_IOCTL as _, &mut vals) } < 0 {
        return None;
    }
    Some((vals.bits & 1) as u32)
}

#[cfg(test)]
mod tests {
    #[test]
    fn uapi_sizes() {
        assert_eq!(std::mem::size_of::<super::LineRequest>(), 592);
        assert_eq!(super::GPIO_V2_GET_LINE_IOCTL, 0xc250_b407);
        assert_eq!(super::GPIO_V2_LINE_GET_VALUES_IOCTL, 0xc010_b40e);
    }
}
