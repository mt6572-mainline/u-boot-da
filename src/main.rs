use std::{
    env::args,
    error::Error,
    fs,
    io::{Write, stdout},
    thread::sleep,
    time::Duration,
};

use serialport::{SerialPort, SerialPortInfo, SerialPortType, available_ports};

type Result<T> = core::result::Result<T, Box<dyn Error>>;
type Port = Box<dyn SerialPort>;

const HANDSHAKE: [u8; 3] = [0x0a, 0x50, 0x05];
const DA_ADDR: u32 = 0x81e00000;

trait DA {
    fn write_and_check(&mut self, byte: u8, expected: u8) -> Result<bool>;
    fn echo_u8(&mut self, byte: u8) -> Result<()>;
    fn echo_u32(&mut self, data: u32) -> Result<()>;
    fn echo_addr(&mut self, addr: u32) -> Result<()>;
}

impl DA for Port {
    fn write_and_check(&mut self, byte: u8, expected: u8) -> Result<bool> {
        self.write_all(&[byte])?;
        let mut buf = [0; 1];
        self.read_exact(&mut buf)?;
        Ok(byte == expected)
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

    fn echo_addr(&mut self, addr: u32) -> Result<()> {
        self.write_all(&addr.to_le_bytes())?;
        let mut buf = [0; 4];
        self.read_exact(&mut buf)?;

        let result = u32::from_le_bytes(buf);
        if addr == result {
            Ok(())
        } else {
            Err(format!("Data doesn't match! Expected: {addr:#x}, got: {result:#x}").into())
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
    port.read_exact(&mut buf)?;
    let status = u16::from_be_bytes(buf);
    if status != 0 {
        return Err(format!("Got non-zero status while sending DA info: {status}").into());
    }

    port.write_all(payload)?;

    port.read_exact(&mut buf)?;
    port.read_exact(&mut buf)?;
    let status = u16::from_be_bytes(buf);
    if status != 0 {
        Err(format!("Failed to send U-Boot: {status}").into())
    } else {
        Ok(())
    }
}

fn jump_da(port: &mut Port, addr: u32) -> Result<()> {
    port.echo_u8(0xd5)?;
    port.echo_addr(addr)?;

    let mut buf = [0; 2];
    port.read_exact(&mut buf)?;
    let status = u16::from_be_bytes(buf);
    if status != 0 {
        Err(format!("Failed jumping to {addr:#x}").into())
    } else {
        Ok(())
    }
}

fn run(uboot: &[u8]) -> Result<()> {
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
    let mut port = serialport::new(port.port_name, 921600)
        .timeout(Duration::from_millis(500))
        .open()?;

    /* Read "READY", just to be safe let's expect it may appear up to 4 times */
    let mut buf = [0; 20];
    port.read(&mut buf)?;

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

    println!("Handshake completed");
    println!("Uploading U-Boot to {DA_ADDR:#x}...");
    if let Err(e) = send_da(&mut port, DA_ADDR, &uboot) {
        println!("Failed uploading U-Boot ({e}), retrying...");
        drop(port);
        return run(uboot);
    }
    println!("Jumping to {DA_ADDR:#x}...");
    jump_da(&mut port, DA_ADDR)?;
    println!("All done");

    Ok(())
}

fn main() -> Result<()> {
    let path = args().nth(1).ok_or("Provide a path to U-Boot binary")?;
    let uboot = fs::read(path)?;
    run(&uboot)
}
