use std::{
    error::Error,
    fs,
    io::{Write, stdout},
    path::{Path, PathBuf},
    thread::sleep,
    time::Duration,
};

use bincode::Encode;
use clap::{Parser, Subcommand, ValueEnum};
use clap_num::maybe_hex;
use colored::Colorize;
use derive_more::IsVariant;
use serialport::{SerialPort, SerialPortInfo, SerialPortType, available_ports};

mod logging;

type Result<T> = core::result::Result<T, Box<dyn Error>>;
type Port = Box<dyn SerialPort>;

const HANDSHAKE: [u8; 3] = [0x0a, 0x50, 0x05];

const DA_ADDR: u32 = 0x81e00000;
const BOOT_ARG_ADDR: u32 = 0x800d0000;

trait DA {
    fn write_and_check(&mut self, byte: u8, expected: u8) -> Result<bool>;
    fn echo_u8(&mut self, byte: u8) -> Result<()>;
    fn echo_u32(&mut self, data: u32) -> Result<()>;
}

impl DA for Port {
    fn write_and_check(&mut self, byte: u8, expected: u8) -> Result<bool> {
        self.write_all(&[byte])?;
        let mut buf = [0; 1];
        self.read_exact(&mut buf)?;
        Ok(u8::from_be_bytes(buf) == expected)
    }

    fn echo_u8(&mut self, byte: u8) -> Result<()> {
        self.write_and_check(byte, byte)?;

        Ok(())
    }

    fn echo_u32(&mut self, data: u32) -> Result<()> {
        self.write_all(&data.to_be_bytes())?;
        let mut buf = [0; 4];
        self.read_exact(&mut buf)?;

        let result = u32::from_be_bytes(buf);
        if data == result {
            Ok(())
        } else {
            Err(format!("Data doesn't match! Expected: {data:#x}, got: {result:#x}").into())
        }
    }
}

#[derive(Clone, IsVariant, Subcommand)]
enum Command {
    /// Boot bare-metal payload through send_da and jump_da preloader commands
    Boot {
        /// Binaries to upload
        #[arg(short, long, value_delimiter = ' ', num_args = 1..)]
        input: Vec<PathBuf>,

        /// Addresses for binaries
        #[arg(short, long, value_delimiter = ' ', num_args = 1.., value_parser=maybe_hex::<u32>)]
        upload_address: Vec<u32>,

        /// Final jump address, jumps to 0x81e00000 if not set
        #[arg(short, long, value_parser=maybe_hex::<u32>)]
        jump_address: Option<u32>,

        /// Payload boot mode
        #[arg(short, long)]
        mode: Option<Mode>,

        /// LK boot mode
        #[arg(long)]
        lk_mode: Option<LkBootMode>,
    },

    /// Boot preloader patcher and dump preloader with changes (debugging)
    DumpPreloader,
}

#[derive(Clone, Default, ValueEnum, IsVariant)]
#[clap(rename_all = "kebab_case")]
enum Mode {
    #[default]
    Raw,
    Lk,
}

#[derive(Parser)]
#[command(version)]
struct Cli {
    /// Force booting preloader patcher
    #[arg(short, long)]
    force: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Clone, Default, Encode, ValueEnum)]
#[clap(rename_all = "kebab_case")]
#[repr(u32)]
enum LkBootMode {
    #[default]
    Normal,
    Meta,
    Recovery,
    SwReboot,
    Factory,
    Advmeta,
    AteFactory,
    Alarm,
    Fastboot = 99,
    Download,
}

#[derive(Default, Encode)]
#[repr(C)]
struct BootArgument {
    magic: u32,
    mode: u32,
    e_flag: u32,
    log_port: u32,
    log_baudrate: u32,
    log_enable: u8,
    reserved: [u8; 3],
    dram_rank_num: u32,
    dram_rank_size: [u32; 4],
    boot_reason: u32,
    meta_com_type: u32,
    meta_com_id: u32,
    boot_time: u32,
    /* da_info_t */
    addr: u32,
    arg1: u32,
    arg2: u32,
    /* SEC_LIMIT */
    magic_num: u32,
    forbid_mode: u32,
}

impl BootArgument {
    pub fn lk(mode: LkBootMode) -> Self {
        Self {
            magic: 0x504c504c,
            mode: mode as u32,
            e_flag: 0,
            log_port: 0x11005000,
            log_baudrate: 921600,
            log_enable: 1,
            dram_rank_num: 1,
            dram_rank_size: [0x20000000, 0, 0, 0],
            boot_reason: 1,
            boot_time: 1337,
            ..Default::default()
        }
    }
}

fn get_ports() -> Result<Vec<SerialPortInfo>> {
    Ok(available_ports()?
        .into_iter()
        .filter(|p| match &p.port_type {
            SerialPortType::UsbPort(p) => p.vid == 0x0e8d && p.pid == 0x2000,
            _ => false,
        })
        .collect())
}

fn send_da(port: &mut Port, addr: u32, payload: &[u8]) -> Result<()> {
    port.echo_u8(0xd7)?;

    let mut buf = [0; 1];
    match port.read(&mut buf) {
        Ok(_) => port.clear(serialport::ClearBuffer::Input)?, // clean garbage because ???
        Err(_) => (),
    }

    port.echo_u32(addr)?;
    port.echo_u32(payload.len() as u32)?;
    port.echo_u32(0)?;

    let mut buf = [0; 2];
    /* Status is always 0 */
    port.read_exact(&mut buf)?;

    port.write_all(payload)?;

    port.read_exact(&mut buf)?;
    port.read_exact(&mut buf)?;
    let status = u16::from_be_bytes(buf);
    if status != 0 {
        Err(format!("usbdl_verify_da failed, DAA may be enabled: {status}").into())
    } else {
        Ok(())
    }
}

fn jump_da(port: &mut Port, addr: u32) -> Result<()> {
    port.echo_u8(0xd5)?;
    port.echo_u32(addr)?;

    let mut buf = [0; 2];
    /* Status is always 0 if DA verification passed (it is at this point) */
    port.read_exact(&mut buf)?;

    Ok(())
}

fn read32(port: &mut Port, addr: u32, dwords: u32) -> Result<Vec<u32>> {
    port.echo_u8(0xd1)?;
    port.echo_u32(addr)?;
    port.echo_u32(dwords)?;

    let mut buf = [0; 2];
    port.read_exact(&mut buf)?;

    let mut vec = Vec::with_capacity(dwords as usize);
    let mut buf = [0; 4];
    for _ in 0..dwords {
        port.read_exact(&mut buf)?;
        vec.push(u32::from_be_bytes(buf));
    }

    let mut buf = [0; 2];
    port.read_exact(&mut buf)?;

    Ok(vec)
}

fn open_port() -> Result<Port> {
    log!("Waiting for the preloader interface");
    let port = loop {
        let ports = get_ports()?;

        if ports.len() > 1 {
            return Err("Please disconnect other devices in the preloader mode".into());
        } else if ports.is_empty() {
            log!(".");
        } else {
            println!("");
            break ports[0].clone();
        }

        sleep(Duration::from_millis(500));
    };

    println!("Found device at {}", &port.port_name);
    Ok(serialport::new(port.port_name, 921600)
        .timeout(Duration::from_millis(2000))
        .open()?)
}

fn handshake(port: &mut Port) -> Result<()> {
    let mut buf = [0; 1];
    loop {
        port.write(&[0xa0])?;
        port.flush()?;
        port.read_exact(&mut buf)?;

        if buf[0] == 0x5f {
            break;
        }
    }

    for byte in HANDSHAKE {
        port.write_and_check(byte, !byte)?;
    }

    /* Clean garbage because we spam with handshake  */
    sleep(Duration::from_millis(200));
    port.clear(serialport::ClearBuffer::All)?;

    Ok(())
}

fn run(cli: Cli) -> Result<()> {
    let mut port = open_port()?;

    /* Read "READY", just to be safe let's expect it may appear up to 4 times */
    let mut buf = [0; 20];
    port.read(&mut buf)?;
    handshake(&mut port)?;

    let (no_patcher, payload) = match &cli.command {
        Command::Boot {
            upload_address,
            input,
            ..
        } => {
            let no_patcher =
                upload_address.len() == 1 && upload_address[0] == DA_ADDR && !cli.force;
            (
                no_patcher,
                fs::read(if no_patcher {
                    &input[0]
                } else {
                    Path::new("payload/preloader_patcher.bin")
                })?,
            )
        }
        Command::DumpPreloader => (false, fs::read(Path::new("payload/preloader_patcher.bin"))?),
    };

    if no_patcher {
        println!(
            "Preloader won't be patched, some commands may be not available due to security checks"
        );
    }

    log!("Uploading payload to {DA_ADDR:#x}...");
    if let Err(e) = status!(send_da(&mut port, DA_ADDR, &payload)) {
        eprintln!("{e}");
        drop(port);
        return run(cli);
    }
    log!("Jumping to {DA_ADDR:#x}...");
    status!(jump_da(&mut port, DA_ADDR))?;

    if !no_patcher {
        log!("Trying to sync with patched preloader...");
        status!(handshake(&mut port))?;
    }

    match cli.command {
        Command::Boot {
            input,
            upload_address,
            jump_address,
            mode,
            lk_mode,
        } => {
            if !no_patcher {
                let mode = mode.unwrap_or_default();

                for (i, a) in input.into_iter().zip(upload_address) {
                    let mut payload = fs::read(i)?;
                    if mode.is_lk() {
                        payload.drain(0..0x200);
                    }
                    log!("Uploading payload to {a:#x}...");
                    status!(send_da(&mut port, a, &payload))?;
                }

                if mode.is_lk() {
                    log!("Preparing boot argument for LK...");
                    status!(send_da(
                        &mut port,
                        BOOT_ARG_ADDR,
                        &bincode::encode_to_vec(
                            BootArgument::lk(lk_mode.unwrap_or_default()),
                            bincode::config::standard()
                                .with_little_endian()
                                .with_fixed_int_encoding(),
                        )?,
                    ))?;
                }

                let jump = jump_address.unwrap_or_default();
                log!("Jumping to {jump:#x}...");
                status!(jump_da(&mut port, jump))?;
            }
        }
        Command::DumpPreloader => {
            log!("Dumping preloader from ram...");
            let preloader = status!(read32(&mut port, 0x2007500, (1 * 1024 * 1024) / 4))?
                .into_iter()
                .map(|u32| u32.to_le_bytes())
                .flatten()
                .collect::<Vec<_>>();
            fs::write("preloader.bin", preloader)?;
            return Ok(());
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match &cli.command {
        Command::Boot {
            input,
            upload_address,
            ..
        } => {
            assert!(!input.is_empty());
            assert_eq!(input.len(), upload_address.len());
        }
        _ => (),
    }

    run(cli)
}
