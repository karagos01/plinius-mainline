//! RPC requests of the modem (/dev/ccci_rpc): board facts the stock kernel
//! reads from the device tree, and the AMMS DRDI copy of the RF parameter
//! sets of this board variant.

use crate::wire::{i32p, u32_at, u32p};
use crate::{log, verbose};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::fd::RawFd;

const RPC_GET_GPIO_NUM: u32 = 0x4002;
const RPC_GET_EINT_ATTR: u32 = 0x4005;
const RPC_GPIO_ADC: u32 = 0x4009;
const RPC_LHIF_MAPPING: u32 = 0x400d;
const RPC_DTSI_QUERY: u32 = 0x400e;
const RPC_AMMS_DRDI: u32 = 0x4014;
const RPC_QUERY_0X4016: u32 = 0x4016;

const GPIO_MAX: usize = 10;
const GPIO_NAME_LEN: usize = 34;
const ADC_NAME_LEN: usize = 33;
const RPC_REQ_GPIO_PIN: u16 = 1 << 0;
const RPC_REQ_GPIO_VALUE: u16 = 1 << 1;
const UNSET: u32 = 0x0f0f_0f0f;

/// MD GPIO name -> (AP GPIO, MD view number), from the stock device tree
const MD_GPIOS: &[(&str, u32, u32)] = &[
    ("GPIO_FDD_Band_Support_Detection_1", 114, 114),
    ("GPIO_FDD_Band_Support_Detection_2", 9, 9),
];

/// MD GPIO numbers by name, from the stock "mediatek,gpio_usage_mapping" node
const MD_GPIO_NUMS: &[(&str, i32)] = &[
    ("GPIO_SIM1_HOT_PLUG", 46),
    ("GPIO_SIM2_HOT_PLUG", 47),
    ("GPIO_SIM1_SCLK", 83),
    ("GPIO_SIM1_SRST", 84),
    ("GPIO_SIM1_SIO", 85),
    ("GPIO_SIM2_SCLK", 86),
    ("GPIO_SIM2_SRST", 87),
    ("GPIO_SIM2_SIO", 88),
    ("GPIO_FDD_BAND_SUPPORT_DETECT_1ST_PIN", 114),
    ("GPIO_FDD_BAND_SUPPORT_DETECT_2ND_PIN", 9),
];
const RPC_ERR_NOT_FOUND: i32 = -5;

/// EINT attributes of the SIM hot plug detection, from the stock
/// MD1-SIMx-HOT-PLUG-EINT nodes: number, debounce (ms), polarity,
/// sensitivity, socket type, dedicated, source pin
const MD_EINTS: &[(&str, [i32; 7])] = &[
    ("MD1_SIM1_HOT_PLUG_EINT", [0, 10, 0, 0, 0, 0, 1]),
    ("MD1_SIM2_HOT_PLUG_EINT", [1, 10, 0, 0, 0, 0, 2]),
];
const RPC_ERR_EINT_NOT_FOUND: i32 = -11;

fn cstr(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

fn rpc_gpio_adc(args: &[Vec<u8>]) -> Vec<Vec<u8>> {
    let Some(req) = args.first() else {
        return vec![i32p(-1), i32p(-1)];
    };
    if req.len() != 4 + GPIO_MAX * (GPIO_NAME_LEN + 4) + ADC_NAME_LEN + 8 {
        log(&format!("rpc: unexpected GPIO_ADC request size {}", req.len()));
        return vec![i32p(-1), i32p(-1)];
    }
    let mask = u16::from_le_bytes([req[0], req[1]]);
    let valid = u16::from_le_bytes([req[2], req[3]]);
    let mut nums = [UNSET; GPIO_MAX];
    let mut vals = [UNSET; GPIO_MAX];
    for i in 0..GPIO_MAX {
        if valid & (1 << i) == 0 {
            continue;
        }
        let off = 4 + i * GPIO_NAME_LEN;
        let name = cstr(&req[off..off + GPIO_NAME_LEN]);
        let Some(&(_, ap, md)) = MD_GPIOS.iter().find(|g| g.0 == name) else {
            continue;
        };
        if mask & RPC_REQ_GPIO_PIN != 0 {
            nums[i] = md;
        }
        if mask & RPC_REQ_GPIO_VALUE != 0 {
            vals[i] = crate::gpio::value(ap).unwrap_or(0);
        }
        log(&format!("rpc: {name} = GPIO{ap}: {}", vals[i]));
    }
    let mut out = Vec::new();
    for v in nums.iter().chain(vals.iter()).chain([UNSET, UNSET].iter()) {
        out.extend_from_slice(&v.to_le_bytes());
    }
    vec![u32p(0), out]
}

fn rpc_get_gpio_num(args: &[Vec<u8>]) -> Vec<Vec<u8>> {
    let name = cstr(&args[0]);
    let num = MD_GPIO_NUMS.iter().find(|g| g.0 == name).map(|g| g.1);
    log(&format!("rpc: gpio {name} = {}", opt(num)));
    match num {
        Some(n) => vec![i32p(0), i32p(n)],
        None => vec![i32p(RPC_ERR_NOT_FOUND), i32p(RPC_ERR_NOT_FOUND)],
    }
}

fn rpc_get_eint_attr(args: &[Vec<u8>]) -> Vec<Vec<u8>> {
    let name = cstr(&args[0]);
    let attr = u32_at(&args[2], 0).unwrap_or(u32::MAX) as usize;
    let key = name.replace('-', "_");
    let val = MD_EINTS
        .iter()
        .find(|e| e.0 == key)
        .and_then(|e| e.1.get(attr).copied());
    log(&format!("rpc: eint {name} attr {attr} = {}", opt(val)));
    match val {
        Some(v) => vec![i32p(0), i32p(v)],
        None => vec![i32p(RPC_ERR_EINT_NOT_FOUND), i32p(RPC_ERR_EINT_NOT_FOUND)],
    }
}

// AMMS DRDI: copy the RF parameter sets of this board variant from the
// md1drdi image of the modem partition to the DRDI shared memory

const DRDI_INIT: u8 = 1;
const DRDI_COPY: u8 = 2;
const DRDI_VERSION: u8 = 3;
const DRDI_SET_MAX: u8 = 16;
const DRDI_LEN_MAX: u32 = 0x10000;
const DRDI_SMEM_SIZE: usize = 0x10000;
const MD_IMG_MAGIC: u32 = 0x5888_1688;
const MD_IMG_HDR: u64 = 0x200;

/// Offset and size of a sub-image in the modem partition
fn find_md_image(part: &str, name: &[u8]) -> Option<(u64, u32)> {
    let mut f = File::open(part).ok()?;
    let mut off = 0u64;
    loop {
        let mut hdr = [0u8; MD_IMG_HDR as usize];
        f.seek(SeekFrom::Start(off)).ok()?;
        f.read_exact(&mut hdr).ok()?;
        if u32_at(&hdr, 0)? != MD_IMG_MAGIC {
            return None;
        }
        let size = u32_at(&hdr, 4)?;
        let n = &hdr[8..40];
        let end = n.iter().position(|&c| c == 0).unwrap_or(n.len());
        if &n[..end] == name {
            return Some((off + MD_IMG_HDR, size));
        }
        off = (off + MD_IMG_HDR + size as u64 + 15) & !15;
    }
}

pub struct Drdi {
    part: String,
    img: Option<(u64, u32)>,
    smem: *mut u8,
    nsets: usize,
    copied: bool,
}

impl Drdi {
    pub fn new(rpc_fd: RawFd, part: &str) -> std::io::Result<Self> {
        // SAFETY: maps the DRDI shared memory the driver exports on the fd
        let smem = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                DRDI_SMEM_SIZE,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                rpc_fd,
                0,
            )
        };
        if smem == libc::MAP_FAILED {
            return Err(std::io::Error::last_os_error());
        }
        let img = find_md_image(part, b"md1drdi");
        match img {
            Some((o, s)) => log(&format!("drdi: md1drdi at {o:#x}, size {s:#x}")),
            None => log("drdi: no md1drdi image"),
        }
        Ok(Drdi { part: part.into(), img, smem: smem.cast(), nsets: 0, copied: false })
    }

    fn handle(&mut self, req: &[u8]) -> (i32, [u8; 8]) {
        let (cmd, seq, ver, n) = (req[0], req[1], req[4], req[5]);
        let mut out = [0u8; 8];
        out[1] = seq;
        let Some((img_off, img_size)) = self.img else {
            out[0] = 0xff;
            return (-1, out);
        };
        match cmd {
            DRDI_INIT => {
                out[4] = DRDI_VERSION;
                if ver != DRDI_VERSION || n >= DRDI_SET_MAX {
                    out[0] = 0xff;
                    out[6] = 0xff;
                    return (-1, out);
                }
                for i in 0..n as usize {
                    let start = u32_at(req, 8 + i * 8).unwrap_or(u32::MAX) as u64;
                    let size = u32_at(req, 12 + i * 8).unwrap_or(u32::MAX) as u64;
                    if start + size > img_size as u64 {
                        out[0] = 0xfe;
                        out[6] = 0xff;
                        return (-1, out);
                    }
                }
                self.nsets = n as usize;
                self.copied = false;
                (0, out)
            }
            DRDI_COPY => {
                if self.copied {
                    return (0, out);
                }
                let sets: Vec<(u32, u32, u32)> = (0..self.nsets)
                    .map(|i| {
                        let o = 8 + i * 12;
                        (
                            u32_at(req, o).unwrap_or(u32::MAX),
                            u32_at(req, o + 4).unwrap_or(u32::MAX),
                            u32_at(req, o + 8).unwrap_or(u32::MAX),
                        )
                    })
                    .collect();
                let bad = sets.iter().any(|&(src, dst, ln)| {
                    ln != 0
                        && (ln > DRDI_LEN_MAX
                            || src as u64 + ln as u64 > img_size as u64
                            || dst as usize + ln as usize > DRDI_SMEM_SIZE)
                });
                if bad {
                    out[0] = 0xff;
                    return (-1, out);
                }
                let Ok(mut f) = File::open(&self.part) else {
                    out[0] = 0xff;
                    return (-1, out);
                };
                for &(src, dst, ln) in sets.iter().filter(|s| s.2 != 0) {
                    let mut buf = vec![0u8; ln as usize];
                    if f.seek(SeekFrom::Start(img_off + src as u64)).is_err()
                        || f.read_exact(&mut buf).is_err()
                    {
                        out[0] = 0xff;
                        return (-1, out);
                    }
                    // SAFETY: dst + ln <= DRDI_SMEM_SIZE was checked above
                    unsafe {
                        std::ptr::copy_nonoverlapping(buf.as_ptr(), self.smem.add(dst as usize), buf.len());
                    }
                    log(&format!("drdi: set {src:#x} -> {dst:#x}, {ln:#x}"));
                }
                let last = req[4] == 1;
                self.copied = last;
                out[4] = if last { 0 } else { 0xff };
                (0, out)
            }
            _ => {
                out[0] = 0xff;
                (-1, out)
            }
        }
    }
}

pub fn handle(drdi: &mut Option<Drdi>, op: u32, args: &[Vec<u8>]) -> Vec<Vec<u8>> {
    match op {
        RPC_GPIO_ADC => return rpc_gpio_adc(args),
        RPC_GET_GPIO_NUM if args.len() >= 2 => return rpc_get_gpio_num(args),
        RPC_GET_EINT_ATTR if args.len() >= 3 => return rpc_get_eint_attr(args),
        RPC_DTSI_QUERY if !args.is_empty() => {
            log(&format!("rpc: dtsi query {}", cstr(args[0].get(2..).unwrap_or(&[]))));
            // like the stock kernel, report "not set"
            let mut out = Vec::new();
            for _ in 0..17 {
                out.extend_from_slice(&UNSET.to_le_bytes());
            }
            return vec![i32p(0), out];
        }
        RPC_LHIF_MAPPING if args.len() == 1 && args[0].len() >= 8 => {
            let net_if = u32_at(&args[0], 0).unwrap();
            let lhif_q = u32_at(&args[0], 4).unwrap();
            log(&format!("rpc: lhif mapping ccmni{net_if} -> queue {lhif_q}"));
            return vec![i32p(0), i32p(0)];
        }
        RPC_QUERY_0X4016 => return vec![i32p(0), i32p(0)],
        RPC_AMMS_DRDI if args.first().is_some_and(|a| a.len() == 0xbc) => {
            if let Some(d) = drdi.as_mut() {
                let (ret, out) = d.handle(&args[0]);
                if verbose() {
                    log(&format!("drdi: req {} -> {ret} {}", hex(&args[0][..8]), hex(&out)));
                }
                return vec![i32p(ret), out.to_vec()];
            }
        }
        _ => {}
    }
    let a: Vec<String> = args.iter().map(|a| hex(a)).collect();
    log(&format!("rpc: unhandled op {op:#x} {a:?}"));
    vec![i32p(-1)]
}

fn opt<T: std::fmt::Display>(v: Option<T>) -> String {
    v.map_or("None".into(), |v| v.to_string())
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|c| format!("{c:02x}")).collect()
}
