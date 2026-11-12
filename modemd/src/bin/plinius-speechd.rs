// SPDX-License-Identifier: GPL-2.0-only
//! plinius-speechd: call audio for the MT6878 modem.
//!
//! During a call the modem's speech codec talks to the AFE over the PCM 1
//! interface; the AP has to (1) turn the modem's speech path on over the
//! CCCI speech channel (/dev/ccci_aud), (2) keep the "Hostless_Speech"
//! front end running, which clocks the PCM 1 and codec back ends, and
//! (3) acknowledge the modem's speech notifications.
//!
//! The daemon follows the audio routing: when the modem downlink is routed
//! to the codec ("ADDA_DL_CH1 PCM_1_CAP_CH1" on, e.g. by the UCM "Voice Call"
//! verb that callaudiod selects for a call), it turns speech on, and off
//! again when that route goes away.
//!
//! Wire format of the speech messages (SpeechMessengerNormal in the stock
//! HAL; see also q25_speech.c of the Marathon Shell q25-modem package):
//!   mailbox: [0xffffffff, id << 16 | param16, channel, param32]
//!   payload: [0, len, channel, id << 16 | len] + [0xa2a2, type, size] + data

use std::ffi::CString;
use std::fs;
use std::io::{Error, Result};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::time::{Duration, Instant};

const CCCI_AUD: &str = "/dev/ccci_aud";
const CCCI_PCM_TX: u32 = 5;
const MAILBOX_MAGIC: u32 = 0xffff_ffff;
const AP_PAYLOAD_SYNC: u16 = 0xa2a2;

const MSG_A2M_SPH_ON: u16 = 0x2f20;
const MSG_A2M_SPH_OFF: u16 = 0x2f21;
const MSG_M2A_ACK: u16 = 0x8000;
const DATA_TYPE_SPH_INFO: u16 = 25;

/// Modem notifications the stock HAL acknowledges (ack id = id & 0x7fff):
/// modem alive, EPOF, network status, network codec, EM data request.
const AUTO_ACK: [u16; 5] = [0xafa0, 0xaf78, 0xaf75, 0xaf90, 0xaf70];

const APP_NORMAL: u8 = 0;
const APP_LOOPBACK: u8 = 2;

/// The control whose state says "a call is routed": modem downlink -> codec
const ROUTE_CTL: &str = "ADDA_DL_CH1 PCM_1_CAP_CH1";

fn log(msg: &str) {
    eprintln!("plinius-speechd: {msg}");
}

fn now() -> Instant {
    Instant::now()
}

// ---------------------------------------------------------------------------
// ioctl helpers (aarch64 layouts, checked against <sound/asound.h>)

const fn ioc(dir: u64, ty: u8, nr: u8, size: usize) -> u64 {
    (dir << 30) | ((size as u64) << 16) | ((ty as u64) << 8) | nr as u64
}
const IOC_W: u64 = 1;
const IOC_R: u64 = 2;

const HW_PARAMS_SIZE: usize = 608;
const SW_PARAMS_SIZE: usize = 136;
const CTL_VALUE_SIZE: usize = 1224;
const CTL_EVENT_SIZE: usize = 72;

const PCM_HW_PARAMS: u64 = ioc(IOC_R | IOC_W, b'A', 0x11, HW_PARAMS_SIZE);
const PCM_SW_PARAMS: u64 = ioc(IOC_R | IOC_W, b'A', 0x13, SW_PARAMS_SIZE);
const PCM_PREPARE: u64 = ioc(0, b'A', 0x40, 0);
const PCM_START: u64 = ioc(0, b'A', 0x42, 0);
const PCM_DROP: u64 = ioc(0, b'A', 0x43, 0);
const CTL_ELEM_READ: u64 = ioc(IOC_R | IOC_W, b'U', 0x12, CTL_VALUE_SIZE);
const CTL_SUBSCRIBE: u64 = ioc(IOC_R | IOC_W, b'U', 0x16, 4);

fn ioctl(fd: &OwnedFd, req: u64, arg: *mut libc::c_void) -> Result<()> {
    if unsafe { libc::ioctl(fd.as_raw_fd(), req as _, arg) } < 0 {
        return Err(Error::last_os_error());
    }
    Ok(())
}

fn open(path: &str, flags: i32) -> Result<OwnedFd> {
    let c = CString::new(path).unwrap();
    let fd = unsafe { libc::open(c.as_ptr(), flags | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn put32(b: &mut [u8], off: usize, v: u32) {
    b[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

fn put64(b: &mut [u8], off: usize, v: u64) {
    b[off..off + 8].copy_from_slice(&v.to_le_bytes());
}

fn get32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(b[off..off + 4].try_into().unwrap())
}

// ---------------------------------------------------------------------------
// Hostless_Speech PCM

const SNDRV_PCM_ACCESS_RW_INTERLEAVED: u32 = 3;
const SNDRV_PCM_FORMAT_S16_LE: u32 = 2;
const HW_PARAM_FIRST_MASK: usize = 0; // ACCESS, FORMAT, SUBFORMAT
const HW_PARAM_FIRST_INTERVAL: u32 = 8;
const HW_PARAM_CHANNELS: u32 = 10;
const HW_PARAM_RATE: u32 = 11;
const HW_PARAM_PERIOD_SIZE: u32 = 13;
const HW_PARAM_PERIODS: u32 = 15;
const HW_PARAM_LAST_INTERVAL: u32 = 19;

/// The device number of the Hostless_Speech PCM on card `card`
fn hostless_device(card: u32) -> Option<u32> {
    let dir = format!("/proc/asound/card{card}");
    for e in fs::read_dir(&dir).ok()?.flatten() {
        let name = e.file_name().into_string().ok()?;
        let Some(dev) = name.strip_prefix("pcm").and_then(|s| s.strip_suffix('p')) else {
            continue;
        };
        let info = fs::read_to_string(e.path().join("info")).unwrap_or_default();
        if info.lines().any(|l| l.starts_with("id: Hostless_Speech")) {
            return dev.parse().ok();
        }
    }
    None
}

fn pcm_open(card: u32, dev: u32, capture: bool, rate: u32) -> Result<OwnedFd> {
    let path = format!("/dev/snd/pcmC{card}D{dev}{}", if capture { 'c' } else { 'p' });
    let fd = open(&path, libc::O_RDWR)?;

    let mut p = [0u8; HW_PARAMS_SIZE];
    // masks (3 used + 5 reserved, 32 bytes each) at 4: allow all, then pin
    for m in 0..3 {
        p[4 + m * 32..4 + m * 32 + 32].fill(0xff);
    }
    let set_mask = |p: &mut [u8], n: usize, bit: u32| {
        let off = 4 + (n - HW_PARAM_FIRST_MASK) * 32;
        p[off..off + 32].fill(0);
        p[off + (bit as usize / 8)] |= 1 << (bit % 8);
    };
    set_mask(&mut p, 0, SNDRV_PCM_ACCESS_RW_INTERLEAVED);
    set_mask(&mut p, 1, SNDRV_PCM_FORMAT_S16_LE);
    set_mask(&mut p, 2, 0);
    // intervals (12 bytes each) at 260: min, max, flags (bit 2 integer)
    let ival = |p: &mut [u8], n: u32, min: u32, max: u32, int: bool| {
        let off = 260 + ((n - HW_PARAM_FIRST_INTERVAL) as usize) * 12;
        put32(p, off, min);
        put32(p, off + 4, max);
        put32(p, off + 8, if int { 4 } else { 0 });
    };
    for n in HW_PARAM_FIRST_INTERVAL..=HW_PARAM_LAST_INTERVAL {
        ival(&mut p, n, 0, u32::MAX, false);
    }
    let period = rate / 50; // 20 ms
    ival(&mut p, HW_PARAM_CHANNELS, 2, 2, true);
    ival(&mut p, HW_PARAM_RATE, rate, rate, true);
    ival(&mut p, HW_PARAM_PERIOD_SIZE, period, period, true);
    ival(&mut p, HW_PARAM_PERIODS, 4, 4, true);
    put32(&mut p, 512, u32::MAX); // rmask
    put32(&mut p, 520, u32::MAX); // info
    ioctl(&fd, PCM_HW_PARAMS, p.as_mut_ptr().cast())?;

    // never stop on an xrun: nothing reads or writes the buffer
    let buffer = (period * 4) as u64;
    let mut boundary = buffer;
    while boundary * 2 <= (i64::MAX as u64) - buffer {
        boundary *= 2;
    }
    let mut s = [0u8; SW_PARAMS_SIZE];
    put32(&mut s, 4, 1); // period_step
    put64(&mut s, 16, 1); // avail_min
    put64(&mut s, 32, boundary); // start_threshold: started explicitly
    put64(&mut s, 40, boundary); // stop_threshold
    put64(&mut s, 64, boundary);
    ioctl(&fd, PCM_SW_PARAMS, s.as_mut_ptr().cast())?;
    ioctl(&fd, PCM_PREPARE, std::ptr::null_mut())?;
    ioctl(&fd, PCM_START, std::ptr::null_mut())?;
    Ok(fd)
}

// ---------------------------------------------------------------------------
// ALSA control: follow the call routing

struct Ctl {
    fd: OwnedFd,
}

impl Ctl {
    fn open(card: u32) -> Result<Ctl> {
        let fd = open(&format!("/dev/snd/controlC{card}"), libc::O_RDWR | libc::O_NONBLOCK)?;
        let mut on: i32 = 1;
        ioctl(&fd, CTL_SUBSCRIBE, (&mut on as *mut i32).cast())?;
        Ok(Ctl { fd })
    }

    /// Value 0 of the mixer control `name`
    fn read(&self, name: &str) -> Result<bool> {
        let mut v = [0u8; CTL_VALUE_SIZE];
        put32(&mut v, 4, 2); // SNDRV_CTL_ELEM_IFACE_MIXER
        v[16..16 + name.len()].copy_from_slice(name.as_bytes());
        ioctl(&self.fd, CTL_ELEM_READ, v.as_mut_ptr().cast())?;
        Ok(get32(&v, 72) != 0)
    }

    /// Drain the queued events; true if any of them was a value change
    fn drain(&self) -> bool {
        let mut changed = false;
        let mut ev = [0u8; CTL_EVENT_SIZE * 8];
        loop {
            let n = unsafe {
                libc::read(self.fd.as_raw_fd(), ev.as_mut_ptr().cast(), ev.len())
            };
            if n <= 0 {
                return changed;
            }
            changed |= ev.chunks(CTL_EVENT_SIZE).take(n as usize / CTL_EVENT_SIZE)
                .any(|e| get32(e, 0) == 0 && get32(e, 4) & 1 != 0); // ELEM, VALUE
        }
    }
}

// ---------------------------------------------------------------------------
// Speech messages to and from the modem

struct Modem {
    fd: OwnedFd,
}

impl Modem {
    fn write(&self, b: &[u8]) -> Result<()> {
        let n = unsafe { libc::write(self.fd.as_raw_fd(), b.as_ptr().cast(), b.len()) };
        if n != b.len() as isize {
            return Err(Error::last_os_error());
        }
        Ok(())
    }

    fn mailbox(&self, id: u16, p16: u16, p32: u32) -> Result<()> {
        let mut b = [0u8; 16];
        put32(&mut b, 0, MAILBOX_MAGIC);
        put32(&mut b, 4, (id as u32) << 16 | p16 as u32);
        put32(&mut b, 8, CCCI_PCM_TX);
        put32(&mut b, 12, p32);
        self.write(&b)
    }

    fn payload(&self, id: u16, ty: u16, data: &[u8]) -> Result<()> {
        let len = data.len() as u32 + 6;
        let mut b = vec![0u8; 16 + len as usize];
        put32(&mut b, 4, len);
        put32(&mut b, 8, CCCI_PCM_TX);
        put32(&mut b, 12, (id as u32) << 16 | len);
        b[16..18].copy_from_slice(&AP_PAYLOAD_SYNC.to_le_bytes());
        b[18..20].copy_from_slice(&ty.to_le_bytes());
        b[20..22].copy_from_slice(&(data.len() as u16).to_le_bytes());
        b[22..].copy_from_slice(data);
        self.write(&b)
    }

    /// Handle one message from the modem, if any arrives within `timeout`;
    /// returns its id
    fn recv(&self, timeout: Duration) -> Result<Option<u16>> {
        let mut p = libc::pollfd { fd: self.fd.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        if unsafe { libc::poll(&mut p, 1, timeout.as_millis() as i32) } <= 0 {
            return Ok(None);
        }
        let mut b = [0u8; 4096];
        let n = unsafe { libc::read(self.fd.as_raw_fd(), b.as_mut_ptr().cast(), b.len()) };
        if n < 0 {
            return Err(Error::last_os_error());
        }
        if n < 16 {
            return Ok(None);
        }
        let id = if get32(&b, 0) == MAILBOX_MAGIC {
            (get32(&b, 4) >> 16) as u16
        } else {
            (get32(&b, 12) >> 16) as u16
        };
        if AUTO_ACK.contains(&id) {
            self.mailbox(id & 0x7fff, 0, 0)?;
        } else {
            log(&format!("modem message {id:#06x} ({n} bytes)"));
        }
        Ok(Some(id))
    }

    fn wait_ack(&self, msg: u16, timeout: Duration) -> Result<()> {
        let end = now() + timeout;
        while now() < end {
            if self.recv(end - now())? == Some(MSG_M2A_ACK | msg) {
                return Ok(());
            }
        }
        Err(Error::new(std::io::ErrorKind::TimedOut, format!("no ack for {msg:#06x}")))
    }

    fn speech_on(&self, app: u8, rate: u32) -> Result<()> {
        // sph_info_t (128 bytes): parameters left invalid -> modem defaults
        let mut info = [0u8; 128];
        info[0] = app;
        info[2] = match rate {
            8000 => 0,
            32000 => 2,
            48000 => 3,
            _ => 1,
        };
        // pick up what is queued (modem alive, EM requests) first
        while self.recv(Duration::from_millis(50))?.is_some() {}
        self.payload(MSG_A2M_SPH_ON, DATA_TYPE_SPH_INFO, &info)?;
        self.wait_ack(MSG_A2M_SPH_ON, Duration::from_secs(3))
    }

    fn speech_off(&self) -> Result<()> {
        self.mailbox(MSG_A2M_SPH_OFF, 0, 0)?;
        self.wait_ack(MSG_A2M_SPH_OFF, Duration::from_millis(1500))
    }
}

// ---------------------------------------------------------------------------

struct Call {
    _pcm: Vec<OwnedFd>,
}

fn call_start(modem: &Modem, card: u32, rate: u32, app: u8) -> Result<Call> {
    let hold_only = std::env::var_os("SPEECHD_NO_MODEM").is_some();
    let dev = hostless_device(card)
        .ok_or_else(|| Error::other("no Hostless_Speech PCM"))?;
    let mut pcm = Vec::new();
    for capture in [false, true] {
        pcm.push(pcm_open(card, dev, capture, rate)?);
    }
    if hold_only {
        log("PCM only, modem speech left off");
        return Ok(Call { _pcm: pcm });
    }
    modem.speech_on(app, rate)?;
    log(&format!("speech on ({rate} Hz{})", if app == APP_LOOPBACK { ", loopback" } else { "" }));
    Ok(Call { _pcm: pcm })
}

fn call_stop(modem: &Modem, call: Call) {
    if let Err(e) = modem.speech_off() {
        log(&format!("speech off: {e}"));
    }
    for fd in &call._pcm {
        let _ = ioctl(fd, PCM_DROP, std::ptr::null_mut());
    }
    log("speech off");
}

fn usage() -> ! {
    eprintln!(
        "usage: plinius-speechd [--card N] [--rate HZ] [--loopback SECONDS]\n\
         \n  --rate HZ           PCM rate between AFE and modem (default 16000)\
         \n  --loopback SECONDS  test: speech on with the modem looping the\
         \n                      microphone back to the earpiece, then exit"
    );
    std::process::exit(2);
}

fn main() {
    let mut card = 0u32;
    let mut rate = 16000u32;
    let mut loopback: Option<u64> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut val = || args.next().and_then(|v| v.parse().ok()).unwrap_or_else(|| usage());
        match a.as_str() {
            "--card" => card = val() as u32,
            "--rate" => rate = val() as u32,
            "--loopback" => loopback = Some(val()),
            _ => usage(),
        }
    }

    let modem = loop {
        match open(CCCI_AUD, libc::O_RDWR) {
            Ok(fd) => break Modem { fd },
            Err(e) if loopback.is_none() => {
                log(&format!("{CCCI_AUD}: {e}, retrying"));
                std::thread::sleep(Duration::from_secs(5));
            }
            Err(e) => {
                log(&format!("{CCCI_AUD}: {e}"));
                std::process::exit(1);
            }
        }
    };

    if let Some(secs) = loopback {
        let call = call_start(&modem, card, rate, APP_LOOPBACK).unwrap_or_else(|e| {
            log(&format!("speech on: {e}"));
            std::process::exit(1);
        });
        let end = now() + Duration::from_secs(secs);
        while now() < end {
            if let Err(e) = modem.recv(end - now()) {
                log(&format!("modem: {e}"));
                break;
            }
        }
        call_stop(&modem, call);
        return;
    }

    let ctl = loop {
        match Ctl::open(card) {
            Ok(c) => break c,
            Err(e) => {
                log(&format!("sound card {card}: {e}, retrying"));
                std::thread::sleep(Duration::from_secs(5));
            }
        }
    };
    log("ready");
    let mut call: Option<Call> = None;
    let mut check = true;
    loop {
        if check {
            let routed = ctl.read(ROUTE_CTL).unwrap_or(false);
            if routed && call.is_none() {
                match call_start(&modem, card, rate, APP_NORMAL) {
                    Ok(c) => call = Some(c),
                    Err(e) => log(&format!("speech on: {e}")),
                }
            } else if !routed {
                if let Some(c) = call.take() {
                    call_stop(&modem, c);
                }
            }
        }
        let mut p = [
            libc::pollfd { fd: modem.fd.as_raw_fd(), events: libc::POLLIN, revents: 0 },
            libc::pollfd { fd: ctl.fd.as_raw_fd(), events: libc::POLLIN, revents: 0 },
        ];
        if unsafe { libc::poll(p.as_mut_ptr(), 2, -1) } < 0 {
            continue;
        }
        if p[0].revents != 0 {
            if let Err(e) = modem.recv(Duration::ZERO) {
                log(&format!("modem: {e}"));
                std::thread::sleep(Duration::from_secs(1));
            }
        }
        check = p[1].revents != 0 && ctl.drain();
    }
}
