use std::{
    error::Error,
    fs,
    io::{Write, stdout},
    path::{Path, PathBuf},
    thread::sleep,
    time::Duration,
};

use clap::Parser;
use clap_num::maybe_hex;
use serialport::{SerialPort, SerialPortInfo, SerialPortType, available_ports};

type Result<T> = core::result::Result<T, Box<dyn Error>>;
type Port = Box<dyn SerialPort>;

const HANDSHAKE: [u8; 3] = [0x0a, 0x50, 0x05];
const DA_ADDR: u32 = 0x81e00000;

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

#[derive(Parser)]
#[command(version)]
struct Cli {
    /// Binaries to upload
    #[arg(short, long, value_delimiter = ' ', num_args = 1..)]
    input: Vec<PathBuf>,

    /// Addresses for binaries
    #[arg(short, long, value_delimiter = ' ', num_args = 1.., value_parser=maybe_hex::<u32>)]
    upload_address: Vec<u32>,

    /// Final jump address, jumps to 0x81e00000 if not set
    #[arg(short, long, value_parser=maybe_hex::<u32>)]
    jump_address: Option<u32>,
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

fn open_port() -> Result<Port> {
    print!("Waiting for the preloader interface");
    stdout().flush()?;
    let port = loop {
        let ports = get_ports()?;

        if ports.len() > 1 {
            return Err("Please disconnect other devices in the preloader mode".into());
        } else if ports.is_empty() {
            print!(".");
            stdout().flush()?;
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

    let no_patcher = cli.upload_address.len() == 1 && cli.upload_address[0] == DA_ADDR;
    if no_patcher {
        println!(
            "Preloader won't be patched, some commands may be not available due to security checks"
        );
    }
    let payload = fs::read(if no_patcher {
        &cli.input[0]
    } else {
        Path::new("payload/preloader_patcher.bin")
    })?;

    if no_patcher {
        println!("Uploading payload to {DA_ADDR:#x}...");
    } else {
        println!("Uploading preloader patcher...");
    }
    if let Err(e) = send_da(&mut port, DA_ADDR, &payload) {
        eprintln!("Failed uploading payload ({e}), retrying...");
        drop(port);
        return run(cli);
    }
    println!("Jumping to {DA_ADDR:#x}...");
    jump_da(&mut port, DA_ADDR)?;

    if !no_patcher {
        println!("Trying to sync with patched preloader...");
        handshake(&mut port)?;

        for (i, a) in cli.input.into_iter().zip(cli.upload_address) {
            let payload = fs::read(i)?;
            println!("Uploading payload to {a:#x}...");
            send_da(&mut port, a, &payload)?;
        }

        let jump = cli.jump_address.unwrap_or(DA_ADDR);
        println!("Jumping to {jump:#x}...");
        jump_da(&mut port, jump)?;
    }
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    assert!(!cli.input.is_empty());
    assert_eq!(cli.input.len(), cli.upload_address.len());

    run(cli)?;

    Ok(())
}
