use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

const PACKET_SIZE: usize = 64;
const MODE_PROPERTY: u8 = 0x03;
const MODE_HARDWARE: u8 = 0x01;
const MODE_SOFTWARE: u8 = 0x02;
const IRONCLAW_SE_ID: &str = "HID_ID=0003:00001B1C:00002B32";
const COMMAND_DESCRIPTOR: &[u8] = &[0x06, 0x42, 0xff, 0x09, 0x01];
const INPUT_DESCRIPTOR: &[u8] = &[0x06, 0x42, 0xff, 0x09, 0x02];

#[derive(Debug)]
pub struct BragiControl {
    command_path: PathBuf,
    input_path: PathBuf,
    software_mode: bool,
}

impl BragiControl {
    pub fn activate() -> io::Result<Self> {
        let (command_path, input_path) =
            discover_at(Path::new("/sys/class/hidraw"))?.ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "Wireless SE Bragi interfaces not found",
                )
            })?;
        let mut control = Self {
            command_path,
            input_path,
            software_mode: false,
        };
        let current = control.get_mode()?;
        if current != MODE_HARDWARE && current != MODE_SOFTWARE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unexpected Bragi mode {current}"),
            ));
        }
        if current != MODE_SOFTWARE {
            control.set_mode(MODE_SOFTWARE)?;
        }
        control.software_mode = true;
        Ok(control)
    }

    pub fn input_path(&self) -> &Path {
        &self.input_path
    }

    fn open_command(&self) -> io::Result<File> {
        OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&self.command_path)
    }

    fn transact(&self, packet: [u8; PACKET_SIZE]) -> io::Result<[u8; PACKET_SIZE]> {
        let mut device = self.open_command()?;
        let mut report = [0_u8; PACKET_SIZE + 1];
        report[1..].copy_from_slice(&packet);
        device.write_all(&report)?;

        let deadline = Instant::now() + Duration::from_secs(2);
        let mut response = [0_u8; PACKET_SIZE];
        loop {
            match device.read(&mut response) {
                Ok(PACKET_SIZE) => break,
                Ok(size) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        format!("short Bragi response: {size} bytes"),
                    ));
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "timed out waiting for Bragi response",
                        ));
                    }
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => return Err(error),
            }
        }
        if response[1] != packet[1] || response[2] != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Bragi command rejected with error 0x{:02x}", response[2]),
            ));
        }
        Ok(response)
    }

    fn get_mode(&self) -> io::Result<u8> {
        let mut packet = [0_u8; PACKET_SIZE];
        packet[..4].copy_from_slice(&[0x08, 0x02, MODE_PROPERTY, 0x00]);
        Ok(self.transact(packet)?[3])
    }

    fn set_mode(&self, mode: u8) -> io::Result<()> {
        let mut packet = [0_u8; PACKET_SIZE];
        packet[..6].copy_from_slice(&[0x08, 0x01, MODE_PROPERTY, 0x00, mode, 0x00]);
        self.transact(packet).map(|_| ())
    }
}

impl Drop for BragiControl {
    fn drop(&mut self) {
        if self.software_mode {
            let _ = self.set_mode(MODE_HARDWARE);
            self.software_mode = false;
        }
    }
}

fn discover_at(root: &Path) -> io::Result<Option<(PathBuf, PathBuf)>> {
    let mut command = None;
    let mut input = None;
    for entry in fs::read_dir(root)?.flatten() {
        let device = entry.path().join("device");
        let uevent = fs::read_to_string(device.join("uevent")).unwrap_or_default();
        if !uevent.lines().any(|line| line == IRONCLAW_SE_ID) {
            continue;
        }
        let descriptor = fs::read(device.join("report_descriptor")).unwrap_or_default();
        let node = PathBuf::from("/dev").join(entry.file_name());
        if descriptor.starts_with(COMMAND_DESCRIPTOR) {
            command = Some(node);
        } else if descriptor.starts_with(INPUT_DESCRIPTOR) {
            input = Some(node);
        }
    }
    Ok(command.zip(input))
}

pub fn button_mask(report: &[u8]) -> Option<u16> {
    if report.len() == PACKET_SIZE && report[1] == 0x02 {
        Some(u16::from_le_bytes([report[2], report[3]]))
    } else {
        None
    }
}

pub fn source_for_bit(bit: u8) -> String {
    format!("BRAGI_BUTTON:{}", bit + 1)
}

pub fn linux_code_for_bit(bit: u8) -> u16 {
    const LUT: [u16; 16] = [
        272, 273, 274, 276, 275, 277, 278, 280, 281, 279, 282, 283, 284, 285, 286, 287,
    ];
    LUT[usize::from(bit.min(15))]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_extended_button_mask() {
        let mut report = [0_u8; PACKET_SIZE];
        report[1] = 0x02;
        report[2] = 0x80;
        report[3] = 0x02;
        assert_eq!(button_mask(&report), Some(0x0280));
        assert_eq!(source_for_bit(7), "BRAGI_BUTTON:8");
        assert_eq!(linux_code_for_bit(3), 276);
        assert_eq!(linux_code_for_bit(9), 279);
    }
}
