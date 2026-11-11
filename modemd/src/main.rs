//! plinius-modemd: the AP-side servers the MT6878 modem needs to boot and
//! run on mainline Linux (mtk-ccci driver).
//!
//! * /dev/ccci_fs  - file system server: the modem's NVRAM lives under
//!   `--root` (see fs.rs)
//! * /dev/ccci_rpc - RPC server: GPIO/EINT queries, DTSI queries, LHIF
//!   mapping and the DRDI calibration copy from the modem partition
//!   (see rpc.rs)
//!
//! Unless `--no-start` is given it then starts the modem through the
//! driver's sysfs `state` file; with `--radio` it also opens the SIM session
//! over MIPC (radio state 0: the user enters the PIN, then turns the radio
//! on, e.g. through ModemManager). Under systemd (Type=notify) it reports
//! READY=1 once the modem is up; `--ready-file` does the same for OpenRC.

mod fs;
mod gpio;
mod mipc;
mod rpc;
mod wire;

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::linux::net::SocketAddrExt;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::{SocketAddr, UnixDatagram};
use std::sync::atomic::{AtomicBool, Ordering};
use wire::{build_params, header, parse_header, parse_params, HDR_LEN, RESP};

const STATE: &str = "/sys/bus/platform/drivers/mtk-ccci/10209000.modem/state";
const FS_MORE: u32 = 1 << 31;
const FS_FRAG_DATA: usize = 0xd80;
const READ_MAX: usize = 0x10000;

static VERBOSE: AtomicBool = AtomicBool::new(false);
static LOG_STDERR: AtomicBool = AtomicBool::new(false);

pub fn verbose() -> bool {
    VERBOSE.load(Ordering::Relaxed)
}

/// One log line, "HH:MM:SS message", like the Python server
pub fn log(msg: &str) {
    // SAFETY: time/localtime_r into a zeroed tm
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        libc::localtime_r(&t, &mut tm);
    }
    let line = format!("{:02}:{:02}:{:02} {msg}\n", tm.tm_hour, tm.tm_min, tm.tm_sec);
    if LOG_STDERR.load(Ordering::Relaxed) {
        let _ = std::io::stderr().write_all(line.as_bytes());
    } else {
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(line.as_bytes());
        let _ = out.flush();
    }
}

fn die(msg: &str) -> ! {
    log(msg);
    std::process::exit(1);
}

fn open_rw(path: &str) -> File {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap_or_else(|e| die(&format!("cannot open {path}: {e}")))
}

fn read_pkt(f: &mut File) -> Vec<u8> {
    let mut buf = vec![0u8; READ_MAX];
    match f.read(&mut buf) {
        Ok(n) => buf.truncate(n),
        Err(e) => {
            log(&format!("read failed: {e}"));
            buf.clear();
        }
    }
    buf
}

fn write_pkt(f: &mut File, pkt: &[u8]) {
    if let Err(e) = f.write(pkt) {
        log(&format!("write failed: {e}"));
    }
}

fn serve_rpc(ch: &mut File, drdi: &mut Option<rpc::Drdi>, pkt: &[u8]) {
    let Some(h) = parse_header(pkt) else {
        log(&format!("rpc: short packet ({} bytes)", pkt.len()));
        return;
    };
    let (op, args) = parse_params(&pkt[HDR_LEN..]);
    if verbose() {
        log(&format!("rpc: op {op:#x} slot {}", h.slot));
    }
    let mut out = header(h.data0, h.slot).to_vec();
    out.extend_from_slice(&build_params(op | RESP, &rpc::handle(drdi, op, &args)));
    write_pkt(ch, &out);
    if verbose() {
        log(&format!("rpc: replied {op:#x}"));
    }
}

/// FS channel: requests may arrive in fragments (FS_MORE), and replies are
/// sent the same way; every continuation repeats the 4 byte op id.
struct FsChannel {
    file: File,
    server: fs::FsServer,
    partial: std::collections::HashMap<u32, Vec<u8>>,
}

impl FsChannel {
    fn serve(&mut self, pkt: &[u8]) {
        let Some(h) = parse_header(pkt) else {
            log(&format!("fs: short packet ({} bytes)", pkt.len()));
            return;
        };
        let end = (h.len as usize).clamp(HDR_LEN, pkt.len());
        let body = &pkt[HDR_LEN..end];
        match self.partial.get_mut(&h.slot) {
            Some(p) => p.extend_from_slice(body.get(4..).unwrap_or(&[])),
            None => {
                self.partial.insert(h.slot, body.to_vec());
            }
        }
        if h.data0 & FS_MORE != 0 {
            return;
        }
        let req = self.partial.remove(&h.slot).unwrap();
        let (op, args) = parse_params(&req);
        let params = self.server.handle(op, &args);
        if verbose() {
            let p: Vec<String> = params.iter().map(|p| rpc::hex(&p[..p.len().min(8)])).collect();
            log(&format!("fs: op {op:#x} -> {p:?}"));
        }
        self.reply(op | RESP, &params, h.slot);
    }

    fn reply(&mut self, op: u32, params: &[Vec<u8>], slot: u32) {
        let stream = build_params(op, params);
        let (opb, rest) = stream.split_at(4);
        let chunks: Vec<&[u8]> = if rest.is_empty() { vec![&[]] } else { rest.chunks(FS_FRAG_DATA).collect() };
        let last = chunks.len() - 1;
        for (i, c) in chunks.iter().enumerate() {
            let mut pkt = header(if i < last { FS_MORE } else { 0 }, slot).to_vec();
            pkt.extend_from_slice(opb);
            pkt.extend_from_slice(c);
            write_pkt(&mut self.file, &pkt);
        }
    }
}

/// Tell systemd (Type=notify) about our state, if it runs us
fn sd_notify(state: &str) {
    let Some(path) = std::env::var_os("NOTIFY_SOCKET") else { return };
    let path = path.as_encoded_bytes();
    let addr = match path.strip_prefix(b"@") {
        Some(name) => SocketAddr::from_abstract_name(name),
        None => SocketAddr::from_pathname(std::ffi::OsStr::from_bytes(path)),
    };
    let r = addr.and_then(|a| {
        let s = UnixDatagram::unbound()?;
        s.connect_addr(&a)?;
        s.send(state.as_bytes())
    });
    if let Err(e) = r {
        log(&format!("sd_notify failed: {e}"));
    }
}

fn start_modem(radio: bool, ready_file: Option<String>) {
    log("starting the modem");
    let r = OpenOptions::new().write(true).open(STATE).and_then(|mut f| f.write_all(b"start"));
    if let Err(e) = r {
        die(&format!("modem start failed: {e}"));
    }
    log("modem ready");
    if radio {
        // SIM PIN entry stays with the user (AT+CPIN), then radio state 1
        mipc::radio_state(false);
    }
    sd_notify("READY=1");
    if let Some(p) = ready_file {
        if let Err(e) = File::create(&p) {
            log(&format!("cannot create {p}: {e}"));
        }
    }
}

struct Opts {
    root: String,
    modem_part: String,
    no_start: bool,
    radio: bool,
    ready_file: Option<String>,
}

fn usage() -> ! {
    eprintln!(
        "usage: plinius-modemd [--root DIR] [--modem-part DEV] [--no-start] [--radio] [--ready-file F] [-v]\n\
         \n  --root DIR        modem NVRAM root (default /var/lib/modem)\
         \n  --modem-part DEV  modem image partition (default /dev/disk/by-partlabel/modem_a)\
         \n  --no-start        only serve, do not start the modem\
         \n  --radio           start the SIM over MIPC once the modem is ready\
         \n  --ready-file F    create F once the modem is up (for OpenRC)\
         \n  -v, --verbose     log every request"
    );
    std::process::exit(2);
}

fn parse_args() -> Opts {
    let mut o = Opts {
        root: "/var/lib/modem".into(),
        modem_part: "/dev/disk/by-partlabel/modem_a".into(),
        no_start: false,
        radio: false,
        ready_file: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let (key, inline) = match a.split_once('=') {
            Some((k, v)) if k.starts_with("--") => (k.to_string(), Some(v.to_string())),
            _ => (a.clone(), None),
        };
        let mut value = || inline.clone().or_else(|| it.next()).unwrap_or_else(|| usage());
        match key.as_str() {
            "--root" => o.root = value(),
            "--modem-part" => o.modem_part = value(),
            "--no-start" => o.no_start = true,
            "--radio" => o.radio = true,
            "--ready-file" => o.ready_file = Some(value()),
            "-v" | "--verbose" => VERBOSE.store(true, Ordering::Relaxed),
            _ => usage(),
        }
    }
    o
}

/// Debug builds only: serve FS requests from stdin (u32 length + request
/// params) with replies on stdout, for differential tests against
/// ccci_fs.py.
#[cfg(debug_assertions)]
fn fs_replay(root: &str) -> ! {
    LOG_STDERR.store(true, Ordering::Relaxed);
    let mut server = fs::FsServer::new(root);
    let (mut inp, mut out) = (std::io::stdin().lock(), std::io::stdout().lock());
    loop {
        let mut n = [0u8; 4];
        if inp.read_exact(&mut n).is_err() {
            std::process::exit(0);
        }
        let mut req = vec![0u8; u32::from_le_bytes(n) as usize];
        inp.read_exact(&mut req).unwrap();
        let (op, args) = parse_params(&req);
        let reply = build_params(op | RESP, &server.handle(op, &args));
        out.write_all(&(reply.len() as u32).to_le_bytes()).unwrap();
        out.write_all(&reply).unwrap();
        out.flush().unwrap();
    }
}

fn main() {
    #[cfg(debug_assertions)]
    if let Some(root) = std::env::var_os("MODEMD_FS_REPLAY") {
        fs_replay(root.to_str().unwrap());
    }
    let o = parse_args();
    let mut fs = FsChannel {
        file: open_rw("/dev/ccci_fs"),
        server: fs::FsServer::new(&o.root),
        partial: Default::default(),
    };
    let mut rpc_ch = open_rw("/dev/ccci_rpc");
    let mut drdi = match rpc::Drdi::new(rpc_ch.as_raw_fd(), &o.modem_part) {
        Ok(d) => Some(d),
        Err(e) => {
            log(&format!("drdi: cannot map shared memory: {e}"));
            None
        }
    };
    if !o.no_start {
        let (radio, ready_file) = (o.radio, o.ready_file.clone());
        if let Some(p) = &ready_file {
            let _ = std::fs::remove_file(p);
        }
        std::thread::spawn(move || start_modem(radio, ready_file));
    }

    let mut pfd = [
        libc::pollfd { fd: fs.file.as_raw_fd(), events: libc::POLLIN, revents: 0 },
        libc::pollfd { fd: rpc_ch.as_raw_fd(), events: libc::POLLIN, revents: 0 },
    ];
    loop {
        // SAFETY: two valid pollfds
        if unsafe { libc::poll(pfd.as_mut_ptr(), 2, -1) } < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            die(&format!("poll failed: {e}"));
        }
        if (pfd[0].revents | pfd[1].revents) & libc::POLLNVAL != 0 {
            die("ccci channel error");
        }
        if pfd[0].revents & libc::POLLIN != 0 {
            let pkt = read_pkt(&mut fs.file);
            if !pkt.is_empty() {
                fs.serve(&pkt);
            }
        }
        if pfd[1].revents & libc::POLLIN != 0 {
            let pkt = read_pkt(&mut rpc_ch);
            if !pkt.is_empty() {
                serve_rpc(&mut rpc_ch, &mut drdi, &pkt);
            }
        }
    }
}
